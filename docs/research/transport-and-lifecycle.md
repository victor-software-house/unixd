# Research: transport, lifecycle, and coalescing

- **Checked:** 2026-09-24, against the crates.io API, source at pinned commits,
  the man pages installed on macOS 26.6.2, and upstream documentation.
- **Feeds:** the [transport][transport-change], [lifecycle][lifecycle-change],
  and [coalescing][coalescing-change] changes, and the [design][design].

## 1. Result

Four existing crates and two short pieces of our own code cover the transport
and lifecycle:

| Job | Choice | Runner-up |
|:--|:--|:--|
| Framing with a size cap | [`tokio-util`][tokio-util] 0.7.19 `LinesCodec::new_with_max_length` | a `take(max + 1).read_until(b'\n')` loop |
| Envelopes and versioning | our own [`serde`][serde] structs; the major version in the socket name | JSON-RPC 2.0 field names without a crate |
| Drain and idle exit | `CancellationToken`, `TaskTracker`, [`tokio`][tokio] 1.53.1 timers and signals | `JoinSet` with `abort_all` |
| Single flight | our own map of shared task handles, about 40 lines | [`async_singleflight`][async-singleflight] 0.6.2 |
| Blocking client with timeouts | std `UnixStream` with `set_read_timeout` and `set_write_timeout` | [`socket2`][socket2] 0.6.5 `connect_timeout` |
| Peer uid | [`tokio`][tokio] `UnixStream::peer_cred`, and [`rustix`][rustix-geteuid] 1.1.5 `geteuid` | `nix` 0.31.3 |

No RPC framework, shutdown framework, or single-flight crate is worth its
dependency cost here.

## 2. Framing

1. **`LinesCodec::new_with_max_length`.** Each [`decode`][lines-decode] call
   scans at most `max_length + 1` bytes. With no newline in that span, it
   returns `MaxLineLengthExceeded` and enters a discard mode.
   [`FramedRead`][framed-none] then yields the error followed by `None`, so the
   caller can drop the connection at once. It never reads to the end of an
   oversize frame. It also rejects invalid UTF-8.
2. **Memory bound.** The read buffer [starts at 8 KiB][framed-capacity] and
   grows while a line is read. The peak of about twice the cap is inferred from
   `BytesMut` growth, not measured.
3. **`LengthDelimitedCodec`.** It [checks the limit][length-limit] before it
   buffers a body, but its binary length prefix is not newline-delimited JSON.
4. **`serde_json::StreamDeserializer`.** It has a [recursion limit][json-depth]
   of 128 and no byte limit, so it is unsafe on an untrusted socket.
5. **A hand-written `take` loop.** Tokio's `Take` implements `AsyncBufRead`,
   and `read_until` is [cancel-safe][read-until]. The cap holds, but EOF, a
   trailing `\r`, and a line exactly at the limit become our code.

## 3. Envelopes and versioning

1. [jsonrpsee][jsonrpsee] 0.26.0 ships HTTP, WebSocket, and WASM transports,
   and no Unix socket transport.
2. [tarpc][tarpc] 0.38.0 has a Unix transport, but it
   [frames with `LengthDelimitedCodec`][tarpc-framing] and its own envelope.
   The wire format would then belong to tarpc.
3. [interprocess][interprocess] 2.4.4 abstracts socket types, which matters
   mainly on Windows. [parity-tokio-ipc][parity-ipc] 0.9.0 was last released
   2021-07-06.
4. [fnox][fnox-wire] hashes its wire version into the socket filename, so a
   client and a daemon on different wire formats never meet on one socket.
   OpenSSH and gpg-agent do not version their socket names.

The choice: the major version goes in the socket name and in every envelope.
A `v` field carries additive minor changes.

## 4. Drain and idle exit

1. [`TaskTracker`][task-tracker] offers `close`, `wait`, `spawn`, and
   `token`. Dropping it does not abort its tasks, so it is for waiting.
2. [`CancellationToken`][cancel-token] offers `child_token`, `cancelled`, and
   `run_until_cancelled`.
