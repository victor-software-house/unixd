# Design

## Decisions

| # | Decision | Alternative | Why the alternative lost |
|--:|:--|:--|:--|
| 1 | One generic `lookup::<T>` and `store::<T>` over caller keys. | One method pair per request kind, as the consumer has. | A per-kind enum ties the crate to one program. |
| 2 | Keys are named parts, sorted, length-prefixed, and hashed with SHA-256 and the crate's entry format version. | Hash a caller-serialized value. | Sorted named parts make part order irrelevant, and a length prefix keeps `("ab", "c")` distinct from `("a", "bc")`. |
| 3 | The caller's payload schema is stored in the entry, not hashed into the key. | Hash the schema into the key, as the design first said. | A hashed schema makes old entries unreachable, which contradicts serving them as stale after a bump. |
| 4 | `std::fs::File::lock` for the per-key and maintenance locks. | [`fs2`][fs2], as the consumer uses. | The standard library has it since 1.89, so the dependency goes away. |
| 5 | [`tempfile`][tempfile] `persist`, then sync the parent directory. | The consumer's own temporary-name counter. | `tempfile` picks a unique name and cleans up on failure. |
| 6 | Prune holds the maintenance lock shared plus its own `prune.lock`, and removes each file under that key's lock, taken without waiting; a held key is skipped. | Prune holds the maintenance lock exclusively. | Every held key lock holds the maintenance lock shared, so a daemon whose keys are always locked by someone never pruned and grew past its caps. Only `clear` needs the exclusive lock. |
| 7 | `Failure::serves_stale` is a closed table. | A caller predicate. | The matrix is the contract; a predicate lets each consumer drift. |
| 8 | A key's lock file is deleted by its holder before unlocking; a waiter that locked the deleted file compares device and inode with the path and retries. | Keep lock files and sweep them in prune. | Keys that never store, such as failed fetches, would pile up lock files between prunes. No crate implements this; see the [research][research]. |
| 9 | Capacity eviction removes the least recently used entry first, by file modification time; a hit refreshes it at most once per `touch_after`. | Evict by store time. | A hot old entry was evicted before a cold new one. sccache, ccache, and Go's build cache evict by last use. |
| 10 | A write prunes when the last prune is older than `sweep_every`, and prune removes entries unused for `unused_after`. | Rely on the caps alone. | Neither macOS nor Linux cleans a plain program's cache directory, so an unexpired entry nobody reads would stay until a cap forced it out. |
| 11 | Entry and `prune.lock` modification times come from the cache's `Clock`. | The filesystem's own time. | Tests move time through the clock; mixing two clocks would make eviction order untestable. |
| 12 | Prune reads each entry only up to its `value` key. | Parse the whole entry. | `Written` serializes `value` last, and a quote inside a string is escaped, so the first `,"value":` ends the header. A full read costs up to the byte cap per prune. A corrupt value is left for a locked lookup to remove. |
| 13 | `Limits` deserializes with every field optional, durations as `30d` or `1h` through [`humantime-serde`][humantime-serde], and unknown fields refused. The crate reads no config file. | A crate-owned config file. | The consumer owns its config; it embeds a `cache` section and sets only what it needs. A misspelled field is an error instead of a silent default. |
| 14 | `default_root(name)` returns `~/Library/Caches/<name>` on macOS and `$XDG_CACHE_HOME/<name>` or `~/.cache/<name>` elsewhere, and `open` writes `CACHEDIR.TAG`. | Leave the root to each consumer. | Every consumer would pick its own place. Go, sccache, and ccache use these same places, and backup tools skip a tagged directory. |

## Overrides and disable paths

| Automatic behaviour | Override | Disable path |
|:--|:--|:--|
| Prune after a write crosses a hard cap | `Limits` caps and targets | caps of `u64::MAX` |
| Daily sweep of unused entries | `Limits::sweep_every` and `Limits::unused_after` | `Duration::MAX` for either |
| Refreshing an entry's last use on a hit | `Limits::touch_after` | `Duration::MAX` |
| Removing a corrupt entry on a locked read or in prune | none | not disableable; the entry is invalid. An unlocked read never removes anything. |
| Serving an older-schema entry as stale | the caller's schema number | the caller clears the cache |

[fs2]: https://crates.io/crates/fs2
[humantime-serde]: https://crates.io/crates/humantime-serde
[research]: ../../../docs/research/cache-maintenance.md
[tempfile]: https://crates.io/crates/tempfile
