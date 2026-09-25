# Proposal

## Why

UXD-004. The [activation proof][activation] showed that launchd and
`systemd --user` can own the socket and pass it on stdin. It also measured one
failure: with a 1 s idle timeout and the systemd default start limits, 6 of 25
connections were refused and the socket stayed `failed`. With
`StartLimitBurst=100`, all 25 succeeded.

The first consumer starts its own daemon behind a start lock instead. Two of
its lock tests fail intermittently, and one run admitted two spawners
([extraction map][map]). Its daemon also inherits the client's working
directory, and it unlinks the socket on exit.

## What Changes

1. `unixd` takes its listener from stdin with safe `std` calls, and changes
   its working directory to `/` first.
2. `unixd` installs and uninstalls its own units: a LaunchAgent plist on macOS
   and a socket and service pair on Linux. Both set the restart limits
   explicitly.
3. One drain path serves `SIGINT`, `SIGTERM`, a shutdown request, and the idle
   timer. The daemon never unlinks the socket and writes no PID file.

## Capabilities

### New Capabilities

- `lifecycle`: the stdin listener, unit install and uninstall, drain, and idle
  exit.

### Modified Capabilities

None. The [`activation`][activation-spec] spec covers the example daemon; this
change moves that path into the crate.

## Impact

1. New dependencies in `unixd`: [`tokio-util`][tokio-util] (`rt`) for
   `TaskTracker` and `CancellationToken`, tokio `signal`, and [`plist`][plist]
   for the LaunchAgent file.
2. Installing runs `launchctl` on macOS and `systemctl --user` on Linux.

[activation]: ../../../docs/research/activation-and-cache.md#4-live-activation-measurements
[activation-spec]: ../../specs/activation/spec.md
[map]: ../../../docs/research/extraction-map.md
[plist]: https://crates.io/crates/plist
[tokio-util]: https://crates.io/crates/tokio-util
