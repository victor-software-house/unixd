# unixd

Two Rust crates for the shape a per-user background service keeps taking:
a socket-activated daemon, and a bounded on-disk cache in front of a slow
upstream.

| Crate | Owns | Async runtime |
|:--|:--|:--|
| `unixd` | Activation, unit install, peer identity, bounded framing, drain lifecycle, single flight, secrets in memory | [Tokio][tokio] |
| `unixd-cache` | Freshness horizons, stale-if-error policy, cross-process locks, atomic writes, bounds | None |

They are separate crates because consumers need different halves. A local
message broker needs the runtime shell and must not hold a cache of anything.
A CLI with an HTTP upstream needs both, and needs the cache to work in its
direct no-daemon mode, which has no reactor at all. One crate would force
Tokio onto the second consumer's direct path and a cache onto the first.

## Status

Both crates are implemented and unreleased (`0.0.0`). `unixd` has the
transport, serving from the service manager's socket with drain and idle
exit, unit install for launchd and systemd, single flight, and an in-memory
secret store. Read [`docs/design/daemon-and-cache.md`](docs/design/daemon-and-cache.md)
for the contract, and [`docs/research/`](docs/research/) for the evidence
behind it, before changing code.

The install tests change the user's launchd or systemd state, so they are
ignored by default; [`crates/unixd/tests/install.rs`](crates/unixd/tests/install.rs)
says how to run them.

## Scope

macOS on Apple Silicon and Linux. The platform service manager owns the
socket and starts the daemon on demand: [launchd][launchd] on macOS,
[`systemd --user`][systemd] on Linux. The daemon never spawns itself.

Not in scope: self-spawn, other platforms, an HTTP adapter, pub/sub, a
mailbox, a durable message queue, or a heartbeat.

## License

MIT.

[launchd]: https://keith.github.io/xcode-man-pages/launchd.plist.5.html
[systemd]: https://www.freedesktop.org/software/systemd/man/latest/systemd.socket.html
[tokio]: https://tokio.rs
