# Research: activation, peer identity, and the disk cache

- **Read:** 2026-09-24, from man pages, first-party docs, the [crates.io] API,
  and source pinned to a commit.
- **Measured:** 2026-09-24, on macOS with launchd and on Linux with systemd 259.
- **Feeds:** [`docs/design/daemon-and-cache.md`][design]

Each finding names its source. A claim that comes only from reading code, with
no run behind it, says so.

## Summary

1. No maintained crate hands a caller an owned listener from launchd without
   the caller writing `unsafe`. Both managers can pass the listening socket on
   standard input instead, and safe standard-library calls adopt it. Both
   platforms were measured working.
2. [`tokio`][tokio-ucred] reads peer credentials on macOS and Linux with no
   `unsafe`.
3. No cache crate implements stale-if-error with cross-process locks and size
   bounds. The cache stays hand-written, on [`File::lock`][std-releases] and
   [`tempfile`][tempfile].
4. Under systemd's default start limit, a fast idle cycle fails the socket
   until someone resets it. The installer must set the limits.

## 1. launchd socket activation

The [`launchd.plist(5)`][launchd-plist] `Sockets` key makes launchd create the
socket. A job then either calls `launch_activate_socket` or, with
`inetdCompatibility` `Wait = true`, receives the listening socket on its
standard input.

| # | Crate | Version | What the caller gets | Verdict |
|--:|:--|:--|:--|:--|
| 1 | [`raunch`][raunch] | 1.0.1, 2024-04-15 | `Vec<RawFd>` from `activate_socket` | Adopting a `RawFd` needs `unsafe` in our code, which `forbid` rules out |
| 2 | [`launchd`][launchd-crate] | 0.3.0, 2023-08-08 | plist building only | Not an activation crate |
| 3 | [`listenfd`][listenfd-issue] | 1.0.2 | systemd only; launchd support requested in 2019 with no reply | Linux fallback only |

The stdin path needs no crate. [`launchd.plist(5)`][launchd-plist] says that
with `inetdCompatibility` the listening socket is passed through the stdio file
descriptors. Rust then adopts it with `stdin().as_fd().try_clone_to_owned()`
and `UnixListener::from`, both stable since 1.63.

▲ The same man page asks new jobs to avoid `inetdCompatibility`. The live runs
below show it working on the current macOS; a later release could change that.

## 2. systemd socket activation

| # | Crate | Version | What the caller gets | Verdict |
|--:|:--|:--|:--|:--|
| 1 | [`listenfd`][listenfd-take] | 1.0.2, 2025-01-19 | a standard `UnixListener`, no `unsafe` | Good; reads `LISTEN_FDS` only |
| 2 | [`tokio-listener`][tokio-listener] | 0.5.2, 2025-09-11 | a Tokio listener, `LISTEN_FDNAMES` aware | Good, but a large framework |
| 3 | [`sd-notify`][sd-notify] | 0.5.0, 2026-03-09 | `notify()` is safe; `listen_fds()` returns `RawFd` | Readiness only |
| 4 | [`libsystemd`][libsystemd] | 0.7.2, 2025-04-30 | a descriptor behind `IntoRawFd` | Caller needs `unsafe` |
| 5 | [`systemd`][systemd-crate] | 0.10.1 | links the C library | Rejected |

With `StandardInput=socket` on a single-socket `Accept=no` unit
([`systemd.exec(5)`][systemd-exec]), the same stdin code as on macOS works, so
one code path serves both platforms.

## 3. Idle exit and restart limits

1. Apple's [launch daemon guide][apple-launchd] says a job may exit when idle
   and launchd starts it again on the next request. The plist `TimeOut` key is
   no longer implemented, so the daemon times itself out. `KeepAlive` must stay
   unset for on-demand start.
2. `ThrottleInterval` defaults to 10 s between launches. Measured below, it did
   not delay socket-triggered relaunches.
3. [`systemd.socket(5)`][systemd-socket]: with `Accept=no` the socket stays in
   PID 1, and `FlushPending=no` (the default) keeps pending connections for the
   next start. Lennart Poettering confirms [no traffic is lost][poettering]
   apart from a connection already accepted.
