use crate::lifecycle::event::LifecycleEventType;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleState {
    Idle,
    EventDetected {
        event_type: LifecycleEventType,
        timestamp: u64,
    },
    WaitingForSync {
        event_type: LifecycleEventType,
        timestamp: u64,
    },
    SyncCompleted {
        event_type: LifecycleEventType,
    },
    TimedOut {
        event_type: LifecycleEventType,
    },
    Released,
    Stopped,
}

impl LifecycleState {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::EventDetected { .. } => "event_detected",
            Self::WaitingForSync { .. } => "waiting_for_sync",
            Self::SyncCompleted { .. } => "sync_completed",
            Self::TimedOut { .. } => "timed_out",
            Self::Released => "released",
            Self::Stopped => "stopped",
        }
    }

    pub fn is_active_wait(&self) -> bool {
        matches!(self, Self::WaitingForSync { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_state_names_are_stable() {
        assert_eq!(LifecycleState::Idle.name(), "idle");
        assert_eq!(
            LifecycleState::WaitingForSync {
                event_type: crate::lifecycle::event::LifecycleEventType::Shutdown,
                timestamp: 1,
            }
            .name(),
            "waiting_for_sync"
        );
    }
}