3. [`Sleep::reset`][sleep-reset] moves an idle deadline without allocating.
   [`SignalKind::terminate`][signal-kind] and `interrupt` cover the two stop
   signals.
4. Dropping the runtime [stops spawned tasks][runtime-drop] at their next
   yield point.

The sequence that follows from these:

1. A `select!` over accept, `SIGTERM`, `SIGINT`, a shutdown request, and the
   idle `Sleep`. The `Sleep` is reset on every accept and every finished
   request.
2. On exit from the loop, drop the listener, close the tracker, and cancel the
   token given to connections.
3. Wait on the tracker with a grace timeout.
4. Return from `main`. Dropping the runtime aborts what is left.

Two frameworks were set aside:

- [tokio-graceful-shutdown][tgs-deps] 0.20.0 adds `miette`, `atomic`,
  `bytemuck`, and `thiserror` for a subsystem tree nobody needs here.
- [tokio-graceful][tokio-graceful] 0.2.2 was last released 2024-09-30 and wraps
  the same pattern.

## 5. Single flight

| Option | When the leader is cancelled | Errors |
|:--|:--|:--|
| [`async_singleflight`][asf-leader] 0.6.2 | one follower is promoted and runs its own future | only the leader sees `E` |
| [`singleflight-async`][sfa-wait] 0.2.0, last released 2024-11-06 | the next waiter runs its own closure | shared when `T` is a cloneable `Result` |
| [`moka`][moka-guard] 0.12.16 `try_get_with` | waiters retry up to 200 times, then panic | waiters get `Arc<E>`; successes are cached, so pure coalescing needs invalidation |
| tokio `OnceCell` | another waiter [starts a new attempt][once-cell] | one value; needs our own map |
| [`futures::future::Shared`][shared] 0.3.34 | runs while any clone polls it; a panic poisons it | needs `Output: Clone`, so `Arc<E>` |

The choice is our own map. The work runs as a task on the `TaskTracker`, and
its `JoinHandle` is shared through `Shared`. Every waiter then gets the value
or `Arc<E>`, a client that disconnects does not cancel shared work, the entry
leaves the map when the work ends, and drain waits for work in flight.

## 6. Client and socket paths

1. **Path length.** `sun_path` is 104 bytes on macOS and
   [108 on Linux][unix7]. Rust std [rejects a path][std-sun-path] whose length
   reaches the array size, so the usable maximum is 103 and 107 bytes.
2. **Connect with a timeout.** Neither std nor tokio has a timeout for a Unix
   socket connect. A local connect does no network waiting.
   [`socket2`][socket2] `connect_timeout` switches to non-blocking and polls,
   but Linux returns `EAGAIN` for a full backlog, and that path is untested.
3. **Linux placement.** The [XDG spec][xdg] requires `$XDG_RUNTIME_DIR` to be
   owned by the user with mode 0700, and removes it at full logout.
   systemd expands `%t` to it in a user unit.
