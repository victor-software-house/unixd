# Design: a socket-activated per-user daemon, and a cache in front of it

- **Status:** Proposed, revised 2026-09-24
- **Scope:** both crates in this workspace
- **Platforms:** macOS on Apple Silicon and Linux. Nothing else.
- **Evidence:** [`docs/research/`][research]: activation and cache,
  transport and lifecycle, and the first consumer's extraction map

## What this decides

Two crates, separately selectable:

| Crate | Owns | Async runtime |
|:--|:--|:--|
| `unixd` | Activation, listener ownership, peer identity, bounded framing, lifecycle, unit install, client transport | [Tokio][tokio] |
| `unixd-cache` | Freshness, stale-if-error policy, cross-process locks, atomic writes, bounds, prune | None |

## Why two crates rather than one

The two known consumer shapes need different halves.

1. A local message broker needs the runtime shell and is explicitly forbidden
   from holding a refreshed cache of anything. It takes `unixd` alone.
2. A CLI with an HTTP upstream and a disk cache needs both, and needs the cache
   to work identically in its direct no-daemon mode, which has no async runtime
   at all. It takes both, and `unixd-cache` must not require a reactor.

One crate containing both would force a [Tokio][tokio] dependency on the direct path of
consumer 2 and a cache module on consumer 1. Neither is acceptable, so the
split is a requirement.

## Where the code comes from

The first consumer already runs a working daemon and cache of its own: a
Unix-socket server with drain and idle exit, a same-UID peer check,
newline-delimited JSON frames capped at 16 MiB, and a disk cache with fresh and
stale horizons, per-key file locks, atomic writes, and prune. Its tests cover
each of these.

unixd is extracted from that code, not written fresh. Each slice moves one
proven piece, generalizes it over the consumer's own types, and holds it to
this document. Where the two disagree, this document wins and the consumer
changes when it adopts the crate.

The consumer's code is tied to its domain in two ways the extraction removes:

1. Transport returns the consumer's error type and dispatches the consumer's
   operations. `unixd` takes a request handler trait and a caller-owned error.
2. The cache names each cached request kind in one enum and has a lookup and a
   store method per kind. `unixd-cache` takes caller-built keys and a caller
   policy, and stores any serializable value.

The one piece that is not extracted is how the daemon starts. The consumer
spawns its own daemon behind a start lock. That is the design this crate
rejects; see the next section.

## Optional presentation seam

Neither crate renders anything. A consumer that wants a daemon status or cache
statistics to look native in its own output can depend on a semantic document
layer and have these crates emit documents rather than strings. That dependency
must stay optional and must not pull a terminal renderer, a serializer, or an
argument parser into either crate.

## Layer A: `unixd`

### Activation: the platform service manager owns the socket

The daemon never starts itself. The platform service manager creates the
socket, listens on it, and starts the daemon on the first connection:

| Platform | Manager | Unit |
|:--|:--|:--|
| macOS | [`launchd`][launchd-plist], per-user agent | a LaunchAgent plist with a `Sockets` entry |
| Linux | [`systemd --user`][systemd-socket] | a `.socket` unit with `Accept=no` and a matching `.service` |

Self-spawn is out, not deferred. It needs a start lock, a readiness poll,
stale-socket adoption, and convergence tests for concurrent first clients.
Every one of those is a race window. The first consumer's own start-lock tests
fail intermittently, and in one run the lock let two daemons spawn. A
self-spawned daemon also inherits the launching client's working directory;
when that directory is removed, the daemon is stranded. Activation removes the
whole class, because no client process is ever the parent.

### How the daemon receives its socket

Both managers can hand the listening socket to the process on standard input:

- launchd: `inetdCompatibility` with `Wait = true`;
- systemd: [`StandardInput=socket`][systemd-exec] on a single-socket `Accept=no` unit.

