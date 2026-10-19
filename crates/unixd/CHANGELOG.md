# Changelog

## unixd 0.0.5

- Releases run through verctl's reusable release workflow at 0.5.1: each push to `main` publishes any version commit not yet fully released, and opens or updates the Version PR on `verctl/release/main`. `verify` gains ctl-core 0.6.27's `check:versions`, which fails a pull request that edits a manifest version by hand ([#41](https://github.com/victor-software-house/unixd/pull/41)).

## unixd 0.0.4

- Raise `rust-version` to 1.99.0. Both crates now build, lint, and test on the pinned stable 1.99.0 instead of a dated nightly, and the declared minimum equals it. A consumer on an older compiler stays on the previous release.

## unixd 0.0.3

- The client searches only newly read bytes for the end of a reply, so a reply near the frame cap no longer takes seconds to read.

## unixd 0.0.2

- `Fault` carries optional `details`, which a handler error supplies through `Failure::details`, and `unsupported_version` reports the requested and supported majors there.

## unixd 0.0.1

- First release: a socket-activated per-user daemon runtime with unit install for launchd and systemd, a blocking client, single flight, and an in-memory secret store.

