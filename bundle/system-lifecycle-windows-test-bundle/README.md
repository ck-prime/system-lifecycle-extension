# Windows Test Bundle

Manual validation package for **system_lifecycle** on Windows 10/11 (Python 3.14, x64).

## Quick start

```powershell
cd /d path\to\system-lifecycle-windows-test-bundle
py -3.14 -m pip install --force-reinstall wheels\system_lifecycle-0.1.0-cp314-cp314-win_amd64.whl
powershell -ExecutionPolicy Bypass -File run_test_detached.ps1
```

Use **Start menu → Shut down**. After reboot, open `log.txt` and `lifecycle_native.log`.

See [INSTALL.txt](INSTALL.txt) for full instructions.

## Success criteria

**log.txt**

```text
event detected: type=shutdown
wait complete
synccomplete called, returned=True
lifecycle guard released — OS may continue shutdown/sleep
```

Single `event detected` block. No `running` lines after sync.

**lifecycle_native.log**

```text
native: WM_ENDSESSION sync finished completed=true
native: PostQuitMessage sent after shutdown sync
```

## Do not

- Run `py test_lifecycle.py` inside CMD or Cursor for shutdown tests (adds extra processes to the shutdown UI).
- Start the test twice without killing old `pythonw.exe` processes.
- Use `shutdown /p` (bypasses normal session shutdown).

## Files

| File | Purpose |
|------|---------|
| `wheels/*.whl` | Prebuilt extension |
| `run_test_detached.ps1` / `.cmd` | Launch test via `pythonw` (recommended) |
| `test_lifecycle.py` | 6s sync test with logging |
| `log.txt` | Python output |
| `lifecycle_native.log` | Native bridge debug log |
| `.lifecycle_test.lock` | Single-instance lock (created at runtime) |
