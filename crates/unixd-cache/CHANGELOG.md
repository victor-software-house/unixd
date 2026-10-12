# Changelog

## unixd-cache 0.0.2

- Raise `rust-version` to 1.99.0. Both crates now build, lint, and test on the pinned stable 1.99.0 instead of a dated nightly, and the declared minimum equals it. A consumer on an older compiler stays on the previous release.

## unixd-cache 0.0.1

- First release: a bounded on-disk cache with fresh and stale horizons, cross-process key locks, atomic writes, and configurable limits.

