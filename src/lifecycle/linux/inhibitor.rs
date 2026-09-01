use crate::errors::{LifecycleError, LifecycleResult};
use crate::lifecycle::event::{LifecycleEventType, PlatformEvent};
use std::os::unix::io::{AsRawFd, RawFd};

pub struct InhibitorGuard {
    fd: Option<zbus::zvariant::OwnedFd>,
    released: bool,
}

impl InhibitorGuard {
    pub fn new(fd: zbus::zvariant::OwnedFd) -> Self {
        Self {
            fd: Some(fd),
            released: false,
        }
    }

    pub fn raw_fd(&self) -> Option<RawFd> {
        self.fd.as_ref().map(|fd| fd.as_raw_fd())
    }
}

impl crate::lifecycle::platform::PlatformGuard for InhibitorGuard {
    fn release(&mut self) -> LifecycleResult<()> {
        if self.released {
            return Ok(());
        }
        self.fd.take();
        self.released = true;
        Ok(())
    }
}

pub fn what_for_event(event: LifecycleEventType) -> &'static str {
    match event {
        LifecycleEventType::Shutdown => "shutdown",
        LifecycleEventType::Reboot => "shutdown:handle-reboot-key",
        LifecycleEventType::Sleep | LifecycleEventType::Hibernate => "sleep",
    }
}

pub async fn acquire_inhibitor(
    connection: &zbus::Connection,
    event: LifecycleEventType,
) -> LifecycleResult<InhibitorGuard> {
    use crate::lifecycle::linux::logind::ManagerProxy;

    let proxy = ManagerProxy::new(connection)
        .await
        .map_err(|err| LifecycleError::Platform(err.to_string()))?;

    let fd = proxy
        .inhibit(
            "delay",
            "system-lifecycle-extension",
            "Application synchronization in progress",
            what_for_event(event),
        )
        .await
        .map_err(|err| LifecycleError::Platform(err.to_string()))?;

    Ok(InhibitorGuard::new(fd))
}
