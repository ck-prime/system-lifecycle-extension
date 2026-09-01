# Testing Guide

## Test tiers

| Tier | Location | Runs automatically | Purpose |
|------|----------|--------------------|---------|
| Unit | `src/**` `#[cfg(test)]` | Yes (`cargo test`) | State machine, mock guards |
| Integration | `tests/lifecycle_integration.rs` | Yes | Rust ↔ Python callback, timeout, exceptions |
| Real OS | Manual in VM / on hardware | **No** | Shutdown/sleep/reboot/lid close |

## Safe CI policy

`cargo test`, `pytest`, and `maturin develop` **must not** initiate real shutdown, reboot, or sleep.

Use `MockLifecycle` / injected events for automated coverage.

## Local development

```bash
export PYO3_USE_ABI3_FORWARD_COMPATIBILITY=1  # Python 3.14+
maturin develop --release
cargo test --all-features
python python/test_lifecycle.py
```

The repo `python/test_lifecycle.py` uses a 60-second sync delay for interactive manual testing. For faster Windows shutdown validation, use the test bundle (6-second sync).

On Windows, put Python DLLs on `PATH` before `cargo test`.

## Windows test bundle (validated shutdown)

Location: `bundle/system-lifecycle-windows-test-bundle/`

Includes a cp314 wheel, detached launcher, and logging test script.

```powershell
cd /d path\to\bundle\system-lifecycle-windows-test-bundle
py -3.14 -m pip install --force-reinstall wheels\system_lifecycle-0.1.0-cp314-cp314-win_amd64.whl
powershell -ExecutionPolicy Bypass -File run_test_detached.ps1
```

Then **Start menu → Shut down** (not `shutdown /p`).

### Expected success (`log.txt`)

```text
event detected: type=shutdown
waiting for 6 seconds
wait complete
synccomplete called, returned=True
lifecycle guard released — OS may continue shutdown/sleep
```

No duplicate `event detected` lines. No `running` lines after sync (process exits).

### Expected success (`lifecycle_native.log`)

```text
native: ShutdownBlockReasonCreate succeeded
native: WM_QUERYENDSESSION returning allow=true handled=true
native: WM_ENDSESSION waiting for synccomplete
native: WM_ENDSESSION sync finished completed=true
native: PostQuitMessage sent after shutdown sync
```

### Common mistakes

| Mistake | Symptom |
|---------|---------|
| Running `py test_lifecycle.py` inside CMD/Cursor | CMD/Cursor listed in shutdown UI; duplicate handlers |
| Two test processes running | Duplicate log lines, garbled `lifecycle_native.log` |
| `shutdown /p` (force) | Bypasses normal session shutdown flow |
| Forgetting to reinstall wheel after rebuild | Old native behavior in logs |

Kill stray `pythonw.exe` in Task Manager before retesting.

## Manual OS test matrix

| Platform | Version | shutdown | reboot | sleep | lid close | callback | synccomplete | timeout | callback exception | duplicate synccomplete |
|----------|---------|----------|--------|-------|-----------|----------|--------------|---------|--------------------|------------------------|
| Ubuntu | 22.04 / 24.04 / 26.04 | manual | manual | manual | manual | auto | manual | omit `synccomplete()` | raise in callback | call twice |
| macOS | Intel / Apple Silicon | manual | manual | manual | N/A | auto | manual | omit `synccomplete()` | raise in callback | call twice |
| **Windows** | **10 / 11** | **validated** | manual | **validated** | manual | auto | manual | omit `synccomplete()` | raise in callback | call twice |

For timeout, exception, and duplicate-`synccomplete()` behavior, prefer `cargo test --test lifecycle_integration` (automated mock platform).

Additional checks during manual runs:

- Python process remains stable (no deadlock)
- OS transition continues after `synccomplete()` or timeout
- On Windows shutdown: machine powers off without a second “Shut down anyway” click

## VM testing

Use isolated VMs/snapshots. Snapshot before each manual lifecycle test.

- **Linux**: Ubuntu 22.04, 24.04, 26.04 (x86_64 and ARM64)
- **Windows**: Windows 10 and 11 x86_64
- **macOS**: Supported Apple hardware (licensed VM per Apple policy)

## Mock platform tests

`MockLifecycle` supports injected events without touching the OS:

```bash
cargo test --test lifecycle_integration
```

## Python GIL / callback thread

- Lifecycle callbacks are dispatched on a **dedicated Python thread** (queue + worker), not from the Rust platform worker.
- The controller worker waits for `synccomplete()` **without** holding the GIL.
- Callback exceptions are logged; the session remains until `synccomplete()` or timeout.
