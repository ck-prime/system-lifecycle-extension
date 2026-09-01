use crate::errors::{LifecycleError, LifecycleResult};
use crate::lifecycle::dispatch::CallbackDispatcher;
use crate::lifecycle::event::{LifecycleEvent, PlatformEvent};
use crate::lifecycle::platform::{PlatformGuard, PlatformLifecycle};
use crate::lifecycle::sync_wait::SyncWait;
use crate::native_log::{native_log, set_log_path};
use parking_lot::Mutex;
use pyo3::prelude::*;
use std::sync::mpsc::{self, SyncSender};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tracing::{error, info, warn};

use super::state::LifecycleState;

struct ActiveSession {
    event: LifecycleEvent,
    guard: Box<dyn PlatformGuard>,
}

struct WorkerContext {
    state: Arc<Mutex<LifecycleState>>,
    sync_wait: Arc<SyncWait>,
    ready_tx: SyncSender<()>,
    platform: Arc<Mutex<Box<dyn PlatformLifecycle>>>,
    max_wait: Duration,
    stop_signal: Arc<StdMutex<bool>>,
}

impl WorkerContext {
    fn handle_platform_event(&self, platform_event: PlatformEvent) {
        native_log(&format!(
            "bridge: platform event received event={}",
            platform_event.event.event_type.as_str()
        ));

        {
            let current = self.state.lock();
            if matches!(*current, LifecycleState::Stopped) {
                native_log("bridge: event ignored, controller stopped");
                return;
            }
            if current.is_active_wait() {
                native_log("bridge: duplicate event ignored while waiting for sync");
                warn!(
                    event = platform_event.event.event_type.as_str(),
                    "duplicate lifecycle event ignored while waiting for sync"
                );
                return;
            }
        }

        let event = platform_event.event;
        info!(
            platform = std::env::consts::OS,
            event = event.event_type.as_str(),
            "lifecycle event detected"
        );

        *self.state.lock() = LifecycleState::EventDetected {
            event_type: event.event_type,
            timestamp: event.timestamp,
        };

        let mut guard = {
            let platform = self.platform.lock();
            match platform.acquire_guard(event.event_type) {
                Ok(guard) => guard,
                Err(err) => {
                    error!(error = %err, "failed to acquire lifecycle guard");
                    *self.state.lock() = LifecycleState::Idle;
                    return;
                }
            }
        };

        info!(
            event = event.event_type.as_str(),
            "lifecycle guard acquired"
        );
        self.sync_wait.reset();

        *self.state.lock() = LifecycleState::WaitingForSync {
            event_type: event.event_type,
            timestamp: event.timestamp,
        };

        if let Err(err) = self.ready_tx.send(()) {
            native_log(&format!("bridge: failed to signal worker ready: {err}"));
            *self.state.lock() = LifecycleState::Idle;
            let _ = guard.release();
            return;
        }

        native_log("bridge: waiting for synccomplete");
        info!("waiting for synccomplete");

        let completed = self.sync_wait.wait(self.max_wait);
        native_log(&format!("bridge: sync wait finished completed={completed}"));
        let mut session = ActiveSession { event, guard };

        if completed {
            info!(
                event = session.event.event_type.as_str(),
                "synccomplete received"
            );
            *self.state.lock() = LifecycleState::SyncCompleted {
                event_type: session.event.event_type,
            };
        } else {
            warn!(
                event = session.event.event_type.as_str(),
                timeout_seconds = self.max_wait.as_secs(),
                "timeout reached; releasing lifecycle guard"
            );
            *self.state.lock() = LifecycleState::TimedOut {
                event_type: session.event.event_type,
            };
        }

        if let Err(err) = session.guard.release() {
            error!(error = %err, "failed to release lifecycle guard");
        } else {
            info!(
                event = session.event.event_type.as_str(),
                "lifecycle guard released"
            );
        }

        *self.state.lock() = LifecycleState::Released;
        *self.state.lock() = LifecycleState::Idle;
    }
}

pub struct LifecycleController {
    state: Arc<Mutex<LifecycleState>>,
    sync_wait: Arc<SyncWait>,
    max_wait: Duration,
    dispatcher: Arc<CallbackDispatcher>,
    platform: Arc<Mutex<Box<dyn PlatformLifecycle>>>,
    worker: Mutex<Option<JoinHandle<()>>>,
    stop_signal: Arc<StdMutex<bool>>,
    running: std::sync::atomic::AtomicBool,
}

