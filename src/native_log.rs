use parking_lot::Mutex;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

static NATIVE_LOG_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);
static NATIVE_LOG_WRITE: Mutex<()> = Mutex::new(());

pub fn set_log_path(path: PathBuf) {
    *NATIVE_LOG_PATH.lock() = Some(path);
}

pub fn native_log(message: &str) {
    let _write_guard = NATIVE_LOG_WRITE.lock();
    let path = NATIVE_LOG_PATH
        .lock()
        .clone()
        .unwrap_or_else(default_log_path);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "[{timestamp}ms] {message}");
        let _ = file.flush();
    }
}

fn default_log_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_default()
        .join("lifecycle_native.log")
}