4. The same page sets `TriggerLimitBurst` to 20 activations per 2 s, and
   [`systemd.unit(5)`][systemd-unit] sets `StartLimitBurst` to 5 starts per
   10 s. Both count every idle-exit restart.
5. So the daemon stops accepting before it exits, and never unlinks the socket
   or calls `shutdown(2)` on it.

## 4. Live activation measurements

The throwaway [`examples/activation.rs`][example] ran under each manager with a
5 s idle timeout.

| # | Observation | macOS, launchd | Linux, systemd 259 `--user` |
|--:|:--|:--|:--|
| 1 | Socket before any connection | `srw-------`, no process | `srw-------` in a `drwx------` directory, no process |
| 2 | First request, start included | 490 ms | 12 ms |
| 3 | Second request | same process | same process |
| 4 | 7 s later | no process, socket present | no process, socket present |
| 5 | Request right after the idle exit | new process, 17 ms | new process, 14 ms |
| 6 | Request after 16 s idle | new process, 18 ms | new process, 28 ms |
| 7 | Manager state | `runs = 3`, last exit code 0 | 3 starts, service and socket `success` |

A second Linux run used a 1 s idle timeout and a connection every 0.9 to 1.2 s:

| # | Unit settings | Connections served | Result |
|--:|:--|:--|:--|
| 1 | systemd defaults | 19 of 25 | the service hit its start limit after 10 starts; the socket entered `failed` and refused the rest until `reset-failed` |
| 2 | `StartLimitBurst=100` in 10 s | 25 of 25, over 14 starts | connections that arrived during an exit were served by the next start |

## 5. Installing per-user units

[`service-manager`][service-manager] 0.11.0 writes only a `.service` file on
systemd, with no `.socket`, and uses the legacy `launchctl load` and `unload`
on macOS. No other maintained crate expresses a systemd socket and service
pair. `unixd` writes both unit kinds itself and uses `launchctl bootstrap` and
`bootout`.

## 6. Peer identity

| # | Crate | macOS | Linux | Verdict |
|--:|:--|:--|:--|:--|
| 1 | [`tokio`][tokio-ucred] 1.53.1 `UnixStream::peer_cred` | `getpeereid` plus `LOCAL_PEEREPID` | `SO_PEERCRED` | Chosen |
| 2 | [`rustix`][rustix-peercred] 1.1.5 `socket_peercred` | not available | yes | Linux only |
| 3 | [`nix`][nix-getpeereid] 0.31.3 `getpeereid` and `PeerCredentials` | yes | yes | Fallback without Tokio |

## 7. The disk cache

| # | Crate | Version | Missing | Verdict |
|--:|:--|:--|:--|:--|
| 1 | [`http-cache-semantics`][http-cache-semantics] | 3.0.0 | `stale-if-error`; needs `http` request and response types | Optional freshness math behind a feature |
| 2 | [`cacache`][cacache] | 13.1.0 | per-key cross-process locks, size bounds, a freshness model | Rejected |
| 3 | [`http-cache`][http-cache] | 0.21.0 | synchronous use, stale-if-error | Rejected |
| 4 | [`moka`][moka] | 0.12.16 | a disk tier | Rejected |
| 5 | [`foyer`][foyer] | 0.22.6 | a runtime-free API | Rejected |

The pieces reused instead are [`std::fs::File::lock`][std-releases] (stable
since 1.89) for per-key locks, and [`tempfile`][tempfile] `persist` for atomic
writes.

## 8. Prior art: fnox

[fnox][fnox] v1.35.3 runs one daemon per profile and starts it itself:

1. The client [spawns `daemon serve`][fnox-spawn] with `setsid`, working
   directory `/`, and null stdio, then polls every 50 ms up to 50 times.
2. [`serve`][fnox-serve] probes the socket, removes it if stale, binds, and on
   idle exit removes the socket path.
3. It checks the peer uid with its own `unsafe` libc calls.
4. Fixed faults include a daemon wedged after its spawn directory was deleted
   ([discussion 793][fnox-793]).

Reading the code, not running it: two concurrent first clients can race
between the stale-socket probe and the bind, and a connection made between the
idle exit and the unlink is dropped. Socket activation removes both windows,
because the manager owns the socket and no client ever starts the daemon.

## Not in this record

Framing, envelopes, drain, coalescing, and socket placement are in the
[transport and lifecycle research][transport-research].

