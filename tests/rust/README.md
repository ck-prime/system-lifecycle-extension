# Rust unit tests

Library unit tests live alongside modules under `src/**`.

Integration tests:

- `tests/lifecycle_integration.rs` — mock lifecycle + Python callback flows

Run:

```bash
export PYO3_USE_ABI3_FORWARD_COMPATIBILITY=1  # Python 3.14+
cargo test --all-features
```

On Windows, ensure Python DLLs are on `PATH` before running tests.
