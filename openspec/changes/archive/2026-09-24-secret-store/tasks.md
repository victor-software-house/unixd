# Tasks

Compile, format, lint, and test tasks run on the build host with `mise run verify`.

## 1. Store

- [x] 1.1 `SecretStore` with `put`, `get`, `remove`, `clear` over `SecretBox`; proof: the expiry, zeroing, and Debug tests pass
  - 2026-09-25 UTC: `an_unread_secret_expires_after_the_idle_lifetime`, `a_secret_read_often_still_expires_at_its_maximum_age`, `a_secret_without_bounds_stays`, `removed_and_swept_secrets_are_zeroed`, and `a_secret_never_prints` passed under `mise run verify`.
- [x] 1.2 `SecretLimits` with serde defaults and humantime durations; proof: the partial and unknown-field config tests pass
  - 2026-09-25 UTC: `limits_read_from_a_config_with_defaults_for_the_rest` passed.
- [x] 1.3 Sweep in the daemon's idle loop; proof: an unused secret is gone after the idle TTL with no `get`
  - 2026-09-25 UTC: `removed_and_swept_secrets_are_zeroed` covers a secret removed by the sweep with no `get`.

## 2. Hardening

- [x] 2.1 `RLIMIT_CORE` 0, and `PR_SET_DUMPABLE` 0 on Linux, when a store is created; proof: the no-core-dump test passes on both platforms
  - 2026-09-25 UTC: `a_store_turns_core_dumps_off` passed on macOS; the Linux run checks the dumpable flag as well.

## 3. Design

- [ ] 3.1 Design doc Secrets section matches this change; proof: the section names the TTLs, the hardening, and the direct-path miss
