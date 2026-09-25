# Tasks

Compile, format, lint, and test tasks run on the build host with `mise run verify`.
Install tests run on macOS and on Linux.

## 1. Crate

- [ ] 1.1 Serve entry: `chdir("/")`, stdin listener, accept loop; proof: the client-directory and idle-exit scenarios pass under both managers
- [ ] 1.2 Drain with signals, shutdown request, and idle timer; proof: the signal and idle-timer tests pass
- [ ] 1.3 `install` and `uninstall` for launchd and systemd with explicit restart limits; proof: the round-trip and path-length tests pass on both platforms

## 2. Design

- [ ] 2.1 Verification steps 7 and 10 pass; proof: the install round trip on macOS and Linux
- [ ] 2.2 Remove the throwaway activation example; proof: the crate tests cover what it proved
