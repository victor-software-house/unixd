# unixd

Per-user daemon runtime: socket activation, listener ownership, peer identity,
bounded framing, and a single drain path shared by signals, the shutdown
request, and the idle timer.

Unimplemented. See
[`docs/design/daemon-and-cache.md`](../../docs/design/daemon-and-cache.md).

Pairs with [`unixd-cache`](../unixd-cache), which is usable on its own and
needs no async runtime.

MIT.
