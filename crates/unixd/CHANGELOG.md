# Changelog

## unixd 0.0.2

- `Fault` carries optional `details`, which a handler error supplies through `Failure::details`, and `unsupported_version` reports the requested and supported majors there.

## unixd 0.0.1

- First release: a socket-activated per-user daemon runtime with unit install for launchd and systemd, a blocking client, single flight, and an in-memory secret store.

