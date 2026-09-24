# Tasks

Compile, format, lint, and test tasks run on the build host with `mise run verify`.

## 1. Design

- [x] 1.1 Revise `docs/design/daemon-and-cache.md` for native activation on macOS and Linux, extraction from the first consumer, and the cache stack; proof: `openspec validate native-socket-activation` passes
- [x] 1.2 Update the README scope and AGENTS.md rule 3; proof: neither names self-spawn as a future option

## 2. Activation proof

- [x] 2.1 Add a throwaway `examples/activation.rs` that adopts the stdin listener, answers one line per connection, and exits after an idle timeout; proof: `mise run verify` passes
- [x] 2.2 Run it under launchd on macOS: first connection, idle exit, relaunch; proof: the three scenarios in `specs/activation` hold, with the timing recorded
- [x] 2.3 Run it under `systemd --user` on Linux: the same three scenarios; proof: the same, with the timing recorded
- [x] 2.4 Record the result and any fallback decision in the design doc; proof: its open question 2 is answered
