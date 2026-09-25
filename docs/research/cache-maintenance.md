# Research: cache maintenance, file-safety crates, and secrets in memory

- **Checked:** 2026-09-24, against the [crates.io] API, crate sources at
  pinned commits, man pages, and first-party docs.
- **Feeds:** the [design][design], the [disk cache change][disk-cache], and
  the [secret store change][secret-store].

## Summary

1. No maintained crate replaces the cache's hand-written file-safety code.
2. Neither macOS nor Linux cleans a plain program's cache directory, so the
   cache evicts its own entries.
3. Mature tools pair a size cap with eviction by last use, and expire unused
   entries on a daily check.
4. Comparable agents keep secrets in memory only, with an idle TTL and a hard
   maximum, and never write them to disk.

## 1. File-safety crates

The cache hand-writes private directories, private lock files, lock files
deleted on release with an inode re-check, and reads that never follow a
symlink, all on [`rustix`][rustix].

| # | Crate | Version | Covers | Verdict |
|--:|:--|:--|:--|:--|
| 1 | [`cap-std`][cap-std] and `cap-fs-ext` | 4.0.3 | no-follow and nonblocking opens | Rejected: no owner check, follows symlinks at the root, and adds about 8 crates and more `unsafe` than it removes |
| 2 | [`fs4`][fs4], [`fd-lock`][fd-lock], [`fs-lock`][fs-lock], `fmutex` | 1.1.0, 4.0.4, 0.1.16, 0.3.0 | `flock` | Rejected: the same `flock` as `std::fs::File::lock` |
| 3 | [`lockfile`][lockfile] | 0.4.0 | delete on release | Rejected: `create_new` with no `flock`, so a crash leaves a stale lock |
| 4 | [`named-lock`][named-lock] | 0.4.1 | named locks | Rejected: locks in `/tmp`, no mode or owner check |
| 5 | `dir-lock` | 0.5.0 | delete on release | Rejected: needs Tokio |
| 6 | [`pathrs`][pathrs] | 0.2.6 | safe path resolution | Rejected: Linux only |
| 7 | `fslock`, `file-guard`, `file-lock`, `openat`, `openat2` | | locks, no-follow opens | Rejected: unmaintained, `fcntl` locks, or Linux only |

None implements a lock file deleted on release with the waiter's inode
re-check, and none checks the owner of the root. The disk-cache crates
rejected in the [activation research][activation] have no newer release.

## 2. Who cleans a cache directory

| # | Location or tool | Cleanup | Default |
|--:|:--|:--|:--|
| 1 | macOS `~/Library/Caches` | [`deleted`][deleted] purges only for registered services and app containers ([DTS answer][forum-707643]) | nothing for a plain program |
| 2 | macOS `$TMPDIR` | `dirhelper` daily, and a full wipe at boot ([source][dirhelper]) | regular files older than 3 days |
| 3 | `$XDG_CACHE_HOME` | the [spec][basedir] defines no deletion | nothing |
| 4 | systemd `--user` | [`tmpfiles.d`][tmpfiles] can age a directory; no default rule covers `~/.cache` | nothing |
| 5 | [`CACHEDIR.TAG`][cachedir] | backup tools skip the directory | |

What mature tools do:

| # | Tool | Policy |
|--:|:--|:--|
| 1 | [sccache][sccache] | least recently used, 10 GiB, configurable |
| 2 | [ccache][ccache] | least recently used by mtime, 5 GiB, mtime refreshed on hit, configurable |
| 3 | [cargo][cargo-gc] | unused for 1 to 3 months, checked at most daily, configurable |
| 4 | [Go build cache][go-cache] | unused for 5 days, checked at most daily, mtime refreshed at most hourly |
| 5 | [pip][pip], [uv][uv] | manual only |

## 3. Secrets in memory

| # | Tool | TTL | Memory protection |
|--:|:--|:--|:--|
| 1 | [gpg-agent][gpg-agent] | 10 min idle, 2 h maximum | secure memory, no core dumps |
| 2 | [ssh-agent][ssh-agent] | none unless `-t` | not dumpable on Linux, no debugger attach on macOS ([source][ssh-tracing]) |
| 3 | [rbw-agent][rbw] | 1 h lock timeout | `mlock` and zeroize ([source][rbw-locked]), no core dumps |
| 4 | [1Password CLI][op-security] | 10 min idle, 12 h maximum | the desktop app holds the keys |
| 5 | [fnox daemon][fnox-daemon] | 8 h idle for the whole cache | memory only, peer uid check |

None writes a secret to disk. [`secrecy`][secrecy] 0.10.3 gives `SecretBox`,
zeroed on drop and redacted in `Debug`. A failed `mlock` is a warning in rbw,
since Linux defaults `RLIMIT_MEMLOCK` to 8 MiB.

