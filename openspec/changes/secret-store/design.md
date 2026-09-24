# Design

## Context

The daemon runtime from the [lifecycle change][lifecycle] owns one long-lived
process per user. The [research][research] compared five agents that hold
secrets. This change adds the smallest store that matches them.

## Goals

1. A secret never reaches disk: no file, no swap where `mlock` succeeds, and no
   core dump.
2. A secret leaves memory at its TTL, at `remove` or `clear`, or at daemon
   exit, and its bytes are zeroed.
3. Only the daemon's own user can ask for it.

## Non-goals

1. Persistence across daemon restarts.
2. Encryption at rest. Nothing is at rest.
3. Protection from root or from the same user with a debugger allowed by the
   OS. The OS peer check and the dump settings are the whole boundary.

## Decisions

1. **Memory only, in `unixd`.** An encrypted entry in `unixd-cache` lost: it
   needs a key, and the key needs the same in-memory home, so the disk copy
   adds risk and no benefit.
2. **`secrecy` `SecretBox`.** A hand-written zeroing wrapper lost: `secrecy`
   0.10.3 already zeroes on drop, redacts `Debug`, and blocks `Clone` unless
   the value opts in.
3. **Two TTLs.** Idle 10 minutes and maximum 2 hours, gpg-agent's defaults. A
   single TTL lost: a secret in steady use would never expire. Override: the
   store's `SecretLimits` and a per-`put` `ttl`. Disable: a TTL of `None`
   keeps the secret until `remove`, `clear`, or exit.
4. **Expiry on access and on a timer.** A `get` checks both TTLs first. The
   daemon's idle loop sweeps expired secrets once a minute. A sweep on access
   alone lost: an unused secret would stay in memory until daemon exit.
5. **Hardening at daemon start.** `RLIMIT_CORE` 0 on both platforms,
   `PR_SET_DUMPABLE` 0 on Linux, `PT_DENY_ATTACH` on macOS, as ssh-agent does.
   It runs only when the daemon registers a store, so a daemon without secrets
   keeps normal crash dumps. Override: none; a store without it breaks goal 1.
6. **Best-effort `mlock`.** The store locks its pages and logs one warning on
   failure, as rbw does. A hard failure lost: Linux defaults
   `RLIMIT_MEMLOCK` to 8 MiB, and some containers set 0.
7. **Limits from the consumer's config.** `SecretLimits` derives `Deserialize`
   with `#[serde(default, deny_unknown_fields)]` and humantime durations, the
   same shape as the cache's `Limits`.

## Overrides

| # | Behaviour | Override | Disable |
|--:|:--|:--|:--|
| 1 | Idle TTL | `SecretLimits.idle`, per-`put` `ttl` | `None` |
| 2 | Maximum age | `SecretLimits.max_age`, per-`put` `ttl` | `None` |
| 3 | Sweep interval | `SecretLimits.sweep_every` | none; expiry on access still applies |
| 4 | `mlock` | none | fails soft by design |

## Risks

1. A copy made by the consumer outside `SecretBox` is not zeroed. The API
   hands out `&SecretBox` only, and the docs say so.
2. `mlock` can fail silently on a system with swap and a low limit. The
   warning names the limit.

[lifecycle]: ../activation-lifecycle/design.md
[research]: ../../../docs/research/cache-maintenance.md#3-secrets-in-memory
