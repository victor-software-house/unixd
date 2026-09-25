# Tasks

Compile, format, lint, and test tasks run on the build host with `mise run verify`.

## 1. Crate

- [ ] 1.1 Frame reader and writer over `LinesCodec` with a cap; proof: the round-trip and oversize tests pass
- [ ] 1.2 Envelopes, version check, and the caller error trait; proof: the unsupported-major and handler-error tests pass
- [ ] 1.3 Server connection handler with the peer check and deadlines over the `Handler` trait; proof: the same-user, silent-client, and different-user tests pass
- [ ] 1.4 Blocking client; proof: the no-daemon and wrong-request-id tests pass

## 2. Design

- [ ] 2.1 Verification steps 3 and 4 pass; proof: `mise run verify` on the build host
