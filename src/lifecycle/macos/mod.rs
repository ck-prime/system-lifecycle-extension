mod power;

use crate::errors::{LifecycleError, LifecycleResult};
use crate::lifecycle::dispatch::{emit_lifecycle_event, CallbackDispatcher};
use crate::lifecycle::event::{LifecycleEventType, PlatformEvent};
use crate::lifecycle::platform::{PlatformGuard, PlatformLifecycle};
use block2::RcBlock;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSWorkspace, NSWorkspaceWillPowerOffNotification, NSWorkspaceWillSleepNotification,
};
use objc2_foundation::{NSNotification, NSNotificationCenter};
use parking_lot::Mutex;
use power::MacOsGuard;
use crate::lifecycle::sync_wait::SyncWait;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;
use tracing::{error, info};

pub struct MacOsLifecycle {
    listener: Mutex<Option<JoinHandle<()>>>,
    stop_flag: Arc<Mutex<bool>>,
}

impl MacOsLifecycle {
    pub fn new() -> LifecycleResult<Self> {
        Ok(Self {
            listener: Mutex::new(None),
            stop_flag: Arc::new(Mutex::new(false)),
        })
    }
}

impl PlatformLifecycle for MacOsLifecycle {
    fn start(
        &mut self,
        events: SyncSender<PlatformEvent>,
        dispatcher: Arc<CallbackDispatcher>,
        worker_ready: Receiver<()>,
        _sync_wait: Arc<SyncWait>,
        _max_wait: Duration,
    ) -> LifecycleResult<()> {
        *self.stop_flag.lock() = false;
        let stop_flag = Arc::clone(&self.stop_flag);

        let handle = std::thread::Builder::new()
            .name("macos-lifecycle-listener".into())
            .spawn(move || {
                if let Err(err) =
                    run_workspace_observer(events, dispatcher, worker_ready, stop_flag)
                {
                    error!(error = %err, "macos lifecycle listener failed");
                }
            })
            .map_err(|err| LifecycleError::Platform(err.to_string()))?;

        *self.listener.lock() = Some(handle);
        Ok(())
    }

    fn stop(&mut self) -> LifecycleResult<()> {
        *self.stop_flag.lock() = true;
        if let Some(handle) = self.listener.lock().take() {
            let _ = handle.join();
        }
        Ok(())
    }

    fn acquire_guard(&self, event: LifecycleEventType) -> LifecycleResult<Box<dyn PlatformGuard>> {
        Ok(Box::new(MacOsGuard::new(event)?))
    }
}

fn run_workspace_observer(
    events: SyncSender<PlatformEvent>,
    dispatcher: Arc<CallbackDispatcher>,
    worker_ready: Receiver<()>,
    stop_flag: Arc<Mutex<bool>>,
) -> LifecycleResult<()> {
    let (internal_tx, internal_rx) = mpsc::channel::<LifecycleEventType>();
    std::thread::Builder::new()
        .name("macos-lifecycle-forward".into())
        .spawn(move || {
            while let Ok(event_type) = internal_rx.recv() {
                emit_lifecycle_event(
                    &events,
                    &worker_ready,
                    &dispatcher,
                    event_type,
                    "NSWorkspaceNotification",
                );
            }
        })
        .map_err(|err| LifecycleError::Platform(err.to_string()))?;

    let workspace = NSWorkspace::sharedWorkspace();
    let center = workspace.notificationCenter();

    let sleep_tx = internal_tx.clone();
    let sleep_block = RcBlock::new(move |_notification: Option<Retained<NSNotification>>| {
        let _ = sleep_tx.send(LifecycleEventType::Sleep);
    });

    let shutdown_tx = internal_tx;
    let shutdown_block = RcBlock::new(move |_notification: Option<Retained<NSNotification>>| {
        let _ = shutdown_tx.send(LifecycleEventType::Shutdown);
    });

    let sleep_observer = center.addObserverForName_object_queue_usingBlock(
        Some(NSWorkspaceWillSleepNotification),
        None,
        None,
        &sleep_block,
    );

    let power_observer = center.addObserverForName_object_queue_usingBlock(
        Some(NSWorkspaceWillPowerOffNotification),
        None,
        None,
        &shutdown_block,
    );

    info!("macos workspace lifecycle notifications registered");

    while !*stop_flag.lock() {
        std::thread::sleep(std::time::Duration::from_millis(250));
    }

    center.removeObserver(&sleep_observer);
    center.removeObserver(&power_observer);
    Ok(())
}
