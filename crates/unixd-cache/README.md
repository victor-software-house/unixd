# unixd-cache

Bounded on-disk response cache. Two independent freshness horizons, an
explicit stale-if-error matrix, one file lock per key held across processes,
atomic write through temp-file and rename, and prune to a low-water target.

A stale entry answers only a transient upstream failure: a timeout, a network
failure, HTTP 429, or HTTP 5xx. It never answers 401, 403, 404, invalid JSON,
or a schema mismatch.

A key is a namespace plus named parts, sorted and hashed, so part order does
not matter.

Synchronous. No async runtime, no database. The same cache is correct whether
a daemon or a direct in-process call owns it.

Design: [`docs/design/daemon-and-cache.md`](../../docs/design/daemon-and-cache.md).
Requirements: [`openspec/changes/disk-cache`](../../openspec/changes/disk-cache).

MIT.