The daemon then takes it with safe standard-library calls only:
`stdin().as_fd().try_clone_to_owned()`, `UnixListener::from`,
`set_nonblocking(true)`, and [`tokio::net::UnixListener::from_std`][tokio]. No FFI and no
`unsafe`, so the workspace `unsafe_code = "forbid"` stands.

This path is chosen over the named-socket APIs for one reason. No maintained
crate hands a macOS caller an owned listener from `launch_activate_socket`
without the caller writing `unsafe` to adopt a raw descriptor. On Linux,
[`listenfd`][listenfd] does return an owned listener, and it is the fallback if the stdin
path fails its live proof. launchd's man page asks new jobs to avoid
`inetdCompatibility`; the live proof in slice 1 decides whether that warning
matters here.

`Activation` stays a trait with one implementation per platform. Nothing
outside the activation module branches on the platform.

### Two guards against a stranded working directory

Activation already prevents the failure. Two guards make that explicit rather
than incidental:

1. the serve entry point changes its working directory to `/` before it takes
   the listener;
2. runtime and cache roots are resolved to absolute paths once at startup and
   never re-resolved against a relative base.

### Installation contract

`unixd` writes the units itself. [`service-manager`][service-manager] cannot express a systemd
socket and service pair, and no other maintained crate can either.

- macOS: write the plist to `~/Library/LaunchAgents/`, then
  `launchctl bootstrap gui/<uid>`; remove with `launchctl bootout`. Never the
  legacy `load` and `unload`.
- Linux: write the two units to `~/.config/systemd/user/`, then
  `systemctl --user daemon-reload` and `systemctl --user enable --now <name>.socket`.
- one per-user socket, mode `0600`, in a `0700` directory;
- launchd `KeepAlive` unset, so the job runs only on demand;
- service label and socket name carry the protocol major version;
- install and uninstall are idempotent and remove only state this identity owns.

### Peer identity

Every accepted connection is checked for the same effective UID as the serving
process, with [`tokio::net::UnixStream::peer_cred()`][tokio-ucred], before any frame is read.
It uses `getpeereid` on macOS and `SO_PEERCRED` on Linux. A mismatch closes the
connection without a reply and without a log entry containing the peer's
identity.

### Framing

Newline-delimited JSON with a maximum frame size enforced on read. An oversize
frame is a protocol error that closes the connection; it is never buffered to
find its end. Request and response envelopes are versioned, and the major
version appears in both the envelope and the socket name.

### Lifecycle

One drain path shared by `SIGINT`, `SIGTERM`, a protocol shutdown request, and
the idle timer:

1. stop accepting new connections;
2. allow in-flight requests a bounded grace period;
3. abort what remains;
4. exit 0.

The manager owns the socket, so the daemon never unlinks the socket path and
never calls `shutdown(2)` on the listener. Connections that arrive after step 1
wait in the manager's queue and start the next instance.

Idle timeout is configurable and disableable. There is no PID file.

Both managers limit restarts, and an idle exit counts as a stop:

- launchd documents a `ThrottleInterval` of 10 s between launches. Measured on
  macOS, socket-triggered relaunches after an idle exit were not delayed by it
  (17 ms), so the installer still sets it but no request waits on it;
- systemd fails the service after `StartLimitBurst` starts (5 per 10 s by
  default), and the socket after `TriggerLimitBurst` activations (20 per 2 s by
  default for `Accept=no`). A socket failed this way refuses every connection
  until `systemctl --user reset-failed`. Measured on Linux with a 1 s idle
  timeout and a connection every 0.9 to 1.2 s: under the defaults the service
  hit its start limit after 10 starts, the socket entered `failed`, and 6 of 25
  connections were refused. With `StartLimitBurst=100` in a 10 s interval, all
  25 connections succeeded across 14 starts, including connections that
  arrived while the daemon was exiting.

The default idle timeout must be long enough that neither limit is reached in
normal use. The installer sets both limits explicitly rather than inheriting
the defaults.

