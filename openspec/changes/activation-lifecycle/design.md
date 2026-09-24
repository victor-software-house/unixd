# Design

The evidence is in the [activation research][activation] and the
[transport and lifecycle research][research].

## Decisions

| # | Decision | Alternative | Why the alternative lost |
|--:|:--|:--|:--|
| 1 | Take the listener from stdin on both platforms. | `launch_activate_socket` on macOS and [`listenfd`][listenfd] on Linux. | The macOS call needs `unsafe` to adopt a raw descriptor, and the stdin path passed its live proof on both platforms. |
| 2 | Change the working directory to `/` before taking the listener. | Rely on activation alone. | A guard that costs one call makes the stranded-directory failure impossible even if a caller runs the serve entry by hand. |
| 3 | Socket at `$XDG_RUNTIME_DIR/<name>/<name>-v<major>.sock` on Linux, written as `%t/…` in the unit. On macOS, `~/Library/Application Support/<name>/<name>-v<major>.sock`, written as an absolute path. | `$TMPDIR` on macOS. | The macOS temporary directory is cleaned by the system, and launchd does not expand variables in `SockPathName`. |
| 4 | Refuse to install when the socket path exceeds 103 bytes on macOS or 107 on Linux. | A hashed fallback path, as fnox has. | The installer chooses the path, so it can report the limit instead of hiding it. |
| 5 | Write the plist with the [`plist`][plist] crate. | A text template. | Paths and labels need XML escaping, which the crate does. |
| 6 | Set `StartLimitIntervalSec=10` and `StartLimitBurst=100` on the service, and `TriggerLimitIntervalSec=2` and `TriggerLimitBurst=200` on the socket. On macOS set `ThrottleInterval` to 1. | The defaults. | The defaults refused 6 of 25 connections in the measured run. |
| 7 | Drain with `CancellationToken` and `TaskTracker`, then return from `main`. | [tokio-graceful-shutdown][tgs]. | The framework adds four dependencies for a subsystem tree nobody needs. |
| 8 | The idle timer runs only while no connection is open, and restarts after the last one closes. | A timer from the last accept. | A long request would otherwise be cut by the idle exit. |
| 9 | Default idle timeout of 10 minutes; `0` disables it. | The consumer's 4 hours. | The restart limits in decision 6 make short timeouts safe, and a daemon holding memory for hours with no clients has no benefit. |
| 10 | Install and uninstall are idempotent and touch only files named by this label. | Remove any matching unit. | The installer must never remove another program's unit. |

## Overrides and disable paths

| Automatic behaviour | Override | Disable path |
|:--|:--|:--|
| Idle exit | `idle_timeout` | `0` |
| Drain grace period | `drain_timeout` | none; a bound is required |
| Restart limits in the units | `InstallOptions` | none; the defaults are measured to fail |

## Risks

1. launchd's man page asks new jobs to avoid `inetdCompatibility`. No Apple
   statement deprecates it, and nine macOS 26.6.2 system jobs use it.
2. Installing changes per-user launchd and systemd state. Tests install under
   a unique test label and uninstall at the end.

[activation]: ../../../docs/research/activation-and-cache.md
[listenfd]: https://crates.io/crates/listenfd
[plist]: https://crates.io/crates/plist
[research]: ../../../docs/research/transport-and-lifecycle.md
[tgs]: https://crates.io/crates/tokio-graceful-shutdown
