# Linux Test Bundle (CPython 3.10)

Manual validation package for **system_lifecycle** on Linux with **Python 3.10**.

Uses systemd-logind over D-Bus for shutdown and sleep detection. Delay inhibitors are held as **standby** from `LifecycleManager.start()` so logind waits for `synccomplete()` before continuing (up to `InhibitDelayMaxUSec`, typically 30 seconds).

Lid close on most laptops maps to `PrepareForSleep` (`type=sleep`).

## Quick start

```bash
cd bundle/system-lifecycle-linux-py310-test-bundle
chmod +x install.sh run_test_detached.sh
./install.sh
./run_test_detached.sh
```

Use **desktop Shut Down**, or close the lid. After wake/reboot, open `log.txt` and `lifecycle_native.log`.

`install.sh` creates a local `.venv` with `python3.10`.

See [INSTALL.txt](INSTALL.txt) for full instructions.

## Success criteria

**log.txt**

```text
event detected: type=shutdown
waiting for 6 seconds
wait complete
synccomplete called, returned=True
lifecycle guard released — OS may continue shutdown/sleep
```

or for lid close / sleep:

```text
event detected: type=sleep
waiting for 6 seconds
wait complete
synccomplete called, returned=True
lifecycle guard released — OS may continue shutdown/sleep
```

For shutdown, the test process exits after sync (no further `running` lines).

**lifecycle_native.log**

At startup:

```text
native: standby logind inhibitor installed event=shutdown
native: standby logind inhibitor installed event=sleep
```

On shutdown:

```text
native: armed active logind inhibitor from standby event=shutdown
native: logind delay inhibitor active for event=shutdown
bridge: emitting lifecycle event source=PrepareForShutdown event=shutdown
bridge: python callback enqueued event=shutdown
bridge: sync wait finished completed=true
```

On lid close / sleep:

```text
native: armed active logind inhibitor from standby event=sleep
native: logind delay inhibitor active for event=sleep
bridge: emitting lifecycle event source=PrepareForSleep event=sleep
bridge: python callback enqueued event=sleep
bridge: sync wait finished completed=true
```

## Verify inhibitors (optional)

While the test is running:

```bash
busctl call org.freedesktop.login1 /org/freedesktop/login1 \
  org.freedesktop.login1.Manager ListInhibitors | grep system-lifecycle-extension
```

You should see `shutdown` and `sleep` entries with mode `delay`.

## Do not

- Run `python3 test_lifecycle.py` inside Cursor for shutdown tests.
- Start the test twice without killing the previous process.
- Use `systemctl poweroff -i` (bypasses inhibitors).
- Run without a logind session (e.g. bare SSH without D-Bus).

## Files

| File | Purpose |
|------|---------|
| `wheels/*.whl` | Prebuilt extension (cp310, manylinux) |
| `install.sh` | Create local `.venv` with Python 3.10 and install the wheel |
| `run_test_detached.sh` | Launch test detached (recommended) |
| `test_lifecycle.py` | 6s sync test with logging |
