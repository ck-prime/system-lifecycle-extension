mod inhibitor;
mod logind;

use crate::errors::{LifecycleError, LifecycleResult};
use crate::lifecycle::dispatch::{emit_lifecycle_event, CallbackDispatcher};
use crate::lifecycle::event::{LifecycleEventType, PlatformEvent};
use crate::lifecycle::platform::{PlatformGuard, PlatformLifecycle};
use crate::lifecycle::sync_wait::SyncWait;
use crate::native_log::native_log;
use futures_util::StreamExt;
use inhibitor::{InhibitorGuard, NoOpGuard};
use logind::ManagerProxy;
use parking_lot::Mutex;
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, OnceLock};
use std::thread::JoinHandle;
use std::time::Duration;
use tracing::{error, info, warn};

static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

fn runtime() -> LifecycleResult<&'static tokio::runtime::Runtime> {
    if RUNTIME.get().is_none() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("lifecycle-logind")
            .build()
            .map_err(|err| LifecycleError::Platform(err.to_string()))?;
        let _ = RUNTIME.set(rt);
    }
    RUNTIME
        .get()
        .ok_or_else(|| LifecycleError::Platform("failed to initialize tokio runtime".into()))
}

pub struct LinuxLifecycle {
    listener: Mutex<Option<JoinHandle<()>>>,
    stop_tx: Mutex<Option<tokio::sync::watch::Sender<bool>>>,
    pending_guard: Arc<Mutex<Option<InhibitorGuard>>>,
    standby_shutdown: Arc<Mutex<Option<InhibitorGuard>>>,
    standby_sleep: Arc<Mutex<Option<InhibitorGuard>>>,
}

impl LinuxLifecycle {
    pub fn new() -> LifecycleResult<Self> {
        Ok(Self {
            listener: Mutex::new(None),
            stop_tx: Mutex::new(None),
            pending_guard: Arc::new(Mutex::new(None)),
            standby_shutdown: Arc::new(Mutex::new(None)),
            standby_sleep: Arc::new(Mutex::new(None)),
        })
    }
}

impl PlatformLifecycle for LinuxLifecycle {
    fn start(
        &mut self,
        events: SyncSender<PlatformEvent>,
        dispatcher: Arc<CallbackDispatcher>,
        worker_ready: Receiver<()>,
        _sync_wait: Arc<SyncWait>,
        _max_wait: Duration,
    ) -> LifecycleResult<()> {
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        *self.stop_tx.lock() = Some(stop_tx);

        let pending_guard = Arc::clone(&self.pending_guard);
        let standby_shutdown = Arc::clone(&self.standby_shutdown);
        let standby_sleep = Arc::clone(&self.standby_sleep);
        let handle = std::thread::Builder::new()
            .name("linux-lifecycle-listener".into())
            .spawn(move || {
                if let Err(err) = runtime().and_then(|rt| {
                    rt.block_on(listen_logind(
                        events,
                        dispatcher,
                        worker_ready,
                        stop_rx,
                        pending_guard,
                        standby_shutdown,
                        standby_sleep,
                    ))
                }) {
                    error!(error = %err, "linux logind listener failed");
                }
            })
            .map_err(|err| LifecycleError::Platform(err.to_string()))?;

        *self.listener.lock() = Some(handle);
        Ok(())
    }

    fn stop(&mut self) -> LifecycleResult<()> {
        if let Some(stop_tx) = self.stop_tx.lock().take() {
            let _ = stop_tx.send(true);
        }
        if let Some(handle) = self.listener.lock().take() {
            let _ = handle.join();
        }
        *self.pending_guard.lock() = None;
        *self.standby_shutdown.lock() = None;
        *self.standby_sleep.lock() = None;
        Ok(())
    }

    fn acquire_guard(&self, event: LifecycleEventType) -> LifecycleResult<Box<dyn PlatformGuard>> {
        if let Some(guard) = self.pending_guard.lock().take() {
            native_log(&format!(
                "native: logind delay inhibitor active for event={}",
                event.as_str()
            ));
            return Ok(Box::new(guard) as Box<dyn PlatformGuard>);
        }

        native_log(&format!(
            "native: no logind inhibitor held for event={}; continuing without OS delay",
            event.as_str()
        ));
        warn!(
            event = event.as_str(),
            "no logind inhibitor held; lifecycle event will not delay the OS"
        );
        Ok(Box::new(NoOpGuard::new()) as Box<dyn PlatformGuard>)
    }
}

async fn install_standby_inhibitor(
    proxy: &ManagerProxy<'_>,
    slot: &Arc<Mutex<Option<InhibitorGuard>>>,
    event: LifecycleEventType,
) {
    match inhibitor::acquire_inhibitor_with_proxy(proxy, event).await {
        Ok(guard) => {
            native_log(&format!(
                "native: standby logind inhibitor installed event={}",
                event.as_str()
            ));
            *slot.lock() = Some(guard);
        }
        Err(err) => {
            native_log(&format!(
                "native: standby inhibitor install failed event={} error={err}",
                event.as_str()
            ));
            warn!(
                error = %err,
                event = event.as_str(),
                "failed to install standby logind inhibitor"
            );
        }
    }
}

