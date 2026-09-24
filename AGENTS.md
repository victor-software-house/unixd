# unixd

Two crates for a socket-activated per-user daemon and a bounded on-disk
cache. Library only; this repository ships no command.

Read [`docs/design/daemon-and-cache.md`](docs/design/daemon-and-cache.md)
before writing code. It is the contract, not a sketch.

This repo's queue is [`tasks.yaml`](tasks.yaml) (`UXD-###`), run with
`mise run q`.

## Layout

```text
Cargo.toml              # workspace root, no root package
crates/unixd/           # runtime shell, Tokio
crates/unixd-cache/     # cache policy, no reactor
docs/design/
```

## Rules

1. **`unixd-cache` must never depend on an async runtime.** Its consumer's
   direct path has no reactor. Only the daemon coalesces; the direct path
   relies on the key lock.
2. **`unsafe` is forbidden** at the workspace level, and every lint group is
   denied. Do not relax a lint to land a change.
3. **The service manager owns the socket.** [launchd][launchd] on macOS and
   [`systemd --user`][systemd] on Linux, one `Activation` implementation each. No call
   site outside the activation module branches on the platform. The daemon
   never spawns itself.
4. **No credential, path, or peer identity in a log line, an error, or a
   cache key.** Cache identity is about what was fetched, never who asked or
   how it will be rendered.
5. **Always open PRs.** Never push to `main`. Branch `type/short-desc`.
   Never `--no-verify`.
6. **Verification is named per change.** A behaviour claim carries the check
   that proves it. The design document's verification table is the baseline,
   and step 7 is a required regression.

## Development

Rust nightly, pinned by `rust-toolchain.toml`. Tasks come from
`mise.dev.toml`:

```sh
mise run format
mise run lint
mise run test
mise run verify
```

Lints are denied, not warned. `mise run verify` is the gate.

## Changes

Plan a behaviour or contract change as an OpenSpec change in
`openspec/changes/<name>/` before writing code. `openspec/config.yaml` holds
this repository's context and rules, and `openspec validate <name>` checks the
change.

[launchd]: https://keith.github.io/xcode-man-pages/launchd.plist.5.html
[systemd]: https://www.freedesktop.org/software/systemd/man/latest/systemd.socket.html
