use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LifecycleEventType {
    Shutdown,
    Reboot,
    Sleep,
    Hibernate,
}

impl LifecycleEventType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shutdown => "shutdown",
            Self::Reboot => "reboot",
            Self::Sleep => "sleep",
            Self::Hibernate => "hibernate",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "shutdown" => Some(Self::Shutdown),
            "reboot" => Some(Self::Reboot),
            "sleep" => Some(Self::Sleep),
            "hibernate" => Some(Self::Hibernate),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleEvent {
    pub event_type: LifecycleEventType,
    pub timestamp: u64,
}

impl LifecycleEvent {
    pub fn new(event_type: LifecycleEventType) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self {
            event_type,
            timestamp,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlatformEvent {
    pub event: LifecycleEvent,
}

impl PlatformEvent {
    pub fn new(event_type: LifecycleEventType) -> Self {
        Self {
            event: LifecycleEvent::new(event_type),
        }
    }
}
