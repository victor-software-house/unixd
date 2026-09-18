# Design: a socket-activated per-user daemon, and a cache in front of it

- **Status:** Proposed
- **Scope:** both crates in this workspace

## What this decides

Two crates, separately selectable:

| Crate | Owns | Async runtime |
|:--|:--|:--|
| `unixd` | Activation, listener ownership, peer identity, bounded framing, lifecycle, client transport | Tokio |
| `unixd-cache` | Freshness, stale-if-error policy, cross-process locks, atomic writes, bounds, prune | None |

## Why two crates rather than one

The two known consumers need different halves.

1. A local message broker needs the runtime shell and is explicitly forbidden
   from holding a refreshed cache of anything. It takes `unixd` alone.
2. A CLI with an HTTP upstream and a disk cache needs both, and needs the cache
   to work identically in its direct no-daemon mode, which has no async runtime
   at all. It takes both, and `unixd-cache` must not require a reactor.

One crate containing both would force a Tokio dependency on the direct path of
consumer 2 and a cache module on consumer 1. Neither is acceptable, so the
split is a requirement.

## Optional presentation seam

Neither crate renders anything. A consumer that wants a daemon status or cache
statistics to look native in its own output can depend on a semantic document
layer and have these crates emit documents rather than strings. That dependency
must stay optional and must not pull a terminal renderer, a serializer, or an
argument parser into either crate.

## Layer A — `unixd`

### Activation is a trait with one implementation

```rust
pub trait Activation {
    /// Yield the listening socket this process should serve.
    fn listener(&self) -> Result<ServingListener, ActivationError>;
}
```

Version one ships **`InheritedListener` only**: the socket is created and owned
by the platform service manager and handed to the process as an inherited file
descriptor. On macOS that is `launchd` socket activation.

The trait exists so a self-spawning implementation and a systemd `LISTEN_FDS`
implementation can be added without touching any call site. They are **not** in
version one. Nothing in the crate branches on which implementation is active.

### Why activation, not self-spawn

Self-spawn requires a start lock, a readiness poll, stale-socket adoption under
a second lock, and convergence tests for concurrent first clients. Activation
deletes all four. It also removes the failure that motivated this work:

> A self-spawned daemon inherits the launching client's working directory. When
> that directory is later removed, the daemon is stranded and every subsequent
> request fails.

Activation cannot reproduce it, because no client process is the parent. Two
guards make the property explicit rather than incidental:

1. the serve entry point changes its working directory to `/` before binding;
2. runtime and cache roots are resolved to absolute paths once at startup and
   never re-resolved against a relative base.

Both are asserted by the regression in verification step 7.

### Installation contract

- one per-user socket, `SockPathMode = 0600`, in a `0700` directory;
- `KeepAlive = false`, so the service starts on first connection and exits when
  idle;
- service label and socket name carry the protocol major version;
- install and uninstall are idempotent and remove only state this identity owns.

### Peer identity

Every accepted connection is checked for the same effective UID as the serving
process, using the platform peer-credential call, before any frame is read. A
mismatch closes the connection without a reply and without a log entry
containing the peer's identity.

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
4. remove socket and metadata **only** when owned by the exiting process.

Idle timeout is configurable and disableable. A PID file, where one exists, is
informational and is never the liveness authority.

## Layer B — `unixd-cache`

Synchronous, no reactor, no database. Versioned validated JSON envelopes on
disk; directories `0700`, files `0600`.

### Freshness has two independent horizons

Each entry stores `fresh_until` and `stale_until` as separate absolute
timestamps, plus the provenance of the policy that set them. Policy comes either
from a caller-supplied default or from a response-derived directive such as
`Cache-Control`; provenance records which, so an entry written under one regime
is not silently reinterpreted under another.

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

The caller supplies key inputs; the crate hashes them with the schema version.
The crate rejects a key input set at construction if it would embed a
credential, a credential hash, an output format, a destination path, or a
request id. Cache identity is about *what was fetched*, never *who asked* or
*how it will be rendered*.

### Atomicity, locking, and single flight

- validate on every read; a corrupt or schema-invalid entry is a miss and is
  removed best-effort;
- one file lock per key, held across direct and daemon processes alike;
- one maintenance lock for prune and clear;
- write to a temporary file in the same directory, `fsync` the file, rename
  atomically, then `fsync` the parent directory;
- coalescing is a trait the daemon layer implements in process; the direct path
  binds a no-op implementation.

Disk stays canonical. There is no daemon-only in-memory copy that can diverge
from it.

### Bounds

- an entry above the per-entry ceiling is returned to the caller but not stored;
- hard caps on both entry count and total bytes;
- prune removes expired entries first, then the oldest, down to a low-water
  target below the cap;
- prune runs at service startup and after a write crosses a hard cap.

## Non-goals

No self-spawn fallback, no systemd activation, no Linux support, no HTTP
adapter, no pub/sub, no mailbox, no durable message queue, no heartbeat, and no
shared cache across consumers. The activation trait leaves room for the first
two; nothing else here anticipates a consumer that does not exist.

## Sequencing

Each slice is one pull request that leaves the repository releasable.

1. **Scaffold.** Workspace, two member crates, lint tables, CI verify lane. No
   behaviour.
2. **`unixd-cache`.** Policy types, key construction, locks, atomic write, the
   stale matrix, bounds, prune. Fully testable without a socket.
3. **`unixd` transport.** Framing, envelopes, peer identity, client.
4. **`unixd` activation and lifecycle.** `InheritedListener`, install and
   uninstall, drain, idle exit.
5. **Coalescing seam.** The trait, the in-process daemon implementation, the
   direct no-op, and the cross-mode equivalence test.

## Verification

Each step names the check that proves it.

| # | Step | Proof |
|--:|:--|:--|
| 1 | Scaffold | `cargo metadata` lists both members; `cargo tree -p unixd-cache` contains no `tokio` |
| 2 | Framing | round-trip plus an oversize frame rejected without buffering to its end |
| 3 | Peer identity | real socket, same-UID accepted; a non-matching UID closed with no reply |
| 4 | Activation | real service bootstrap in a scratch domain; two concurrent first clients converge on one process |
| 5 | Stale matrix | table test, four serve-stale cases and five never-stale cases, no case unlisted |
| 6 | Atomicity | interrupt between temp write and rename; the previous entry is still readable and valid |
| 7 | Working-directory regression | start the service, remove the requesting client's working directory, prove the next request succeeds |
| 8 | Prune | cross a hard cap, assert the low-water target is reached and no unexpired entry was lost |
| 9 | Cross-mode equivalence | the same request through the daemon client and the direct client yields identical normalized output and identical cache state |

Step 7 is the regression for the reported failure and must exist before either
crate is published.

## Open questions

1. Whether `unixd-cache` should parse HTTP response cache directives itself, or
   take an already-parsed directive from the caller. Taking the parsed directive
   keeps the crate transport-free; parsing would pull an HTTP dependency into a
   crate that otherwise has none.
2. Whether the envelope schema is hand-written in both Rust and any non-Rust
   client, proven by shared golden fixtures, or generated from one source. Start
   with fixtures; revisit only when drift is measured rather than predicted.
