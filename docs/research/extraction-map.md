# Research: what the first consumer has, and what unixd changes

- **Read:** 2026-09-24, the first consumer's daemon, protocol, and cache
  source and its tests. The consumer's repository is private, so this record
  names behaviour, not files.
- **Feeds:** the [design][design] and every open change under
  [`openspec/changes/`][changes].

## 1. What moves unchanged in behaviour

| Piece | Consumer behaviour | Slice |
|:--|:--|:--|
| Frame cap | 16 MiB including the newline; a read never buffers past the cap | [transport][transport] |
| Envelopes | a major and minor version, a request id, a client version of at most 256 bytes with no CR or LF | [transport][transport] |
| Peer check | `peer_cred` before any read; a mismatch closes with no reply and no log | [transport][transport] |
| Client | one deadline over connect, peer check, write, half-close, and read; the reply's request id is checked | [transport][transport] |
| Drain | one path for `SIGINT`, `SIGTERM`, a shutdown request, and idle; the shutdown reply is written before the drain starts | [lifecycle][lifecycle] |
| Idle timer | runs only while no handler is running, and restarts after the last one ends | [lifecycle][lifecycle] |
| Stale matrix | stale only for timeout, network failure, 429, and 5xx | [disk cache][disk-cache] |
| Locks | a shared maintenance lock plus an exclusive per-key lock, both blocking, opened with `O_NOFOLLOW` at 0600 | [disk cache][disk-cache] |
| Schema bump | an older-schema entry is served only as stale | [disk cache][disk-cache] |

## 2. Where the consumer and the design disagree

The design wins in every row. The consumer changes when it adopts the crates.

| # | Consumer | unixd | Slice |
|--:|:--|:--|:--|
| 1 | spawns its own daemon behind a start lock, keeps a PID file and a lifetime lock, removes stale sockets, binds, and unlinks on exit | the service manager owns the socket and passes it on stdin; no PID file, no unlink | [lifecycle][lifecycle] |
| 2 | the spawned daemon inherits the client's working directory; cache roots from the environment stay relative | the serve entry changes to `/`; roots are made absolute once | [lifecycle][lifecycle], [disk cache][disk-cache] |
| 3 | an oversize or malformed request gets an error reply, then the connection closes | the connection closes with no reply | [transport][transport] |
| 4 | no per-connection read timeout, so a silent client holds a handler until drain | a per-connection read deadline | [transport][transport] |
| 5 | keys are a closed enum of request kinds | caller-built keys from named parts | [disk cache][disk-cache] |
| 6 | one lookup and one store method per request kind | one generic serializable value | [disk cache][disk-cache] |
| 7 | locks through `fs2`; a hand-written temporary file and rename | `std::fs::File::lock` and `tempfile` | [disk cache][disk-cache] |
| 8 | capacity eviction by file mtime; leftover temporary files are never removed | eviction by stored time; prune removes leftovers | [disk cache][disk-cache] |
| 9 | coalescing runs in both modes and is typed per request kind | the daemon coalesces; the direct path does not | [coalescing][coalescing] |
| 10 | the protocol error to domain error mapping is a fixed table in the protocol crate | the caller owns the error codes | [transport][transport] |
| 11 | the direct path runs a Tokio runtime | the cache needs no runtime | [disk cache][disk-cache] |

## 3. Failures the design removes

Two of the consumer's start-lock tests fail intermittently. Both run alongside
tests that spawn real daemons.

1. A helper that exits at once must report "unavailable" within 300 ms. Under
   load, the shell has not exited by then, and the result is a timeout.
2. A spawner with a 300 ms budget and a waiter share one start lock. When the
   helper is slow, the spawner times out and releases the lock, and the waiter
   spawns a second helper. The lock admitted two spawners.

Socket activation has no start lock and no spawner, so neither case exists.

## 4. Left to the consumer

1. **Enrichment failures.** One request kind returns an enrichment failure
   without serving stale, even on a timeout. That is the consumer's policy, and
   the adoption slice must decide it explicitly.
2. **Its HTTP server.** It shares the dispatcher with the Unix server. unixd
   has no HTTP adapter.

[changes]: ../../openspec/changes
[coalescing]: ../../openspec/changes/archive/2026-09-24-coalescing/proposal.md
[design]: ../design/daemon-and-cache.md
[disk-cache]: ../../openspec/changes/archive/2026-09-24-disk-cache/proposal.md
[lifecycle]: ../../openspec/changes/archive/2026-09-24-activation-lifecycle/proposal.md
[transport]: ../../openspec/changes/archive/2026-09-24-transport/proposal.md
