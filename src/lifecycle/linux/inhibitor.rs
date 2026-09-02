use crate::errors::{LifecycleError, LifecycleResult};
use crate::lifecycle::event::LifecycleEventType;

pub struct InhibitorGuard {
    fd: Option<zbus::zvariant::OwnedFd>,
    released: bool,
    on_release: Option<Box<dyn FnOnce() + Send>>,
}

/// Used when no logind inhibitor is available.
/// Python still receives the event, but the OS is not held.
pub struct NoOpGuard {
    released: bool,
}

impl InhibitorGuard {
    pub fn new(fd: zbus::zvariant::OwnedFd) -> Self {
        Self {
            fd: Some(fd),
            released: false,
            on_release: None,
        }
    }

    pub fn on_release(mut self, callback: impl FnOnce() + Send + 'static) -> Self {
        self.on_release = Some(Box::new(callback));
        self
    }
}

impl NoOpGuard {
    pub fn new() -> Self {
        Self { released: false }
    }
}

impl crate::lifecycle::platform::PlatformGuard for NoOpGuard {
    fn release(&mut self) -> LifecycleResult<()> {
        self.released = true;
        Ok(())
    }
}

impl crate::lifecycle::platform::PlatformGuard for InhibitorGuard {
    fn release(&mut self) -> LifecycleResult<()> {
        if self.released {
            return Ok(());
        }
        self.fd.take();
        self.released = true;
        if let Some(callback) = self.on_release.take() {
            callback();
        }
        Ok(())
    }
}

pub fn what_for_event(event: LifecycleEventType) -> &'static str {
    match event {
        LifecycleEventType::Shutdown | LifecycleEventType::Reboot => "shutdown",
        LifecycleEventType::Sleep | LifecycleEventType::Hibernate => "sleep",
    }
}

pub async fn acquire_inhibitor_with_proxy(
    proxy: &crate::lifecycle::linux::logind::ManagerProxy<'_>,
    event: LifecycleEventType,
) -> LifecycleResult<InhibitorGuard> {
    let fd = proxy
        .inhibit(
            what_for_event(event),
            "system-lifecycle-extension",
            "Application synchronization in progress",
            "delay",
        )
        .await
        .map_err(|err| LifecycleError::Platform(err.to_string()))?;

    Ok(InhibitorGuard::new(fd))
}

pub async fn acquire_inhibitor(
    connection: &zbus::Connection,
    event: LifecycleEventType,
) -> LifecycleResult<InhibitorGuard> {
    use crate::lifecycle::linux::logind::ManagerProxy;

    let proxy = ManagerProxy::new(connection)
        .await
        .map_err(|err| LifecycleError::Platform(err.to_string()))?;

    acquire_inhibitor_with_proxy(&proxy, event).await
}