impl LifecycleController {
    pub fn new(max_wait: Duration, platform: Box<dyn PlatformLifecycle>) -> Self {
        Self {
            state: Arc::new(Mutex::new(LifecycleState::Idle)),
            sync_wait: Arc::new(SyncWait::new()),
            max_wait,
            dispatcher: Arc::new(CallbackDispatcher::new()),
            platform: Arc::new(Mutex::new(platform)),
            worker: Mutex::new(None),
            stop_signal: Arc::new(StdMutex::new(false)),
            running: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn dispatcher(&self) -> Arc<CallbackDispatcher> {
        Arc::clone(&self.dispatcher)
    }

    pub fn platform(&self) -> Arc<Mutex<Box<dyn PlatformLifecycle>>> {
        Arc::clone(&self.platform)
    }

    pub fn is_running(&self) -> bool {
        self.running.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn start(&self, callback: pyo3::Py<pyo3::PyAny>) -> LifecycleResult<()> {
        if self.is_running() {
            return Err(LifecycleError::AlreadyRunning);
        }

        if let Ok(cwd) = std::env::current_dir() {
            set_log_path(cwd.join("lifecycle_native.log"));
        }

        Python::with_gil(|_py| {
            self.dispatcher
                .start(callback)
                .map_err(|err| LifecycleError::Platform(err.to_string()))
        })?;
        self.sync_wait.reset();
        *self.state.lock() = LifecycleState::Idle;
        if let Ok(mut stop) = self.stop_signal.lock() {
            *stop = false;
        }

        // Rendezvous channel: platform send() blocks until the worker receives.
        let (event_tx, event_rx) = mpsc::sync_channel::<PlatformEvent>(0);
        let (ready_tx, ready_rx) = mpsc::sync_channel::<()>(0);

        let ctx = WorkerContext {
            state: Arc::clone(&self.state),
            sync_wait: Arc::clone(&self.sync_wait),
            ready_tx,
            platform: Arc::clone(&self.platform),
            max_wait: self.max_wait,
            stop_signal: Arc::clone(&self.stop_signal),
        };

        self.running
            .store(true, std::sync::atomic::Ordering::SeqCst);

        let handle = thread::Builder::new()
            .name("lifecycle-controller".into())
            .spawn(move || {
                native_log("bridge: controller worker started");
                loop {
                    if event_rx
                        .recv_timeout(Duration::from_millis(250))
                        .map(|event| {
                            ctx.handle_platform_event(event);
                            true
                        })
                        .unwrap_or(false)
                    {
                        continue;
                    }

                    let should_stop = ctx.stop_signal.lock().map(|flag| *flag).unwrap_or(true);
                    if should_stop {
                        break;
                    }
                }
                native_log("bridge: controller worker exiting");
                info!("lifecycle controller worker exiting");
            })
            .map_err(|err| LifecycleError::Platform(err.to_string()))?;

        *self.worker.lock() = Some(handle);
        self.platform.lock().start(
            event_tx,
            Arc::clone(&self.dispatcher),
            ready_rx,
            Arc::clone(&self.sync_wait),
            self.max_wait,
        )?;
        info!(
            platform = std::env::consts::OS,
            "lifecycle listener started"
        );
        Ok(())
    }

    pub fn sync_complete(&self) -> bool {
        if !self.state.lock().is_active_wait() {
            return false;
        }
        self.sync_wait.signal()
    }

    pub fn stop(&self) -> LifecycleResult<()> {
        if !self.is_running() {
            return Err(LifecycleError::NotRunning);
        }

        *self.state.lock() = LifecycleState::Stopped;
        if let Ok(mut stop) = self.stop_signal.lock() {
            *stop = true;
        }
        self.sync_wait.signal();

        if let Err(err) = self.platform.lock().stop() {
            warn!(error = %err, "error stopping platform lifecycle listener");
        }

        if let Some(handle) = self.worker.lock().take() {
            let _ = handle.join();
        }

        self.dispatcher.clear_callback();
        self.running
            .store(false, std::sync::atomic::Ordering::SeqCst);
        *self.state.lock() = LifecycleState::Idle;
        info!("lifecycle listener stopped");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifecycle::mock::MockLifecycle;

    #[test]
    fn sync_complete_is_idempotent_without_active_wait() {
        let controller =
            LifecycleController::new(Duration::from_secs(1), Box::new(MockLifecycle::new()));
        assert!(!controller.sync_complete());
        assert!(!controller.sync_complete());
    }
}
