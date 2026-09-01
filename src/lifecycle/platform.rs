use crate::errors::LifecycleResult;
use crate::lifecycle::dispatch::CallbackDispatcher;
use crate::lifecycle::event::{LifecycleEventType, PlatformEvent};
use crate::lifecycle::sync_wait::SyncWait;
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::Arc;
use std::time::Duration;

/// RAII handle that releases an OS lifecycle guard when dropped.
pub trait PlatformGuard: Send {
    fn release(&mut self) -> LifecycleResult<()>;
}

/// Platform-specific lifecycle detection and guard acquisition.
pub trait PlatformLifecycle: Send {
    fn start(
        &mut self,
        events: SyncSender<PlatformEvent>,
        dispatcher: Arc<CallbackDispatcher>,
        worker_ready: Receiver<()>,
        sync_wait: Arc<SyncWait>,
        max_wait: Duration,
    ) -> LifecycleResult<()>;
    fn stop(&mut self) -> LifecycleResult<()>;
    fn acquire_guard(&self, event: LifecycleEventType) -> LifecycleResult<Box<dyn PlatformGuard>>;
}
pub fn create_platform() -> LifecycleResult<Box<dyn PlatformLifecycle>> {
    #[cfg(target_os = "linux")]
    {
        return Ok(Box::new(super::linux::LinuxLifecycle::new()?));
    }
    #[cfg(target_os = "macos")]
    {
        return Ok(Box::new(super::macos::MacOsLifecycle::new()?));
    }
    #[cfg(target_os = "windows")]
    {
        Ok(Box::new(super::windows::WindowsLifecycle::new()?))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        Ok(Box::new(super::mock::MockLifecycle::new()))
    }
}

#[cfg(any(test, feature = "mock-platform"))]
pub fn create_mock_platform() -> LifecycleResult<Box<dyn PlatformLifecycle>> {
    Ok(Box::new(super::mock::MockLifecycle::new()))
}
