# Design

## Decisions

| # | Decision | Alternative | Why the alternative lost |
|--:|:--|:--|:--|
| 1 | The platform service manager owns the socket and starts the daemon. | Self-spawn behind a start lock. | Every self-spawn step is a race window. The first consumer's start lock admitted two daemons in a test run, and a spawned daemon can be stranded in a removed directory. |
| 2 | launchd on macOS and `systemd --user` on Linux. | launchd only. | The daemon must run on Linux. |
| 3 | The socket arrives on standard input: launchd `inetdCompatibility` `Wait = true`, systemd `StandardInput=socket`. | `launch_activate_socket` through `raunch`, and `LISTEN_FDS` through `listenfd`. | `raunch` returns a raw descriptor that the caller must adopt with `unsafe`, which the workspace forbids. `listenfd` stays the Linux fallback if the stdin path fails its proof. |
| 4 | unixd writes the plist and the two systemd units itself. | `service-manager`. | It writes only a `.service` on systemd, with no socket unit, and uses the legacy `launchctl load`. |
| 5 | Extract the first consumer's daemon, transport, and cache. | Write unixd fresh from the design. | The consumer's code is tested and in use; a second implementation would drift. |
| 6 | Hand-write the cache on `std::fs::File::lock` and `tempfile`. | `cacache`, `foyer`, `moka`, `http-cache`. | None supports stale-if-error; `cacache` has no cross-process locks or bounded prune; `foyer` and `http-cache` are async; `moka` is in memory. |

## Overrides and disable paths

| Automatic behaviour | Override | Disable path |
|:--|:--|:--|
| Idle exit | the idle timeout | a timeout of none |
| launchd relaunch throttle | the installer's `ThrottleInterval` | not disableable; set it low |
| systemd trigger and start limits | the installer's `TriggerLimitBurst` and `StartLimitBurst` | not disableable; set them high |

## Risks

1. launchd's man page asks new jobs to avoid `inetdCompatibility`. If the live
   proof shows a defect, the fallback on macOS is `raunch`, which needs one
   `unsafe` call, and that needs the maintainer's decision.
2. Whether launchd keeps a waiting connection across a relaunch with this key
   is documented for on-demand jobs in general, not for this key. The proof
   measures it.
