# unixd

Two Rust crates for the shape a per-user background service keeps taking:
a socket-activated daemon, and a bounded on-disk cache in front of a slow
upstream.

| Crate | Owns | Async runtime |
|:--|:--|:--|
| `unixd` | Activation, listener ownership, peer identity, bounded framing, drain lifecycle | Tokio |
| `unixd-cache` | Freshness horizons, stale-if-error policy, cross-process locks, atomic writes, bounds | None |

They are separate crates because consumers need different halves. A local
message broker needs the runtime shell and must not hold a cache of anything.
A CLI with an HTTP upstream needs both, and needs the cache to work in its
direct no-daemon mode, which has no reactor at all. One crate would force
Tokio onto the second consumer's direct path and a cache onto the first.

## Status

Unimplemented. The repository carries the design and the scaffold; both
crates are `0.0.0` with empty public surfaces. Read
[`docs/design/daemon-and-cache.md`](docs/design/daemon-and-cache.md) for the
contract before adding code.

## Scope

Unix-first, macOS first. Version one ships service activation only, behind an
`Activation` trait so a self-spawning implementation and a systemd
`LISTEN_FDS` implementation can arrive without touching a call site.

Not in scope: a self-spawn fallback, systemd, Linux support, an HTTP adapter,
pub/sub, a mailbox, a durable message queue, or a heartbeat.

## License

MIT.
