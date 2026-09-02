# Platform Support Matrix

This document distinguishes **detection**, **delay**, **veto/cancellation**, and **honest limitations**.

| Platform | Event | Detection API | Delay / veto mechanism | Max delay behavior | Limitations | Privileges | Validated |
|----------|-------|---------------|------------------------|--------------------|-------------|------------|-----------|
| Linux (Ubuntu 22.04+) | shutdown | logind `PrepareForShutdown` | Standby **delay inhibitor** fd (`what=shutdown`) held from `start()` | Until fd closed or `max_wait_seconds` timeout | Requires systemd-logind; inhibitor must be held before operation starts | Session user | **Yes** |
| Linux | reboot | same as shutdown | same standby inhibitor | same | Reported as `shutdown` during `PrepareForShutdown` (reboot inferred only when safe) | Session user | Manual |
| Linux | sleep / lid | logind `PrepareForSleep` | Standby **delay inhibitor** fd (`what=sleep`) held from `start()` | same | Lid close usually maps to `sleep` | Session user | **Yes** |
| Linux | hibernate | `PrepareForSleep` | same sleep standby inhibitor | same | May appear as `sleep` on older systemd | Session user | Manual |
| macOS | sleep | `kIOMessageSystemWillSleep`, `NSWorkspaceWillSleepNotification` | Delay `IOAllowPowerChange` + `PreventSystemSleep` assertion | Until `synccomplete()` or `max_wait_seconds` (OS also has ~30s WillSleep cap) | Not identical to logind semantics | User session | **Yes (lid/sleep)** |
| macOS | lid close | `kIOPMMessageClamshellStateChange` (`AppleClamshellState`) | Same as sleep (`WillSleep` hold + assertions) | same | Lid close maps to `sleep`; cannot veto lid-close sleep, only delay it | User session | **Yes** |
| macOS | shutdown / reboot | loginwindow `shutdownInitiated` notify + UIElement app | Hold until `synccomplete()` (notify wait + terminate) | Until sync, timeout, or OS force-quit | SIGTERM alone is too late; process should exit after sync | User session | **Shutdown yes** (reboot best-effort) |
| **Windows 10/11** | **shutdown** | `WM_QUERYENDSESSION` | `ShutdownBlockReasonCreate` + **`WM_ENDSESSION` hold** until sync | Until block reason destroyed, sync completes, or timeout | Must be the blocking process (not a child of CMD); message pump required | User session | **Yes** |
| Windows | reboot | `WM_QUERYENDSESSION` | same as shutdown | same | Reboot vs shutdown not always distinct | User session | Best-effort |
| Windows | sleep / hibernate | `WM_POWERBROADCAST` (`PBT_APMSUSPEND`, lid) | `SetThreadExecutionState(ES_SYSTEM_REQUIRED)` | Temporary hold + timeout | Hibernate vs sleep not always distinct | User session | **Sleep validated** |

## Linux shutdown and sleep behavior (validated)

Observed on Ubuntu with GNOME and the test bundle:

1. On `LifecycleManager.start()`, the extension installs **standby** logind delay inhibitors for `shutdown` and `sleep` (fds held open for the process lifetime).
2. User initiates shutdown or closes the lid (`PrepareForShutdown` / `PrepareForSleep`).
3. The standby inhibitor for that event is transferred to the active session; logind waits for the fd to close.
4. Python callback runs the sync window (6s in the test bundle) on a **dedicated Python thread**.
5. `synccomplete()` closes the inhibitor fd; logind continues (up to `InhibitDelayMaxUSec`, typically 30s).
6. After shutdown sync, the test process exits (`os._exit(0)`).

Requirements for production apps on Linux:

- Call `LifecycleManager.start()` early — delay inhibitors cannot be acquired after logind has started the operation (`OperationInProgress`).
- After shutdown/reboot sync, **terminate the process** so the session can finish.
- Do not use `systemctl poweroff -i` or equivalent inhibitor bypass flags in tests.
- Requires a desktop logind session (not headless SSH without D-Bus).

Test bundle: `bundle/system-lifecycle-linux-py310-test-bundle/`

## Windows shutdown behavior (validated)

Observed on Windows 10/11 with the test bundle:

1. User initiates shutdown (Start menu — not `shutdown /p`).
2. Extension registers block reason: *Application synchronization in progress*.
3. Returns **`TRUE`** from `WM_QUERYENDSESSION` so shutdown is not cancelled.
4. Holds on **`WM_ENDSESSION`** until Python calls `synccomplete()` (up to `max_wait_seconds`).
5. Python callback runs on a **dedicated Python thread** (not the Rust worker).
6. After sync, block reason is destroyed and the **host process exits** so shutdown continues automatically.

Requirements for production apps on Windows:

- Call `LifecycleManager.start()` from the process that should block shutdown (your service/app), not from a short-lived wrapper script.
- After shutdown/reboot sync, **terminate the process** (the test uses `os._exit(0)` after `synccomplete()`).
- Avoid running under CMD/IDE terminals for shutdown-sensitive workloads — they appear separately in the shutdown UI.

## macOS shutdown and lid behavior (validated)

Observed on Apple Silicon with the test bundle:

1. Listener transforms the process to a UIElement app and registers loginwindow `notify(3)` plus IOKit sleep/lid.
2. Lid close: `kIOPMMessageClamshellStateChange` then `kIOMessageSystemWillSleep`; `IOAllowPowerChange` is delayed until `synccomplete()`.
3. Shut Down: `com.apple.system.loginwindow.shutdownInitiated` fires; Python runs the 6s sync; native wait finishes `completed=true`; logout continues.
4. After shutdown sync, the test process exits (`os._exit(0)`).

Requirements for production apps on macOS:

- Call `LifecycleManager.start()` from the process that should delay shutdown (not a short-lived wrapper).
- After shutdown/reboot sync, **terminate the process**.
- Do not rely on SIGTERM: loginwindow may SIGKILL before a multi-second sync finishes.
- Homebrew Python cannot `pip install` this wheel system-wide (PEP 668); use a venv.

## Terminology

- **Detection**: Rust learns that a transition is imminent.
- **Delay**: OS transition is temporarily held while the extension waits for Python sync.
- **Notification**: Python is informed; OS may not be materially delayed if the platform has no hold API.
- **macOS shutdown**: Validated via loginwindow `shutdownInitiated` notify on a UIElement process. SIGTERM/SIGKILL alone is not a delay. Production apps should **exit after shutdown/reboot sync**.
- **Veto / cancellation**: This extension does not cancel shutdown session-wide (no `shutdown /a`).

## Containers and CI

If logind, AppKit session APIs, or a Windows message pump are unavailable, lifecycle detection may fail gracefully at startup. Use `MockLifecycle` in automated tests instead of real OS events.
