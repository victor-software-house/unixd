# unixd-cache

Bounded on-disk response cache. Two independent freshness horizons, an
explicit stale-if-error matrix, one file lock per key held across processes,
atomic write through temp-file and rename, and prune to a low-water target.

Synchronous. No async runtime, no database. The same cache is correct whether
a daemon or a direct in-process call owns it.

Unimplemented. See
[`docs/design/daemon-and-cache.md`](../../docs/design/daemon-and-cache.md).

MIT.