[activation]: activation-and-cache.md#7-the-disk-cache
[basedir]: https://gitlab.freedesktop.org/xdg/xdg-specs/-/blob/d546132d944a5f1e729c52aa6c2623edeaf750ed/basedir/basedir-spec.xml
[cachedir]: https://bford.info/cachedir/
[cap-std]: https://github.com/bytecodealliance/cap-std/blob/b7acf8e8807fe3fab991884d2208b7e03d35a409/cap-primitives/src/rustix/fs/dir_utils.rs#L105-L118
[cargo-gc]: https://github.com/rust-lang/cargo/blob/694054f34bcb04025b16d0eaf075038c0e58a15d/src/workspace/gc.rs#L31-L38
[ccache]: https://github.com/ccache/ccache/blob/b471bbde2923994781821e8d8893e7180ba45168/doc/manual.adoc
[crates.io]: https://crates.io
[deleted]: https://keith.github.io/xcode-man-pages/deleted.8.html
[design]: ../design/daemon-and-cache.md
[dirhelper]: https://github.com/st3fan/osx-10.9/blob/34e34a6a539b5a822cda4074e56a7ced9b57da71/system_cmds-597.1.1/dirhelper.tproj/dirhelper.c#L195-L260
[disk-cache]: ../../openspec/changes/disk-cache/design.md
[fd-lock]: https://github.com/yoshuawuyts/fd-lock/tree/af18798c1790003815a8180fb1929c0a7da1f512
[fnox-daemon]: https://github.com/jdx/fnox/blob/ba1a0889ac674f985608a75bfc5969e521ba4027/docs/guide/daemon.md
[forum-707643]: https://developer.apple.com/forums/thread/707643
[fs-lock]: https://github.com/cargo-bins/cargo-binstall/tree/e00d89e73103a913d466402312bc229045c05c44/crates/fs-lock
[fs4]: https://github.com/al8n/fs4/tree/df476ee1de2926ae4599607c325a5aa1d334501d
[go-cache]: https://github.com/golang/go/blob/6479e38e37f9451d075bfa0b526347d47d301035/src/cmd/go/internal/cache/cache.go#L337-L350
[gpg-agent]: https://github.com/gpg/gnupg/blob/310606278cd54434f1a25de655cb8bf7ce8ac4e4/agent/gpg-agent.c#L339-L342
[lockfile]: https://github.com/derekdreery/lockfile-rs/blob/07f23684c88d939a49007c9b602e74bcf4bebf54/src/lib.rs#L115-L130
[named-lock]: https://github.com/oblique/named-lock/blob/c7a540737900c0068ce37511e925b84fbb2a66c4/src/unix.rs#L16-L22
[op-security]: https://developer.1password.com/docs/cli/app-integration-security/
[pathrs]: https://github.com/cyphar/libpathrs/blob/941f5488a93811bc6c00f6aa4f8c29095252f33b/src/lib.rs#L86
[pip]: https://github.com/pypa/pip/blob/a7002c9771a6c3f0317a4e6b9fbdcd22e643f7b6/docs/html/topics/caching.md
[rbw]: https://github.com/doy/rbw/blob/77464d414a9e41165d5b15f049bee1f0c0f2a982/src/config.rs#L45-L47
[rbw-locked]: https://github.com/doy/rbw/blob/77464d414a9e41165d5b15f049bee1f0c0f2a982/src/locked.rs
[rustix]: https://crates.io/crates/rustix
[sccache]: https://github.com/mozilla/sccache/blob/8396f0209d74d496b7cb27cdf323cd3ff8d4a291/docs/Local.md
[secrecy]: https://github.com/iqlusioninc/crates/tree/70eaa76ea3f4bacd67f3027c4a52948485a67d32/secrecy
[secret-store]: ../../openspec/changes/secret-store/proposal.md
[ssh-agent]: https://github.com/openssh/openssh-portable/blob/ccc26c76cd47ca224ff5e4ef8b96007b65ff0b4e/ssh-agent.c#L192-L193
[ssh-tracing]: https://github.com/openssh/openssh-portable/blob/ccc26c76cd47ca224ff5e4ef8b96007b65ff0b4e/platform-tracing.c#L59-L74
[tmpfiles]: https://github.com/systemd/systemd/blob/7abf4dbbf4662ca29b7525b7f3ddf0105dd1362a/man/tmpfiles.d.xml
[uv]: https://github.com/astral-sh/uv/blob/bea138450f0e620a4ce5765b0e38cff7b9f0799f/docs/concepts/cache.md
