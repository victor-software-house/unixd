# Design

## Decisions

| # | Decision | Alternative | Why the alternative lost |
|--:|:--|:--|:--|
| 1 | One generic `lookup::<T>` and `store::<T>` over caller keys. | One method pair per request kind, as the consumer has. | A per-kind enum ties the crate to one program. |
| 2 | Keys are named parts, sorted, length-prefixed, and hashed with SHA-256 and the crate's entry format version. | Hash a caller-serialized value. | Named parts let the crate reject a credential or output format by name, and sorting makes part order irrelevant. |
| 3 | The caller's payload schema is stored in the entry, not hashed into the key. | Hash the schema into the key, as the design first said. | A hashed schema makes old entries unreachable, which contradicts serving them as stale after a bump. |
| 4 | `std::fs::File::lock` for the per-key and maintenance locks. | [`fs2`][fs2], as the consumer uses. | The standard library has it since 1.89, so the dependency goes away. |
| 5 | [`tempfile`][tempfile] `persist`, then sync the parent directory. | The consumer's own temporary-name counter. | `tempfile` picks a unique name and cleans up on failure. |
| 6 | Prune removes leftover temporary files. | Leave them. | Writers hold the shared maintenance lock while they write, and prune holds it exclusively, so any temporary file prune sees belongs to a writer that died. |
| 7 | `Failure::serves_stale` is a closed table. | A caller predicate. | The matrix is the contract; a predicate lets each consumer drift. |

## Overrides and disable paths

| Automatic behaviour | Override | Disable path |
|:--|:--|:--|
| Prune after a write crosses a hard cap | `Limits` | caps of `u64::MAX` |
| Removing a corrupt entry on read | none | not disableable; the entry is invalid |
| Serving an older-schema entry as stale | the caller's schema number | the caller clears the cache |

## Risks

1. Name-based key rejection catches only named mistakes. A caller that puts a
   credential under an innocent name is not caught. The crate documents the
   rule on `KeyBuilder::part`.

[fs2]: https://crates.io/crates/fs2
[tempfile]: https://crates.io/crates/tempfile
