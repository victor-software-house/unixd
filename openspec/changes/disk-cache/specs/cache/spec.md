# Cache

## ADDED Requirements

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

### Requirement: Keys never carry who asked or how it is shown

The key builder SHALL reject a part whose name marks a credential, a
credential hash, an output format, a destination path, or a request id.

#### Scenario: Token part

- **WHEN** a caller adds a part named `api_token`
- **THEN** the builder returns `Error::ForbiddenKeyPart("api_token")` and the error text contains no part value

### Requirement: Writes are atomic

A store SHALL leave either the previous entry or the new one readable, never a
partial file.

#### Scenario: Interrupted write

- **WHEN** a writer dies after creating its temporary file and before the rename
- **THEN** a lookup returns the previous entry, valid, and the next prune removes the temporary file

### Requirement: Prune reaches the low-water target

When a write crosses a hard cap, the crate SHALL remove expired entries, then
the oldest, until entries and bytes are at or below their targets, and SHALL
NOT remove an unexpired entry while an expired one remains.

#### Scenario: Over the entry cap

- **WHEN** `hard_entries` is 10, `target_entries` is 8, and an eleventh entry is stored
- **THEN** at most 8 entries remain and every removed entry was expired or older than every kept one

### Requirement: A schema bump keeps old entries as stale

An entry written under an older payload schema SHALL be served only as stale,
and SHALL NOT be served as fresh or deleted for that reason alone.

#### Scenario: Schema 2 reads a schema 1 entry

- **WHEN** a cache opened with schema 1 stored an entry with 1 hour of freshness, and a cache opened with schema 2 reads it a minute later
- **THEN** the lookup returns `Lookup::Stale`

### Requirement: Private files

The crate SHALL create its directories with mode `0700` and its files with
mode `0600`, and SHALL refuse a root that is a symlink or owned by another
user.

#### Scenario: Symlinked root

- **WHEN** the cache root is a symlink to another directory
- **THEN** opening the cache returns `Error::UnsafeRoot`
