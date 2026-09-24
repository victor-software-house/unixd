# Transport

## ADDED Requirements

### Requirement: Frames have a size cap

The server SHALL read newline-delimited JSON frames of at most
`max_frame_bytes`, including the newline, and SHALL close the connection
without a reply when a frame exceeds the cap or is not valid JSON.

#### Scenario: Round trip

- **WHEN** a client sends one request frame under the cap
- **THEN** the handler receives the decoded request and the client receives one response frame

#### Scenario: Oversize frame

- **WHEN** `max_frame_bytes` is 1024 and a client writes 64 KiB with no newline
- **THEN** the server closes the connection with no reply, having read at most about 2 KiB of it

### Requirement: Envelopes carry the protocol version

Every request and response envelope SHALL carry the protocol major and minor
version and a request id, and the server SHALL reject a request whose major
version differs from its own.

#### Scenario: Unsupported major

- **WHEN** a v1 server receives a request with major version 2
- **THEN** it replies with an error envelope whose code is `unsupported_version` and closes

### Requirement: The peer is checked before any read

The server SHALL compare the peer's effective uid with its own before it reads
a frame, and SHALL close a mismatched connection without a reply and without
logging the peer's identity.

#### Scenario: Same user

- **WHEN** a process of the same user connects and sends a request
- **THEN** the handler runs

#### Scenario: Different user

- **WHEN** the peer check reports a uid other than the server's
- **THEN** the connection closes with no bytes written and the handler does not run

### Requirement: Connections have deadlines

The server SHALL close a connection that sends no complete frame within
`read_timeout`, or that does not accept the response within `write_timeout`.

#### Scenario: Silent client

- **WHEN** `read_timeout` is 1 s and a client connects and sends nothing
- **THEN** the server closes the connection after about 1 s and still serves the next client

### Requirement: The caller owns error codes

The handler SHALL return a caller error type that supplies an error code and a
retryable flag, and the transport SHALL NOT map codes to its own error type.

#### Scenario: Handler error

- **WHEN** the handler returns an error with code `not_found` and retryable false
- **THEN** the client receives an error envelope with code `not_found` and retryable false

### Requirement: The client has one deadline

The blocking client SHALL apply one deadline to connect, write, and read, SHALL
check the server's uid, and SHALL reject a reply whose request id differs from
the request.

#### Scenario: No daemon

- **WHEN** the socket path does not exist
- **THEN** the client returns `Unavailable` without waiting for the deadline

#### Scenario: Wrong request id

- **WHEN** the server replies with a different request id
- **THEN** the client returns `InvalidFrame`