### Measured activation

The throwaway [`examples/activation.rs`][example] ran under both managers on
2026-09-24. No connection was refused on either platform, the socket survived
every idle exit, and relaunches took 14 to 28 ms. The stdin listener stands,
and the `listenfd` fallback is not needed. The
[measurements][measurements] are in the research record.

## Layer B: `unixd-cache`

Synchronous, no reactor, no database. Versioned validated JSON envelopes on
disk; directories `0700`, files `0600`. No existing crate covers stale-if-error,
so the policy is this crate's own code; the pieces under it come from `std` and
one small crate.

### Freshness has two independent horizons

Each entry stores `fresh_until` and `stale_until` as separate absolute
timestamps, plus the provenance of the policy that set them. Policy comes either
from a caller-supplied default or from a response `Cache-Control` directive;
provenance records which, so an entry written under one regime is not silently
reinterpreted under another.

`Cache-Control` parsing sits behind an optional `http` feature built on
[`http-cache-semantics`][http-cache-semantics], which is synchronous. That crate does not parse
`stale-if-error`, so this crate reads that one directive itself. Without the
feature, the caller passes an already-parsed policy and the crate has no HTTP
dependency.

A schema-version bump migrates existing entries to **stale-only**. It neither
wipes them nor treats them as fresh.

### The stale-if-error matrix is the contract

Serve a stale entry only for:

- a request timeout;
- a network-layer failure;
- HTTP `429`;
- HTTP `5xx`.

Never serve stale for:

- `401` or `403`;
- `404`;
- a response that is not valid JSON;
- a response that does not match the expected schema.

Only successful normalized responses are cached. There is no negative caching.

### Keys

The caller supplies named key parts; the crate sorts them and hashes them
with the crate's own entry format version. The caller's payload schema is
stored in each entry, not hashed into the key, so a schema bump still finds
the old entries and serves them as stale. The crate does not inspect part
names or values; what goes into a key is the caller's choice.

### Atomicity, locking, and single flight

- validate on every read; a corrupt or schema-invalid entry is a miss, removed
  only under its key lock or by prune;
- one file lock per key, [`std::fs::File::lock`][file-lock], held across direct and daemon
  processes alike; the holder deletes the lock file before unlocking, so lock
  files exist only while held;
- a maintenance lock that key holders and prune take shared and only `clear`
  takes exclusively, plus `prune.lock` so one prune runs at a time; prune
  removes each file under its key's lock taken without waiting, and skips a
  held key;
- write to a temporary file in the same directory with [`tempfile`][tempfile], `fsync` it,
  rename atomically with `persist`, then `fsync` the parent directory;
- the daemon coalesces concurrent identical requests in process; the direct
  path does not, and the key lock makes a second process wait for the first
  one's entry. See the [coalescing change][coalescing].

Disk stays canonical. There is no daemon-only in-memory copy that can diverge
from it.

### Bounds

Neither macOS nor Linux cleans a plain program's cache directory, so the
cache bounds itself. Every limit is a `Limits` field with a default; the
[maintenance research][maintenance] compares them with other tools.

- an entry above the per-entry ceiling (8 MiB) is returned to the caller but
  not stored;
- hard caps on both entry count and total bytes (10,000 and 256 MiB);
- prune removes expired entries, then entries unused for 30 days, then the
  least recently used, down to a low-water target (8,000 and 200 MiB);
- a hit records its use at most hourly;
- prune runs after a write crosses a hard cap, and at most daily after any
  write; a consumer's daemon may also call it at startup;
- the root defaults to the per-user cache directory, tagged with
  `CACHEDIR.TAG` so backup tools skip it.

`Limits` deserializes from the consumer's own config; the crates read no
config file.

### Secrets

Secrets never go into the disk cache, encrypted or not. A daemon that needs
to hold one keeps it in memory, in the [secret store][secret-store].

## Non-goals

