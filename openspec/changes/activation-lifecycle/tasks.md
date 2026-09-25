# Tasks

Compile, format, lint, and test tasks run on the build host with `mise run verify`.
Install tests run on macOS and on Linux.

## 1. Crate

- [x] 1.1 Serve entry: `chdir("/")`, stdin listener, accept loop; proof: the client-directory and idle-exit scenarios pass under both managers
  - 2026-09-25 UTC: `the_socket_outlives_an_idle_exit_and_a_removed_client_directory` passed under launchd on macOS (3 runs) and under `systemd --user` on Linux (2 runs), with the echo example as the daemon.
- [x] 1.2 Drain with signals, shutdown request, and idle timer; proof: the signal and idle-timer tests pass
  - 2026-09-25 UTC: `a_signal_during_a_request_still_delivers_its_response`, `a_shutdown_request_is_answered_before_the_drain`, `the_idle_timer_waits_for_open_requests`, and `a_drain_cuts_connections_after_its_timeout` passed on macOS under nextest and on Linux with `--test-threads=1`.
- [x] 1.3 `install` and `uninstall` for launchd and systemd with explicit restart limits; proof: the round-trip and path-length tests pass on both platforms
  - 2026-09-25 UTC: `install_twice_serve_uninstall_and_install_again` passed on both platforms and left no unit file; `a_socket_path_over_the_limit_is_refused_before_any_write` passed in `mise run verify`.

## 2. Design

- [x] 2.1 Verification steps 7 and 10 pass; proof: the install round trip on macOS and Linux
  - 2026-09-25 UTC: step 7 by the removed-directory test, step 10 by the install round trip, on macOS and on Linux. The install tests are ignored by default and were run by hand, as the test file describes.
- [x] 2.2 Remove the throwaway activation example; proof: the crate tests cover what it proved
  - 2026-09-25 UTC: `examples/activation.rs` is replaced by `examples/echo.rs`, which the install tests run.
