use tracing_subscriber::{fmt, EnvFilter};

/// Initialize structured logging for the extension.
///
/// Respects `RUST_LOG` when set; defaults to `info` for this crate.
pub fn init_logging() {
    let _ = fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("system_lifecycle=info")),
        )
        .with_target(false)
        .try_init();
}