[apple-launchd]: https://developer.apple.com/library/archive/documentation/MacOSX/Conceptual/BPSystemStartup/Chapters/CreatingLaunchdJobs.html
[cacache]: https://github.com/zkat/cacache-rs/blob/66eae4b78f75eb2a38a2d25e838a56561294aebf/src/index.rs#L91
[crates.io]: https://crates.io
[design]: ../design/daemon-and-cache.md
[example]: ../../crates/unixd/examples/activation.rs
[fnox]: https://github.com/jdx/fnox
[fnox-793]: https://github.com/jdx/fnox/discussions/793
[fnox-serve]: https://github.com/jdx/fnox/blob/ba1a0889ac674f985608a75bfc5969e521ba4027/src/daemon.rs#L586-L707
[fnox-spawn]: https://github.com/jdx/fnox/blob/ba1a0889ac674f985608a75bfc5969e521ba4027/src/daemon.rs#L516-L584
[foyer]: https://github.com/foyer-rs/foyer/blob/dd46245c45071d1036331e4e2c48e15386017b96/README.md
[http-cache]: https://github.com/06chaynes/http-cache/blob/76207cb48e5bb7ed7e3fa65021560484101147dd/http-cache/Cargo.toml
[http-cache-semantics]: https://github.com/kornelski/rusty-http-cache-semantics/blob/b2141373103994ac3d6cc215d780b377b8e24be3/src/lib.rs#L362-L375
[launchd-crate]: https://crates.io/crates/launchd
[launchd-plist]: https://keith.github.io/xcode-man-pages/launchd.plist.5.html
[libsystemd]: https://github.com/lucab/libsystemd-rs/blob/7004a6013fbb44e58abca82cdd97b5f62e070ac5/src/activation.rs#L271-L283
[listenfd-issue]: https://github.com/mitsuhiko/listenfd/issues/4
[listenfd-take]: https://github.com/mitsuhiko/listenfd/blob/119770e00c0bf6b88b652251cbfe578e545efbcc/src/unix.rs#L110-L126
[moka]: https://github.com/moka-rs/moka/blob/a616ec19e8d4ed938caf8b2c88090331d778d5da/README.md
[nix-getpeereid]: https://github.com/nix-rust/nix/blob/b5933ca178802b558a667514f717a86b3a1cedcc/src/unistd.rs#L3998
[poettering]: https://lists.freedesktop.org/archives/systemd-devel/2020-September/045188.html
[raunch]: https://github.com/bbqsrc/raunch/blob/23617ab9f4957394c8bcfca9de8a19158e3f17c8/src/lib.rs#L48
[rustix-peercred]: https://github.com/bytecodealliance/rustix/blob/287214b889865d8e1406a0ee71cc409b6f6191c8/src/net/sockopt.rs#L1690-L1692
[sd-notify]: https://github.com/lnicola/sd-notify/blob/a4a9b073fb745d6ce15ea3db6cc471f1d370b76f/src/lib.rs#L178
[service-manager]: https://github.com/chipsenkbeil/service-manager-rs/blob/c5464bbd5d9f9b586d49581140ada7e3e9f01c61/src/systemd.rs#L130-L168
[std-releases]: https://github.com/rust-lang/rust/blob/1.89.0/RELEASES.md
[systemd-crate]: https://github.com/codyps/rust-systemd/blob/843cd3eae61fe24d634179def9b1714c7756e6cf/Cargo.toml
[systemd-exec]: https://www.freedesktop.org/software/systemd/man/latest/systemd.exec.html
[systemd-socket]: https://www.freedesktop.org/software/systemd/man/latest/systemd.socket.html
[systemd-unit]: https://www.freedesktop.org/software/systemd/man/latest/systemd.unit.html
[tempfile]: https://crates.io/crates/tempfile
[tokio-listener]: https://github.com/vi/tokio-listener/blob/d51b9c90c5cd26928c5e33b339600e8e4de2f473/src/listener.rs#L221-L252
[tokio-ucred]: https://github.com/tokio-rs/tokio/blob/75fef53d0a8590c2d1dbb63672aa7b7d1ef51155/tokio/src/net/unix/ucred.rs#L294-L338
[transport-research]: transport-and-lifecycle.md
