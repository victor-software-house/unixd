# Tasks

Compile, format, lint, and test tasks run on the build host with `mise run verify`.

## 1. Crate

- [ ] 1.1 Key builder, policy, failure table, and error type; proof: the stale matrix and forbidden-part tests pass
- [ ] 1.2 Store, lookup, locks, atomic write, prune, clear, and usage; proof: the atomicity, prune, schema-bump, and root-safety tests pass
- [ ] 1.3 README and crate docs name the stale matrix and the key rule; proof: `mise run test:doc` passes

## 2. Design

- [ ] 2.1 Design doc: the key digest covers the entry format, the caller schema lives in the entry; proof: the Keys section says so
- [ ] 2.2 Verification steps 2, 5, 6, and 8 pass; proof: `mise run verify` on the build host
