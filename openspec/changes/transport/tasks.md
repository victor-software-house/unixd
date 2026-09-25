# Tasks

Compile, format, lint, and test tasks run on the build host with `mise run verify`.

## 1. Crate

- [x] 1.1 Frame reader and writer over `LinesCodec` with a cap; proof: the round-trip and oversize tests pass
  - 2026-09-24 UTC: `a_request_round_trips`, `an_oversize_frame_is_never_read_past_the_cap` (reads at most the 1 KiB cap of 64 KiB), `a_frame_of_exactly_the_cap_is_read`, and `an_oversize_frame_closes_the_connection_without_a_reply` passed under `mise run verify`.
- [x] 1.2 Envelopes, version check, and the caller error trait; proof: the unsupported-major and handler-error tests pass
  - 2026-09-24 UTC: `another_major_version_is_refused` and `a_handler_error_reaches_the_client_with_its_code` passed.
- [x] 1.3 Server connection handler with the peer check and deadlines over the `Handler` trait; proof: the same-user, silent-client, and different-user tests pass
  - 2026-09-24 UTC: `a_request_round_trips` (same user), `a_silent_client_is_dropped_after_the_read_timeout`, and `another_users_connection_closes_before_any_read` passed. The different-user case runs the server with another uid over a socket pair, since a test cannot connect as another user.
- [x] 1.4 Blocking client; proof: the no-daemon and wrong-request-id tests pass
  - 2026-09-24 UTC: `no_daemon_is_unavailable_at_once` and `a_reply_to_another_request_is_invalid` passed.

## 2. Design

- [x] 2.1 Verification steps 3 and 4 pass; proof: `mise run verify` on the build host
  - 2026-09-24 UTC: `mise run verify` passed on the build host in `dev` and `dev,ci`: 45 tests. Step 3 by the round-trip and cap tests, step 4 by the same-user and different-user tests.
