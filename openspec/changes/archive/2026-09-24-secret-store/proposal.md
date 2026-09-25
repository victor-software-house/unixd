# Proposal

## Why

UXD-007. A consumer may need to hold a token or a decrypted key between
requests. [`unixd-cache`][disk-cache] writes every entry to disk, so a secret
cannot go there, encrypted or not. A plain CLI run ends with the request, so
only the daemon can keep a secret across requests.

[Research][research] found that gpg-agent, ssh-agent, rbw-agent, and the fnox
daemon all keep secrets in memory only, with an idle TTL and, in most, a hard
maximum. None writes a secret to disk.

## What Changes

1. `unixd` gains a secret store that lives in the daemon process: `put`,
   `get`, `remove`, and `clear`.
2. Each secret expires after an idle TTL (10 minutes) and a maximum age
   (2 hours), both configurable per store and per `put`.
3. Values are [`secrecy`][secrecy] `SecretBox`, zeroed on drop and redacted in
   `Debug`.
4. Creating a store turns core dumps off, and on Linux marks the process not
   dumpable, which also keeps a debugger of the same user out.
5. The direct path has no store; the consumer asks the daemon for a secret
   or goes to the secret's source.

## Capabilities

### New Capabilities

- `secrets`: holding, expiring, and protecting secrets in daemon memory.

### Modified Capabilities

None.

## Impact

1. New dependency in `unixd`: [`secrecy`][secrecy] 0.10.3, which brings
   [`zeroize`][zeroize].
2. `rustix` already has the `process` feature for `setrlimit` and `prctl`.
3. `unixd-cache` does not change.

[disk-cache]: ../disk-cache/proposal.md
[research]: ../../../docs/research/cache-maintenance.md#3-secrets-in-memory
[secrecy]: https://crates.io/crates/secrecy
[zeroize]: https://crates.io/crates/zeroize
