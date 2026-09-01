use crate::errors::{LifecycleError, LifecycleResult};
use crate::lifecycle::dispatch::{emit_lifecycle_event, CallbackDispatcher};
use crate::lifecycle::event::{LifecycleEventType, PlatformEvent};
use crate::lifecycle::platform::{PlatformGuard, PlatformLifecycle};
use parking_lot::Mutex;
use crate::lifecycle::sync_wait::SyncWait;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

pub struct MockGuard {
    released: Mutex<bool>,
}

impl MockGuard {
    fn new() -> Self {
        Self {
            released: Mutex::new(false),
        }
    }

    pub fn was_released(&self) -> bool {
        *self.released.lock()
    }
}

impl PlatformGuard for MockGuard {
    fn release(&mut self) -> LifecycleResult<()> {
        *self.released.lock() = true;
        Ok(())
    }
}

#[derive(Clone)]
pub struct MockInjectHandle {
    inject_tx: Arc<Mutex<Option<mpsc::Sender<LifecycleEventType>>>>,
}

impl MockInjectHandle {
    pub fn inject_event(&self, event_type: LifecycleEventType) -> LifecycleResult<()> {
        let tx = self
            .inject_tx
            .lock()
            .clone()
            .ok_or(LifecycleError::NotRunning)?;
        tx.send(event_type)
            .map_err(|err| LifecycleError::Platform(err.to_string()))
    }
}

pub struct MockLifecycle {
    inject_tx: Arc<Mutex<Option<mpsc::Sender<LifecycleEventType>>>>,
    listener: Mutex<Option<JoinHandle<()>>>,
}

impl MockLifecycle {
    pub fn new() -> Self {
        Self {
            inject_tx: Arc::new(Mutex::new(None)),
            listener: Mutex::new(None),
        }
    }

    pub fn inject_handle(&self) -> MockInjectHandle {
        MockInjectHandle {
            inject_tx: Arc::clone(&self.inject_tx),
        }
    }

    pub fn inject_event(&self, event_type: LifecycleEventType) -> LifecycleResult<()> {
        self.inject_handle().inject_event(event_type)
    }
}

impl Default for MockLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

impl PlatformLifecycle for MockLifecycle {
    fn start(
        &mut self,
        events: SyncSender<PlatformEvent>,
        dispatcher: Arc<CallbackDispatcher>,
        worker_ready: Receiver<()>,
        _sync_wait: Arc<SyncWait>,
        _max_wait: Duration,
    ) -> LifecycleResult<()> {
        let (inject_tx, inject_rx) = mpsc::channel();
        *self.inject_tx.lock() = Some(inject_tx);

        let handle = std::thread::Builder::new()
            .name("mock-lifecycle".into())
            .spawn(move || {
                while let Ok(event_type) = inject_rx.recv() {
                    emit_lifecycle_event(
                        &events,
                        &worker_ready,
                        &dispatcher,
                        event_type,
                        "mock_inject",
                    );
                }
            })
            .map_err(|err| LifecycleError::Platform(err.to_string()))?;

        *self.listener.lock() = Some(handle);
        Ok(())
    }

    fn stop(&mut self) -> LifecycleResult<()> {
        *self.inject_tx.lock() = None;
        if let Some(handle) = self.listener.lock().take() {
            let _ = handle.join();
        }
        Ok(())
    }

    fn acquire_guard(&self, _event: LifecycleEventType) -> LifecycleResult<Box<dyn PlatformGuard>> {
        Ok(Box::new(MockGuard::new()))
    }
}
