# cache Specification

## Purpose
`unixd-cache` stores caller values on disk with fresh and stale horizons,
holds one fetch per key across processes, and keeps the cache under
configurable bounds, with no async runtime.

## Requirements

### Requirement: No async runtime

`unixd-cache` SHALL depend on no async runtime.

#### Scenario: Dependency tree

- **WHEN** `mise run check:runtime-free` runs
- **THEN** `cargo tree -p unixd-cache --edges normal` contains no `tokio`

### Requirement: Stale-if-error is a fixed table

The crate SHALL serve a stale entry only after a timeout, a network failure,
HTTP 429, or HTTP 5xx, and SHALL NOT serve it after HTTP 401, 403, or 404, a
response that is not valid JSON, or a response that does not match the
expected schema.

#### Scenario: Server error

- **WHEN** a caller holds a stale entry and the upstream answered HTTP 503
- **THEN** `Failure::Status(503).serves_stale()` is true

#### Scenario: Not found

- **WHEN** a caller holds a stale entry and the upstream answered HTTP 404
- **THEN** `Failure::Status(404).serves_stale()` is false

### Requirement: Writes are atomic

A store SHALL leave either the previous entry or the new one readable, never a
partial file.

#### Scenario: Interrupted write

- **WHEN** a writer dies after creating its temporary file and before the rename
- **THEN** a lookup returns the previous entry, valid, and the next prune removes the temporary file

### Requirement: Prune reaches the low-water target

When a write crosses a hard cap, the crate SHALL remove expired entries, then
the least recently used, until entries and bytes are at or below their
targets. It SHALL NOT remove an unexpired entry while an expired one remains,
except that it SHALL skip an entry whose key another caller holds.

#### Scenario: Over the entry cap

- **WHEN** `hard_entries` is 10, `target_entries` is 8, and an eleventh entry is stored
- **THEN** at most 8 entries remain and every removed entry was expired or used less recently than every kept one

#### Scenario: A read keeps an old entry

- **WHEN** `hard_entries` and `target_entries` are 2, entry A is stored, then B, A is read two hours later, and C is stored
- **THEN** B is removed and A is still `Lookup::Fresh`

### Requirement: Prune never waits for a key lock

Prune SHALL remove a file only under its key's lock taken without waiting,
SHALL skip a key another caller holds, and SHALL run while the calling
thread holds a `KeyLock`.

#### Scenario: Another key is locked

- **WHEN** a caller holds the lock for key B and another caller stores key C past a hard cap
- **THEN** the store returns `Maintenance::Pruned`

#### Scenario: Another prune is running

- **WHEN** one prune holds `prune.lock` and a store crosses a hard cap
- **THEN** the store returns `Maintenance::Deferred(Error::Lock)` and the entry is written

### Requirement: Unused entries expire

A write SHALL run a prune when the last one is older than
`Limits::sweep_every`, and prune SHALL remove an entry not read or written for
`Limits::unused_after`, even when it is unexpired.

#### Scenario: Thirty-one days unused

- **WHEN** an entry fresh for a year is stored and the next store comes 31 days later with the default limits
- **THEN** that store returns `Maintenance::Pruned` with `unused_removed` of 1, and the old entry reads as `Lookup::Miss`

### Requirement: One fetch per key across processes

The lock for a key SHALL exclude every other holder of that key, in any
process sharing the root, until it is dropped or its entry is stored.

#### Scenario: Second caller waits

- **WHEN** one caller holds the lock for a key and a second caller asks for it
- **THEN** the second caller blocks, and after the first stores, its locked lookup returns the first caller's entry as `Lookup::Fresh`

### Requirement: Lock files exist only while held

A key's lock file SHALL exist only while its key is held. Prune and `clear`
SHALL remove a lock file left by a process that died.

#### Scenario: A thousand keys that never store

- **WHEN** a caller locks and drops 1,000 distinct keys without storing
- **THEN** the `locks` directory is empty

### Requirement: An unlocked read never deletes

`Cache::lookup` SHALL NOT remove any file. Only a locked lookup and prune
SHALL remove an unusable entry, and neither SHALL remove an entry from a newer
schema before its stale horizon.

#### Scenario: Corrupt entry

- **WHEN** an entry holds invalid JSON
- **THEN** `Cache::lookup` returns `Lookup::Miss` and the file remains, and `KeyLock::lookup` returns `Lookup::Miss` and removes it

### Requirement: Oversized entries are not stored

A value whose serialized entry exceeds `Limits::max_entry_bytes` SHALL be
returned to the caller as `Stored::TooLarge` and SHALL NOT be written.

#### Scenario: 1 KiB value under a 256-byte ceiling

- **WHEN** `max_entry_bytes` is 256 and a 1 KiB value is stored
- **THEN** the store returns `Stored::TooLarge` and a lookup returns `Lookup::Miss`

### Requirement: Limits come from the consumer's config

`Limits` SHALL deserialize with every field optional and defaulted, SHALL read
durations written like `7d` or `10m`, and SHALL refuse an unknown field.

#### Scenario: Partial config

- **WHEN** a consumer deserializes `{"hard_entries": 50, "unused_after": "7d"}`
- **THEN** `hard_entries` is 50, `unused_after` is seven days, and every other field has its default

#### Scenario: Misspelled field

- **WHEN** a consumer deserializes `{"hard_entires": 50}`
- **THEN** deserialization fails

### Requirement: A schema bump keeps old entries as stale

An entry written under an older payload schema SHALL be served only as stale,
and SHALL NOT be served as fresh or deleted for that reason alone.

#### Scenario: Schema 2 reads a schema 1 entry

- **WHEN** a cache opened with schema 1 stored an entry with 1 hour of freshness, and a cache opened with schema 2 reads it a minute later
- **THEN** the lookup returns `Lookup::Stale`

### Requirement: Private files

The crate SHALL create its directories with mode `0700` and its files with
mode `0600`, SHALL refuse a root that is a symlink or owned by another user,
and SHALL write a `CACHEDIR.TAG` in the root so backup tools skip it.

#### Scenario: Symlinked root

- **WHEN** the cache root is a symlink to another directory
- **THEN** opening the cache returns `Error::UnsafeRoot`
