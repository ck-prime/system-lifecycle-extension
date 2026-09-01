# Platform Support Matrix

This document distinguishes **detection**, **delay**, **veto/cancellation**, and **honest limitations**.

| Platform | Event | Detection API | Delay / veto mechanism | Max delay behavior | Limitations | Privileges |
|----------|-------|---------------|------------------------|--------------------|-------------|------------|
| Linux (Ubuntu 22.04+) | shutdown | logind `PrepareForShutdown` + `ScheduledShutdown` | logind **delay inhibitor** fd | Until fd closed or `max_wait_seconds` timeout in extension | Requires systemd-logind on system bus; headless/container environments may lack logind | Session user |
| Linux | reboot | same as shutdown | same inhibitor (`what` includes reboot handling) | same | Reboot vs shutdown inferred from `ScheduledShutdown` when available | Session user |
| Linux | sleep / lid close | logind `PrepareForSleep` | logind delay inhibitor (`what=sleep`) | same | Lid close usually maps to sleep, not shutdown; older systemd may report hibernate as sleep | Session user |
| Linux | hibernate | `PrepareForSleep` + `SleepOperation` when available | logind delay inhibitor | same | Property may be absent on older systemd; event may appear as `sleep` | Session user |
| macOS Intel/Apple Silicon | sleep | `NSWorkspaceWillSleepNotification` | `IOPMAssertion` prevent idle system sleep | Bounded by extension timeout + OS policy | Not identical to logind delay semantics | User session |
| macOS | shutdown | `NSWorkspaceWillPowerOffNotification` | **No true shutdown inhibitor equivalent** | Notification only; extension timeout still applies to internal wait state | Cannot guarantee shutdown delay like Linux/Windows block reason | User session |
| macOS | reboot | grouped with shutdown notification | notification only | same | same | User session |
| Windows 10/11 | shutdown | `WM_QUERYENDSESSION` | `ShutdownBlockReasonCreate` | Until block reason destroyed or timeout | Must run in interactive session with message queue | User session |
| Windows | reboot | `WM_QUERYENDSESSION` heuristics | `ShutdownBlockReasonCreate` | same | Reboot detection is best-effort via session parameters | User session |
| Windows | sleep / hibernate | `WM_POWERBROADCAST` (`PBT_APMSUSPEND`) | `SetThreadExecutionState(ES_SYSTEM_REQUIRED)` | Temporary execution state hold + timeout | Hibernate vs sleep not always distinguishable | User session |

## Terminology

- **Detection**: Rust learns that a transition is imminent.
- **Delay**: OS transition is temporarily held while the extension waits for Python sync.
- **Notification**: Python is informed; OS may not be materially delayed.
- **Veto / cancellation**: Not used; this extension does not run shell commands like `shutdown /a`.

## Containers and CI

If logind, AppKit session APIs, or a Windows message pump are unavailable, lifecycle detection may fail gracefully at startup with logged errors. Use `MockLifecycle` in automated tests instead of real OS events.
