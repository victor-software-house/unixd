# Proposal

## Why

The design chose launchd socket activation and put systemd, Linux, and
self-spawn out of scope. Three facts change that:

1. The daemon must run on Linux as well as macOS on Apple Silicon.
2. The first consumer already runs a working daemon and cache, so unixd should
   extract that code rather than write it fresh.
3. That consumer starts its daemon by self-spawn behind a start lock. Two of
   its twelve start-lock tests fail intermittently, and in one run the lock let
   two daemons spawn. Its spawned child never changes directory, so the
   stranded working-directory failure this design exists to prevent is live
   there. Another self-spawning tool shipped the same failure and fixed it
   later.

A research pass on 2026-09-24 found that no maintained crate hands a macOS
caller an owned listener from `launch_activate_socket` without `unsafe`, which
this workspace forbids. Both managers can instead pass the listening socket on
standard input, which safe `std` code can adopt. That path is documented but
has not been run here.

## What Changes

1. Revise `docs/design/daemon-and-cache.md`: native activation on both
   platforms, the socket received on standard input, no self-spawn, units
   written by unixd, lifecycle rules for a manager-owned socket, the cache
   built on `std::fs::File::lock` and `tempfile`, `Cache-Control` behind an
   optional feature, and extraction from the first consumer.
2. Update the README scope and AGENTS.md rule 3 to match.
3. Prove the activation path live on macOS and Linux with a throwaway example
   before any crate code depends on it (slice 1).

## Capabilities

### New Capabilities

- `activation`: how the daemon is started and receives its socket.

### Modified Capabilities

None. No spec exists yet.

## Impact

1. No crate code changes in this change beyond a throwaway example.
2. The sequencing gains a first slice (the activation proof) and a last one
   (the consumer adopts the crates).
