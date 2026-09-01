mod inhibitor;
mod logind;

use crate::errors::{LifecycleError, LifecycleResult};
use crate::lifecycle::dispatch::{emit_lifecycle_event, CallbackDispatcher};
use crate::lifecycle::event::{LifecycleEventType, PlatformEvent};
use crate::lifecycle::platform::{PlatformGuard, PlatformLifecycle};
use crate::lifecycle::sync_wait::SyncWait;
use futures_util::StreamExt;
use logind::{classify_shutdown, classify_sleep, ManagerProxy};
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
    connection: Arc<Mutex<Option<zbus::Connection>>>,
    stop_tx: Mutex<Option<tokio::sync::watch::Sender<bool>>>,
}

impl LinuxLifecycle {
    pub fn new() -> LifecycleResult<Self> {
        Ok(Self {
            listener: Mutex::new(None),
            connection: Arc::new(Mutex::new(None)),
            stop_tx: Mutex::new(None),
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

        let connection_slot = Arc::clone(&self.connection);
        let handle = std::thread::Builder::new()
            .name("linux-lifecycle-listener".into())
            .spawn(move || {
                if let Err(err) = runtime().and_then(|rt| {
                    rt.block_on(listen_logind(
                        events,
                        dispatcher,
                        worker_ready,
                        stop_rx,
                        connection_slot,
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
        *self.connection.lock() = None;
        Ok(())
    }

    fn acquire_guard(&self, event: LifecycleEventType) -> LifecycleResult<Box<dyn PlatformGuard>> {
        let connection = self
            .connection
            .lock()
            .clone()
            .ok_or_else(|| LifecycleError::Platform("logind connection unavailable".into()))?;
        let rt = runtime()?;
        let guard = rt
            .block_on(inhibitor::acquire_inhibitor(&connection, event))
            .map(|guard| Box::new(guard) as Box<dyn PlatformGuard>)?;
        Ok(guard)
    }
}

async fn listen_logind(
    events: SyncSender<PlatformEvent>,
    dispatcher: Arc<CallbackDispatcher>,
    worker_ready: Receiver<()>,
    mut stop_rx: tokio::sync::watch::Receiver<bool>,
    connection_slot: Arc<Mutex<Option<zbus::Connection>>>,
) -> LifecycleResult<()> {
    let connection = match zbus::Connection::system().await {
        Ok(conn) => conn,
        Err(err) => {
            warn!(error = %err, "systemd-logind unavailable; lifecycle detection disabled");
            return Err(LifecycleError::Platform(err.to_string()));
        }
    };

    *connection_slot.lock() = Some(connection.clone());
    let connection = Arc::new(connection);

    let proxy = ManagerProxy::new(connection.as_ref())
        .await
        .map_err(|err| LifecycleError::Platform(err.to_string()))?;

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
                    Some(Ok(start)) if start => {
                        let event_type = classify_shutdown(connection.as_ref()).await;
                        emit_lifecycle_event(
                            &events,
                            &worker_ready,
                            &dispatcher,
                            event_type,
                            "PrepareForShutdown",
                        );
                    }
                    Some(Ok(_)) => {}
                    Some(Err(err)) => {
                        error!(error = %err, "PrepareForShutdown stream error");
                        break;
                    }
                    None => break,
                }
            }
            msg = sleep_stream.next() => {
                match msg {
                    Some(Ok(start)) if start => {
                        let event_type = classify_sleep(connection.as_ref()).await;
                        emit_lifecycle_event(
                            &events,
                            &worker_ready,
                            &dispatcher,
                            event_type,
                            "PrepareForSleep",
                        );
                    }
                    Some(Ok(_)) => {}
                    Some(Err(err)) => {
                        error!(error = %err, "PrepareForSleep stream error");
                        break;
                    }
                    None => break,
                }
            }
        }
    }

    Ok(())
}
