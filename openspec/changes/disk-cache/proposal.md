# Proposal

## Why

UXD-002. The first consumer keeps a working disk cache: fresh and stale
horizons, per-key locks, atomic writes, and bounded prune, covered by its own
tests. The code names each cached request kind in one enum, with a lookup and
a store method per kind, so no other program can use it. Its locks come from
[`fs2`][fs2] and its owner checks from [`rustix`][rustix]; the standard library
has covered file locks since 1.89.

[Research][research] found no cache crate with stale-if-error, cross-process
locks, and size bounds together, so the cache stays hand-written.

## What Changes

1. `unixd-cache` stores any serializable value under a caller-built key, with a
   caller policy, and no async runtime.
2. Keys are built from named parts and hashed. A part named like a credential,
   an output format, a destination path, or a request id is rejected.
3. The stale-if-error decision is a table: timeout, network failure, 429, and
   5xx serve stale; 401, 403, 404, invalid JSON, and a schema mismatch do not.
4. Locks use `std::fs::File::lock`; writes use [`tempfile`][tempfile] `persist`
   and sync the file and its directory.
5. The caller's payload schema is stored in each entry. An entry from an older
   schema is served only as stale.
6. The design doc states that the key digest covers the crate's entry format,
   not the caller's schema, so a schema bump keeps its old entries reachable.

## Capabilities

### New Capabilities

- `cache`: storing, reading, and pruning entries.

### Modified Capabilities

None.

## Impact

1. New dependencies in `unixd-cache`: [`serde`][serde], [`serde_json`][serde-json],
   [`sha2`][sha2], [`tempfile`][tempfile], and [`rustix`][rustix] for the
   effective uid and no-follow opens.
2. The optional `http` feature with [`http-cache-semantics`][http-cache-semantics]
   is not in this change. It lands when a consumer needs `Cache-Control`.

[fs2]: https://crates.io/crates/fs2
[http-cache-semantics]: https://crates.io/crates/http-cache-semantics
[research]: ../../../docs/research/activation-and-cache.md#7-the-disk-cache
[rustix]: https://crates.io/crates/rustix
[serde]: https://crates.io/crates/serde
[serde-json]: https://crates.io/crates/serde_json
[sha2]: https://crates.io/crates/sha2
[tempfile]: https://crates.io/crates/tempfile
