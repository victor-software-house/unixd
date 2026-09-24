# Tasks

Compile, format, lint, and test tasks run on the build host with `mise run verify`.

## 1. Crate

- [ ] 1.1 `SingleFlight` on the `TaskTracker`; proof: the identical-calls, distinct-keys, shared-failure, and leader-dropped tests pass
- [ ] 1.2 Cross-mode equivalence test; proof: it passes

## 2. Design

- [x] 2.1 Design doc: coalescing is in the daemon only, with no trait; proof: the Atomicity section says so
  - 2026-09-24 UTC: changed in the base commit of this stack, with the Sequencing entry.
- [ ] 2.2 Verification step 9 passes; proof: `mise run verify` on the build host
