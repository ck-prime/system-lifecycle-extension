# Rust unit tests

Library unit tests live alongside modules under `src/**`.

Integration tests:

- `tests/lifecycle_integration.rs` — mock lifecycle + Python callback flows (sync, timeout, exceptions, duplicate `synccomplete()`)

Real OS lifecycle tests (shutdown/sleep/lid) are manual only.

- Windows: `bundle/system-lifecycle-windows-test-bundle/`
- macOS 3.14: `bundle/system-lifecycle-macos-test-bundle/`
- macOS 3.10: `bundle/system-lifecycle-macos-py310-test-bundle/`

Run:

```bash
export PYO3_USE_ABI3_FORWARD_COMPATIBILITY=1  # Python 3.14+
cargo test --all-features
```

On Windows, ensure Python DLLs are on `PATH` before running tests.