4. **launchd placement.** [`launchd.plist`][launchd-plist] describes
   `SockPathName` as the path to bind, with no variable expansion.
   `EnableGlobbing` applies to program arguments only. Third-party reports
   agree that `~` and `$HOME` arrive as literal text
   ([nix-darwin #406][nix-darwin-406]). The installer must write an absolute
   path.
5. **Prior art.**
   - [fnox][fnox-wire] uses `$XDG_RUNTIME_DIR/fnox`, else a uid-named temp
     directory, with a hashed fallback for long paths. It reads requests with
     an [unbounded `read_line`][fnox-read-line].
   - OpenSSH 10.1 moved agent sockets into `~/.ssh/agent`
     ([release notes][openssh-rn]).
   - gpg-agent uses `/run/user/<uid>/gnupg` when present, else the home
     directory, and [ignores `XDG_RUNTIME_DIR`][gnupg-homedir].
   - 1Password documents `~/.1password/agent.sock` as an optional link to its
     [macOS socket][op-ssh].

## 7. Peer identity

[`UnixStream::peer_cred`][tokio-peer-cred] uses `SO_PEERCRED` on Linux, and
[`LOCAL_PEEREPID` plus `getpeereid`][tokio-ucred-macos] on macOS. The values
are the credentials at connect time. Our own effective uid comes from
[`rustix::process::geteuid`][rustix-geteuid], a safe call.

## 8. `inetdCompatibility` is still supported

The [`launchd.plist`][launchd-plist] page on macOS 26.6.2 asks new projects to
avoid the key, and describes `Wait = true` as passing the listening socket on
stdin. No Apple statement deprecates or removes it. [TN2083][tn2083] and
[Creating Launch Daemons and Agents][apple-launchd] still document it. macOS
26.6.2 ships nine system jobs that use it, including `ssh.plist` and
`com.apple.kdumpd`.

## 9. Not verified

1. The peak memory of `LinesCodec` near the cap. It is inferred.
2. `socket2` `connect_timeout` on a Linux Unix socket with a full backlog.
3. Whether a blocking `connect` blocks on macOS when the backlog is full.
4. launchd `SockPathName` expansion. It rests on the man page and third-party
   reports; no test job was loaded.
5. A hidden Apple deprecation of `inetdCompatibility` that search did not find.

[apple-launchd]: https://developer.apple.com/library/archive/documentation/MacOSX/Conceptual/BPSystemStartup/Chapters/CreatingLaunchdJobs.html
[asf-leader]: https://github.com/PureWhiteWu/async_singleflight/blob/4b2c2768f13bbf4f41ec251d58c9ebebd02dca90/src/group.rs#L165-L186
[async-singleflight]: https://crates.io/crates/async_singleflight
[cancel-token]: https://github.com/tokio-rs/tokio/blob/f2189d3bd69d22638158a0ca8163b2e8daf18c5f/tokio-util/src/sync/cancellation_token.rs#L204-L300
[coalescing-change]: ../../openspec/changes/coalescing/proposal.md
[design]: ../design/daemon-and-cache.md
[fnox-read-line]: https://github.com/jdx/fnox/blob/efaa682d140e7ef1d9383e9116dfe57757d2d659/src/daemon.rs#L824-L832
[fnox-wire]: https://github.com/jdx/fnox/blob/efaa682d140e7ef1d9383e9116dfe57757d2d659/src/daemon.rs#L1306-L1349
[framed-capacity]: https://github.com/tokio-rs/tokio/blob/f2189d3bd69d22638158a0ca8163b2e8daf18c5f/tokio-util/src/codec/framed_impl.rs#L25
[framed-none]: https://github.com/tokio-rs/tokio/blob/f2189d3bd69d22638158a0ca8163b2e8daf18c5f/tokio-util/src/codec/framed_impl.rs#L160-L170
[gnupg-homedir]: https://github.com/gpg/gnupg/blob/310606278cd54434f1a25de655cb8bf7ce8ac4e4/common/homedir.c#L1372-L1411
[interprocess]: https://crates.io/crates/interprocess
[json-depth]: https://github.com/serde-rs/json/blob/de8500740cdcabffb9734f503e4889def823cf10/src/de.rs#L63
[jsonrpsee]: https://crates.io/crates/jsonrpsee
[launchd-plist]: https://keith.github.io/xcode-man-pages/launchd.plist.5.html
[length-limit]: https://github.com/tokio-rs/tokio/blob/f2189d3bd69d22638158a0ca8163b2e8daf18c5f/tokio-util/src/codec/length_delimited.rs#L520-L526
[lifecycle-change]: ../../openspec/changes/activation-lifecycle/proposal.md
[lines-decode]: https://github.com/tokio-rs/tokio/blob/f2189d3bd69d22638158a0ca8163b2e8daf18c5f/tokio-util/src/codec/lines_codec.rs#L112-L164
[moka-guard]: https://github.com/moka-rs/moka/blob/a616ec19e8d4ed938caf8b2c88090331d778d5da/src/future/value_initializer.rs#L186-L213
[nix-darwin-406]: https://github.com/nix-darwin/nix-darwin/issues/406
[once-cell]: https://github.com/tokio-rs/tokio/blob/75fef53d0a8590c2d1dbb63672aa7b7d1ef51155/tokio/src/sync/once_cell.rs#L344-L347
[op-ssh]: https://www.1password.dev/ssh/get-started.md
[openssh-rn]: https://www.openssh.org/releasenotes.html
[parity-ipc]: https://crates.io/crates/parity-tokio-ipc
[read-until]: https://github.com/tokio-rs/tokio/blob/75fef53d0a8590c2d1dbb63672aa7b7d1ef51155/tokio/src/io/util/async_buf_read_ext.rs#L40-L46
[runtime-drop]: https://github.com/tokio-rs/tokio/blob/75fef53d0a8590c2d1dbb63672aa7b7d1ef51155/tokio/src/runtime/runtime.rs#L30-L35
[rustix-geteuid]: https://github.com/bytecodealliance/rustix/blob/287214b889865d8e1406a0ee71cc409b6f6191c8/src/process/id.rs#L53
[serde]: https://serde.rs
[sfa-wait]: https://github.com/ihciah/singleflight-async/blob/60f9c45f6edd8af79c041bc7c6c13ce821e64626/src/lib.rs#L118-L131
[shared]: https://github.com/rust-lang/futures-rs/blob/705e6b5c0f06535b1aac1cb1989a172b3d45be8c/futures-util/src/future/future/shared.rs#L300-L331
[signal-kind]: https://github.com/tokio-rs/tokio/blob/75fef53d0a8590c2d1dbb63672aa7b7d1ef51155/tokio/src/signal/unix.rs#L200
[sleep-reset]: https://github.com/tokio-rs/tokio/blob/75fef53d0a8590c2d1dbb63672aa7b7d1ef51155/tokio/src/time/sleep.rs#L344
[socket2]: https://github.com/rust-lang/socket2/blob/239dd83a4ced08e514d2c38942aab99791119f0d/src/socket.rs#L215-L229
[std-sun-path]: https://github.com/rust-lang/rust/blob/f340ff6a39bd17c6d8e17c90c6f8a60438bf0644/library/std/src/os/unix/net/addr.rs#L39-L42
[tarpc]: https://crates.io/crates/tarpc
[tarpc-framing]: https://github.com/google/tarpc/blob/5d54c216096fb88b0a26d0c431c0483611601110/tarpc/src/serde_transport.rs#L17
[task-tracker]: https://github.com/tokio-rs/tokio/blob/f2189d3bd69d22638158a0ca8163b2e8daf18c5f/tokio-util/src/task/task_tracker.rs#L318-L381
[tgs-deps]: https://github.com/Finomnis/tokio-graceful-shutdown/blob/5692f55101bad9283430efb55c79fe65a17891e4/Cargo.toml#L30-L45
[tn2083]: https://developer.apple.com/library/archive/technotes/tn2083/_index.html
[tokio]: https://tokio.rs
[tokio-graceful]: https://github.com/plabayo/tokio-graceful/blob/2be9b6541d140386ca670f521b5a9870c4e9a167/src/shutdown.rs#L436
[tokio-peer-cred]: https://github.com/tokio-rs/tokio/blob/75fef53d0a8590c2d1dbb63672aa7b7d1ef51155/tokio/src/net/unix/ucred.rs#L81-L144
[tokio-ucred-macos]: https://github.com/tokio-rs/tokio/blob/75fef53d0a8590c2d1dbb63672aa7b7d1ef51155/tokio/src/net/unix/ucred.rs#L287-L338
[transport-change]: ../../openspec/changes/transport/proposal.md
[tokio-util]: https://github.com/tokio-rs/tokio/tree/f2189d3bd69d22638158a0ca8163b2e8daf18c5f/tokio-util
[unix7]: https://man7.org/linux/man-pages/man7/unix.7.html
[xdg]: https://specifications.freedesktop.org/basedir/latest/
