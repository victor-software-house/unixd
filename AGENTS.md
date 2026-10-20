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

The repository setup comes from the shared Copier template (project
`rust-crate`, layout `workspace`): `.copier-answers.yml` records its tag and
answers. Take a new tag with `copier update --defaults` and check
`git diff --check` before committing.
`ci.yml` and `template.yml` call the shared workflows of the public actions
repository at the commit the template pins; their checks report as
`verify / Verify` and `template / Template`. The drift check fails only a
pull request that changes `.copier-answers.yml` without reaching the newest
tag; a push to `main` or the daily run keeps a "Template update available"
issue open while a newer tag would change a file.

`mise.toml` pins stable 1.99.0 for build, lint, and test, and a dated nightly
for `rustfmt.toml`'s unstable options and for `doc`, which builds rustdoc as
docs.rs does; `rust-toolchain.toml` and `rust-version` match the stable pin.
The standard tasks come from the actions repository's `tasks/rust.toml`,
included by `git::` URL, and `mise.toml` composes `verify` from them. This
repository's own tasks are `check:features` in the template's starter
`mise-tasks/check/features.sh`, and `check:runtime-free` in
`mise-tasks/check.toml`.
The template owns `check:repo` (TOML format, Markdown, spelling, workflows,
OpenSpec, and `check:local`); this repository's own check goes in
`mise-tasks/check/local.sh` as `check:local`, never in `check:repo`.
`test` is the one task in `mise.toml`: it runs on nextest, because a signal
test stops its own process, and only an inline task replaces an included one.
Local work loads the `dev` profile, whose `mise install` arms the git hooks;
CI sets `MISE_ENV=ci`.

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
