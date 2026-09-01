use thiserror::Error;

#[derive(Debug, Error)]
pub enum LifecycleError {
    #[error("lifecycle manager is not running")]
    NotRunning,

    #[error("lifecycle manager is already running")]
    AlreadyRunning,

    #[error("invalid state transition from {from} to {to}")]
    InvalidTransition { from: String, to: String },

    #[error("platform lifecycle error: {0}")]
    Platform(String),

    #[error("python bridge error: {0}")]
    PythonBridge(String),

    #[error("timeout waiting for synchronization")]
    SyncTimeout,

    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

pub type LifecycleResult<T> = Result<T, LifecycleError>;
