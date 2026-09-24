# Design

The options are compared in the [research][research].

## Decisions

| # | Decision | Alternative | Why the alternative lost |
|--:|:--|:--|:--|
| 1 | Our own map from key to a shared task handle, about 40 lines. | [`async_singleflight`][asf], [`moka`][moka] `try_get_with`. | `async_singleflight` gives the error only to the leader; `moka` caches successes and needs invalidation for pure coalescing. |
| 2 | Run the work as a task on the `TaskTracker`. | Run it in the leader's future. | A leader that disconnects would cancel the work for every waiter. |
| 3 | Share errors as `Arc<E>`. | Give followers a generic failure. | Every waiter must see the real error, so the stale-if-error table applies to each. |
| 4 | No coalescing trait; the direct path calls the work. | A trait with a no-op binding, as the design first said. | The direct path is synchronous and the daemon is async, so one trait would fit neither, and the no-op adds no behaviour. The cache key lock already serializes callers across processes. |
| 5 | The map entry is removed when the work ends. | Keep finished results for a short time. | A finished result belongs in the disk cache, which stays canonical. |

## Overrides and disable paths

| Automatic behaviour | Override | Disable path |
|:--|:--|:--|
| Coalescing in the daemon | the handler chooses which calls go through `SingleFlight` | call the work directly |

## Risks

1. A panic in shared work reaches every waiter. The task boundary turns it
   into a `JoinError`, which each waiter receives as an error.

[asf]: https://crates.io/crates/async_singleflight
[moka]: https://crates.io/crates/moka
[research]: ../../../docs/research/transport-and-lifecycle.md#5-single-flight