No self-spawn, no platform other than macOS and Linux, no HTTP adapter, no
pub/sub, no mailbox, no durable message queue, no heartbeat, no PID file, and no
shared cache across consumers.

## Sequencing

Each slice is one pull request that leaves the repository releasable.

1. **Activation proof.** A throwaway example daemon receives its socket on
   stdin from [launchd][launchd-plist] on macOS and from [systemd][systemd-socket] on Linux, serves, exits idle,
   and is relaunched by the next connection. This decides the activation path
   before any crate code depends on it.
2. **`unixd-cache`.** Extract the consumer's cache: policy types, key
   construction, locks, atomic write, the stale matrix, bounds, prune.
3. **`unixd` transport.** Extract framing, envelopes, the peer check, and the
   client, over a handler trait.
4. **`unixd` activation and lifecycle.** The stdin listener, unit install and
   uninstall on both platforms, drain, idle exit.
5. **Coalescing.** Single flight in the daemon, and the cross-mode
   equivalence test.
6. **First consumer adopts.** The consumer replaces its own daemon, transport,
   and cache with these crates, and deletes its self-spawn code.
7. **Secret store.** An in-memory store in the daemon with idle and maximum
   TTLs, zeroed on drop, and no core dumps. Nothing earlier depends on it.

## Verification

Each step names the check that proves it.

| # | Step | Proof |
|--:|:--|:--|
| 1 | Activation proof | on macOS and on Linux: the first connection starts the daemon, it serves, exits after the idle timeout, and the next connection starts it again with no connection refused |
| 2 | Scaffold | `cargo tree -p unixd-cache` contains no `tokio` |
| 3 | Framing | round-trip plus an oversize frame rejected without buffering to its end |
| 4 | Peer identity | real socket, same-UID accepted; a non-matching UID closed with no reply |
| 5 | Stale matrix | table test, four serve-stale cases and five never-stale cases, no case unlisted |
| 6 | Atomicity | interrupt between temp write and rename; the previous entry is still readable and valid |
| 7 | Working-directory regression | start the service, remove the requesting client's working directory, prove the next request succeeds |
| 8 | Prune | cross a hard cap, assert the low-water target is reached and no unexpired entry was lost |
| 9 | Cross-mode equivalence | the same request through the daemon client and the direct client yields identical normalized output and identical cache state |
| 10 | Install round-trip | install, connect, uninstall, and install again on both platforms; the second install leaves no duplicate unit |

Step 7 is the regression for the reported failure and must exist before either
crate is published.

## Open questions

1. Whether the envelope schema is hand-written in both Rust and any non-Rust
   client, proven by shared golden fixtures, or generated from one source. Start
   with fixtures; revisit only when drift is measured rather than predicted.

[example]: ../../crates/unixd/examples/activation.rs
[file-lock]: https://doc.rust-lang.org/std/fs/struct.File.html#method.lock
[http-cache-semantics]: https://crates.io/crates/http-cache-semantics
[launchd-plist]: https://keith.github.io/xcode-man-pages/launchd.plist.5.html
[listenfd]: https://crates.io/crates/listenfd
[maintenance]: ../research/cache-maintenance.md
[measurements]: ../research/activation-and-cache.md#4-live-activation-measurements
[research]: ../research/
[secret-store]: ../../openspec/changes/secret-store/proposal.md
[service-manager]: https://crates.io/crates/service-manager
[systemd-exec]: https://www.freedesktop.org/software/systemd/man/latest/systemd.exec.html
[systemd-socket]: https://www.freedesktop.org/software/systemd/man/latest/systemd.socket.html
[tempfile]: https://crates.io/crates/tempfile
[tokio]: https://tokio.rs
[tokio-ucred]: https://docs.rs/tokio/latest/tokio/net/struct.UnixStream.html#method.peer_cred
[coalescing]: ../../openspec/changes/coalescing/design.md
