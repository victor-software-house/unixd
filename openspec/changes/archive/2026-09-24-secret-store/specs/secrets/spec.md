# Secrets

## ADDED Requirements

### Requirement: Secrets stay in memory

The secret store SHALL NOT write a secret to any file, and the daemon SHALL
write no core dump while a store is registered.

#### Scenario: No core dump

- **WHEN** a process creates a store
- **THEN** `getrlimit(RLIMIT_CORE)` reads 0 for both limits, and on Linux the process is not dumpable

### Requirement: Secrets expire

The store SHALL remove a secret once it is idle for longer than its idle TTL,
or older than its maximum age, whichever comes first.

#### Scenario: Idle expiry

- **WHEN** a secret is stored with the default limits and the test clock advances 11 minutes with no `get`
- **THEN** `get` returns `None`

#### Scenario: Maximum age

- **WHEN** a secret is read every 5 minutes for 2 hours and 1 minute on the test clock
- **THEN** the last `get` returns `None`

#### Scenario: No TTL

- **WHEN** a secret is stored with `ttl: None` and the test clock advances 30 days
- **THEN** `get` returns the secret

### Requirement: Removed secrets are zeroed

The store SHALL zero a secret's bytes when it expires, is removed, is
cleared, or the daemon exits.

#### Scenario: Remove

- **WHEN** `remove` drops a secret
- **THEN** the value's `Zeroize` runs, shown by a test value that records the call

### Requirement: Secrets do not print

A secret SHALL NOT appear in `Debug` output, logs, or error messages.

#### Scenario: Debug

- **WHEN** a test formats the store and a stored secret with `{:?}`
- **THEN** the output does not contain the secret's bytes

### Requirement: Limits come from the consumer's config

`SecretLimits` SHALL deserialize from the consumer's config with humantime
durations, default every missing field, and reject an unknown field.

#### Scenario: Partial config

- **WHEN** a config sets only `idle: 5m`
- **THEN** `idle` is 5 minutes and `max_age` is 2 hours

#### Scenario: Unknown field

- **WHEN** a config sets `idel: 5m`
- **THEN** deserialization fails and names `idel`
