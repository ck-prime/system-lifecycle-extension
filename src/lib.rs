mod bridge;
mod errors;
mod logging;
mod native_log;

pub mod lifecycle;

use bridge::register;
use pyo3::prelude::*;

#[pymodule]
fn system_lifecycle(m: &Bound<'_, PyModule>) -> PyResult<()> {
    register(m)
}
