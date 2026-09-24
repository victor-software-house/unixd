# Tasks

Compile, format, lint, and test tasks run on the build host with `mise run verify`.

## 1. Crate

- [x] 1.1 Key builder, policy, failure table, and error type; proof: the stale matrix and key tests pass
  - 2026-09-24 UTC: `only_transient_failures_serve_stale`, `duplicate_part_is_refused`, and `part_order_does_not_change_the_key` passed under `mise run verify`.
- [x] 1.2 Store, lookup, locks, atomic write, prune, clear, and usage; proof: the atomicity, prune, schema-bump, and root-safety tests pass
  - 2026-09-24 UTC: `mise run verify` on the build host passed; nextest ran 20 tests in [`tests/cache.rs`](../../../crates/unixd-cache/tests/cache.rs), all passed.
- [x] 1.3 README and crate docs name the stale matrix; proof: `mise run test:doc` passes
  - 2026-09-24 UTC: the crate doc test passed under `mise run verify`.

## 2. Design

- [x] 2.1 Design doc: the key digest covers the entry format, the caller schema lives in the entry; proof: the Keys section says so
  - 2026-09-24 UTC: [Keys](../../../docs/design/daemon-and-cache.md) states it, changed in the base commit of this stack.
- [x] 2.2 Verification steps 2, 5, 6, and 8 pass; proof: `mise run verify` on the build host
  - 2026-09-24 UTC: step 2 by `check:runtime-free`; step 5 by `only_transient_failures_serve_stale`; step 6 by `prune_removes_leftover_temporary_files`; step 8 by `prune_removes_expired_before_oldest`. The atomicity test places a leftover temporary file beside a stored entry; it does not kill a real writer mid-write.
