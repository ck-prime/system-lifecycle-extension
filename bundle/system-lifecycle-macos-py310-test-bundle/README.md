# macOS Test Bundle (CPython 3.10)

Manual validation package for **system_lifecycle** on macOS with **Python 3.10**.

Same layout and test script as `../system-lifecycle-macos-test-bundle/` (CPython 3.14). The native extension is built for **cp310**.

**Validated native behavior (Apple Silicon, 3.14 bundle):** lid/sleep and Apple menu → Shut Down. Re-run this 3.10 bundle on hardware to confirm the same logs.

## Quick start

```bash
cd bundle/system-lifecycle-macos-py310-test-bundle
chmod +x install.sh run_test_detached.sh
./install.sh
./run_test_detached.sh
```

Use **Apple menu → Shut Down**, or close the lid. After wake/reboot, open `log.txt` and `lifecycle_native.log`.

`install.sh` creates a local `.venv` with `python3.10` (Homebrew Python is PEP 668).

See [INSTALL.txt](INSTALL.txt) for full instructions.

## Success criteria

**log.txt**

```text
event detected: type=shutdown
wait complete
synccomplete called, returned=True
lifecycle guard released — OS may continue shutdown/sleep
```

or for lid close:

```text
event detected: type=sleep
wait complete
synccomplete called, returned=True
lifecycle guard released — OS may continue shutdown/sleep
```

**lifecycle_native.log**

Shutdown:

```text
native: TransformProcessType UIElement status=0
native: loginwindow notify(3) tokens registered
native: notify(3) fired name=com.apple.system.loginwindow.shutdownInitiated
native: com.apple.system.loginwindow.shutdownInitiated sync finished completed=true
```

Lid / sleep:

```text
native: kIOMessageSystemWillSleep
native: kIOMessageSystemWillSleep waiting for synccomplete
native: kIOMessageSystemWillSleep sync finished completed=true
native: IOAllowPowerChange
```

## Do not

- Run `python3 test_lifecycle.py` inside Cursor for shutdown tests.
- Start the test twice without killing the previous process.
- Use `sudo shutdown`.
- Mix this bundle with the 3.14 bundle (different `.venv` / lock files).

## Files

| File | Purpose |
|------|---------|
| `wheels/*.whl` | Prebuilt extension (cp310) |
| `install.sh` | Create local `.venv` with Python 3.10 and install the wheel |
| `run_test_detached.sh` | Launch test detached (recommended) |
| `test_lifecycle.py` | 6s sync test with logging |
