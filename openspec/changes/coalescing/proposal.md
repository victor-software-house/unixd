# Proposal

## Why

UXD-005. The first consumer coalesces concurrent identical requests, so one
upstream call serves them all. Its code does this six times, once per request
kind, with near-identical copies, and it runs in the direct path as well as in
the daemon ([extraction map][map]). The design asks for coalescing in the
daemon only, and for proof that the daemon and the direct path give the same
result and leave the same cache.

## What Changes

1. `unixd` provides `SingleFlight<K, T, E>`: concurrent calls with the same key
   share one run of the work, and every caller gets the value or the shared
   error.
2. The work runs as its own task, so a caller that disconnects does not
   cancel it for the others.
3. The direct path calls the work without coalescing. Across processes, the
   `unixd-cache` key lock already makes a second caller wait and then read the
   stored entry.
4. The design doc drops the coalescing trait with a no-op binding.
5. A test sends the same request through the daemon client and the direct
   path and compares the output and the cache files.

## Capabilities

### New Capabilities

- `coalescing`: single flight in the daemon, and the cross-mode equivalence
  test.

### Modified Capabilities

None.

## Impact

1. New dependency in `unixd`: [`futures-util`][futures-util] for
   `future::Shared`.
2. The equivalence test uses `unixd-cache` as a dev-dependency of `unixd`.

[futures-util]: https://crates.io/crates/futures-util
[map]: ../../../docs/research/extraction-map.md
