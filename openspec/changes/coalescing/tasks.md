# Tasks

Compile, format, lint, and test tasks run on the build host with `mise run verify`.

## 1. Crate

- [x] 1.1 `SingleFlight` on its own tasks; proof: the identical-calls, distinct-keys, shared-failure, and leader-dropped tests pass
  - 2026-09-25 UTC: `ten_identical_calls_share_one_run`, `distinct_keys_run_at_the_same_time`, `every_caller_gets_the_shared_failure_and_nothing_is_kept`, and `a_dropped_leader_does_not_cancel_the_work` passed under `mise run verify`.
- [x] 1.2 Cross-mode equivalence test; proof: it passes
  - 2026-09-25 UTC: `the_daemon_and_the_direct_path_agree` passed: equal output, and equal value, horizons, schema, and source in the two cache entries.

## 2. Design

- [x] 2.1 Design doc: coalescing is in the daemon only, with no trait; proof: the Atomicity section says so
  - 2026-09-24 UTC: changed in the base commit of this stack, with the Sequencing entry.
- [x] 2.2 Verification step 9 passes; proof: `mise run verify` on the build host
  - 2026-09-25 UTC: `mise run verify` passed on the build host: 56 tests.
