# Coalescing

## ADDED Requirements

### Requirement: Identical requests share one run

`SingleFlight` SHALL run the work once for concurrent calls with the same key,
and SHALL give every caller the same value or the same error.

#### Scenario: Ten identical calls

- **WHEN** ten tasks call `SingleFlight::run` with one key while the work takes 200 ms
- **THEN** the work runs once and all ten receive its value

#### Scenario: Distinct keys

- **WHEN** two tasks call with different keys
- **THEN** both works run concurrently

#### Scenario: Shared failure

- **WHEN** the work fails with a timeout while three callers wait
- **THEN** all three receive the timeout error and nothing is cached

### Requirement: A disconnecting caller does not cancel the work

The work SHALL continue when the caller that started it is dropped, as long as
another caller waits or drain has not ended.

#### Scenario: Leader dropped

- **WHEN** the first caller is dropped after the work starts and a second caller waits
- **THEN** the second caller receives the value

### Requirement: Both paths give the same result

A request served by the daemon and the same request served by the direct path
SHALL give identical normalized output and identical cache entries.

#### Scenario: Cross-mode equivalence

- **WHEN** one fixture request runs through the daemon client with cache root A and through the direct path with cache root B
- **THEN** the outputs are equal and the entry files in A and B have equal values and horizons
