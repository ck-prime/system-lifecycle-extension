# Testing Guide

## Test tiers

| Tier | Location | Runs automatically | Purpose |
|------|----------|--------------------|---------|
| Unit | `src/**` `#[cfg(test)]`, `tests/state_test.rs` | Yes (`cargo test`) | State machine, mock guards |
| Integration | `tests/lifecycle_integration.rs` | Yes | Rust ↔ Python callback, timeout, exceptions |
| Real OS | Manual in VM | **No** | Shutdown/sleep/reboot/lid close |

## Safe CI policy

`cargo test`, `pytest`, and `maturin develop` **must not** initiate real shutdown, reboot, or sleep.

Use `MockLifecycle` / injected events for automated coverage.

## Local development

```bash
export PYO3_USE_ABI3_FORWARD_COMPATIBILITY=1  # Python 3.14+
maturin develop --release
cargo test --all-features
python python/test_lifecycle.py --sync-delay 5
```

## Manual OS test matrix

| Platform | Version | shutdown | reboot | sleep | lid close | callback | synccomplete | timeout | callback exception | duplicate synccomplete |
|----------|---------|----------|--------|-------|-----------|----------|--------------|---------|--------------------|------------------------|
| Ubuntu | 22.04 | manual | manual | manual | manual | auto in script | manual | `--never-complete` | `--raise-error` | call twice |
| Ubuntu | 24.04 | manual | manual | manual | manual | auto | manual | `--never-complete` | `--raise-error` | call twice |
| Ubuntu | 26.04 | manual | manual | manual | manual | auto | manual | `--never-complete` | `--raise-error` | call twice |
| macOS | Intel | manual | manual | manual | N/A | auto | manual | `--never-complete` | `--raise-error` | call twice |
| macOS | Apple Silicon | manual | manual | manual | N/A | auto | manual | `--never-complete` | `--raise-error` | call twice |
| Windows | 10 | manual | manual | manual | N/A | auto | manual | `--never-complete` | `--raise-error` | call twice |
| Windows | 11 | manual | manual | manual | N/A | auto | manual | `--never-complete` | `--raise-error` | call twice |

Additional checks during manual runs:

- Python process remains stable (no deadlock)
- Network-independent behavior (local sync simulation only)
- OS transition continues after `synccomplete()` or timeout

## VM testing

Use isolated VMs/snapshots:

### Linux

- Ubuntu 22.04, 24.04, 26.04 (x86_64 and ARM64 where available)
- Install desktop or logind-enabled session
- Snapshot before each manual lifecycle test

### Windows

- Windows 10 and 11 x86_64 VMs
- Snapshot before shutdown/sleep tests

### macOS

- Test on supported Apple hardware or licensed macOS VM per Apple policy
- Do not claim macOS coverage from Linux/Windows CI builds alone

## Mock platform tests (every dev OS)

`MockLifecycle` supports injected events:

- shutdown / reboot / sleep / hibernate
- callback invoked once per active session
- `synccomplete()` releases wait
- timeout releases wait
- duplicate `synccomplete()` is safe

Run:

```bash
cargo test --test lifecycle_integration
```

## Python GIL / deadlock tests

Integration tests verify:

- Python callback execution under GIL
- Rust listener waiting **without** holding GIL
- Callback exceptions logged without crashing the extension
- Timeout path releases wait safely

For interactive GIL stress testing, run the Python script and invoke `synccomplete()` from another Python thread while handling an event.
