# Tasks

Compile, format, lint, and test tasks run on the build host with `mise run verify`.

## 1. Store

- [ ] 1.1 `SecretStore` with `put`, `get`, `remove`, `clear` over `SecretBox`; proof: the expiry, zeroing, and Debug tests pass
- [ ] 1.2 `SecretLimits` with serde defaults and humantime durations; proof: the partial and unknown-field config tests pass
- [ ] 1.3 Sweep in the daemon's idle loop; proof: an unused secret is gone after the idle TTL with no `get`

## 2. Hardening

- [ ] 2.1 `RLIMIT_CORE` 0, `PR_SET_DUMPABLE` 0 on Linux, `PT_DENY_ATTACH` on macOS when a store registers; proof: the no-core-dump test passes on both platforms in CI
- [ ] 2.2 Best-effort `mlock` with one warning on failure; proof: a test with `RLIMIT_MEMLOCK` 0 logs the warning and the store still works

## 3. Design

- [ ] 3.1 Design doc Secrets section matches this change; proof: the section names the TTLs, the hardening, and the direct-path miss