async fn replenish_standby_inhibitor(
    connection: Arc<zbus::Connection>,
    slot: Arc<Mutex<Option<InhibitorGuard>>>,
    event: LifecycleEventType,
) {
    if slot.lock().is_some() {
        return;
    }

    let proxy = match ManagerProxy::new(connection.as_ref()).await {
        Ok(proxy) => proxy,
        Err(err) => {
            native_log(&format!("native: standby replenish proxy failed: {err}"));
            return;
        }
    };

    install_standby_inhibitor(&proxy, &slot, event).await;
}

fn arm_pending_from_standby(
    standby: &Arc<Mutex<Option<InhibitorGuard>>>,
    pending: &Arc<Mutex<Option<InhibitorGuard>>>,
    connection: Arc<zbus::Connection>,
    event: LifecycleEventType,
) {
    let Some(guard) = standby.lock().take() else {
        native_log(&format!(
            "native: no standby inhibitor available for event={}",
            event.as_str()
        ));
        warn!(
            event = event.as_str(),
            "no standby logind inhibitor available; OS may not wait for sync"
        );
        return;
    };

    let standby_slot = Arc::clone(standby);
    let guard = guard.on_release(move || {
        if let Ok(rt) = runtime() {
            rt.spawn(replenish_standby_inhibitor(connection, standby_slot, event));
        }
    });

    native_log(&format!(
        "native: armed active logind inhibitor from standby event={}",
        event.as_str()
    ));
    *pending.lock() = Some(guard);
}

async fn listen_logind(
    events: SyncSender<PlatformEvent>,
    dispatcher: Arc<CallbackDispatcher>,
    worker_ready: Receiver<()>,
    mut stop_rx: tokio::sync::watch::Receiver<bool>,
    pending_guard: Arc<Mutex<Option<InhibitorGuard>>>,
    standby_shutdown: Arc<Mutex<Option<InhibitorGuard>>>,
    standby_sleep: Arc<Mutex<Option<InhibitorGuard>>>,
) -> LifecycleResult<()> {
    let connection = match zbus::Connection::system().await {
        Ok(conn) => conn,
        Err(err) => {
            warn!(error = %err, "systemd-logind unavailable; lifecycle detection disabled");
            return Err(LifecycleError::Platform(err.to_string()));
        }
    };

    let connection = Arc::new(connection);

    let proxy = ManagerProxy::new(connection.as_ref())
        .await
        .map_err(|err| LifecycleError::Platform(err.to_string()))?;

    install_standby_inhibitor(&proxy, &standby_shutdown, LifecycleEventType::Shutdown).await;
    install_standby_inhibitor(&proxy, &standby_sleep, LifecycleEventType::Sleep).await;

    let mut shutdown_stream = proxy
        .receive_prepare_for_shutdown()
        .await
        .map_err(|err| LifecycleError::Platform(err.to_string()))?;
    let mut sleep_stream = proxy
        .receive_prepare_for_sleep()
        .await
        .map_err(|err| LifecycleError::Platform(err.to_string()))?;

    info!("connected to org.freedesktop.login1");

    loop {
        if *stop_rx.borrow() {
            break;
        }

        tokio::select! {
            changed = stop_rx.changed() => {
                if changed.is_ok() && *stop_rx.borrow() {
                    break;
                }
            }
            msg = shutdown_stream.next() => {
                match msg {
                    Some(signal) => match signal.args() {
                        Ok(args) if *args.start() => {
                            arm_pending_from_standby(
                                &standby_shutdown,
                                &pending_guard,
                                Arc::clone(&connection),
                                LifecycleEventType::Shutdown,
                            );
                            emit_lifecycle_event(
                                &events,
                                &worker_ready,
                                &dispatcher,
                                LifecycleEventType::Shutdown,
                                "PrepareForShutdown",
                            );
                        }
                        Ok(_) => {}
                        Err(err) => {
                            error!(error = %err, "PrepareForShutdown signal args error");
                        }
                    },
                    None => break,
                }
            }
            msg = sleep_stream.next() => {
                match msg {
                    Some(signal) => match signal.args() {
                        Ok(args) if *args.start() => {
                            arm_pending_from_standby(
                                &standby_sleep,
                                &pending_guard,
                                Arc::clone(&connection),
                                LifecycleEventType::Sleep,
                            );
                            emit_lifecycle_event(
                                &events,
                                &worker_ready,
                                &dispatcher,
                                LifecycleEventType::Sleep,
                                "PrepareForSleep",
                            );
                        }
                        Ok(_) => {}
                        Err(err) => {
                            error!(error = %err, "PrepareForSleep signal args error");
                        }
                    },
                    None => break,
                }
            }
        }
    }

    Ok(())
}
