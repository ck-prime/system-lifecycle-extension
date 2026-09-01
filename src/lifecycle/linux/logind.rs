use crate::lifecycle::event::LifecycleEventType;
use zbus::proxy;

#[proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
pub trait Manager {
    fn inhibit(
        &self,
        mode: &str,
        who: &str,
        why: &str,
        what: &str,
    ) -> zbus::Result<zbus::zvariant::OwnedFd>;

    #[zbus(property)]
    fn scheduled_shutdown(&self) -> zbus::Result<(String, u64, u32)>;

    #[zbus(property, name = "SleepOperation")]
    fn sleep_operation(&self) -> zbus::Result<String>;

    #[signal]
    fn prepare_for_shutdown(&self, start: bool) -> zbus::Result<()>;

    #[signal]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;
}

pub async fn classify_shutdown(connection: &zbus::Connection) -> LifecycleEventType {
    let proxy = match ManagerProxy::new(connection).await {
        Ok(proxy) => proxy,
        Err(_) => return LifecycleEventType::Shutdown,
    };

    match proxy.scheduled_shutdown().await {
        Ok((kind, _, _)) if kind == "reboot" => LifecycleEventType::Reboot,
        _ => LifecycleEventType::Shutdown,
    }
}

pub async fn classify_sleep(connection: &zbus::Connection) -> LifecycleEventType {
    let proxy = match ManagerProxy::new(connection).await {
        Ok(proxy) => proxy,
        Err(_) => return LifecycleEventType::Sleep,
    };

    match proxy.sleep_operation().await {
        Ok(op) if op.to_ascii_lowercase().contains("hibernate") => LifecycleEventType::Hibernate,
        _ => LifecycleEventType::Sleep,
    }
}
