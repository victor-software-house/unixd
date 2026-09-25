# Proposal

## Why

UXD-003. The first consumer's transport works and is tested: a 16 MiB frame
cap, versioned envelopes, a same-uid peer check, and a client with one
deadline. It is tied to that consumer in two ways. It returns the consumer's
error type, and a fixed table in its protocol crate maps about 30 remote error
codes to that type. The [extraction map][map] lists two more gaps: an
oversize frame gets an error reply before the close, and no read deadline
stops a client that connects and never sends.

## What Changes

1. `unixd` reads newline-delimited JSON frames through [`tokio-util`][tokio-util]
   `LinesCodec` with a caller cap, and closes the connection with no reply on
   an oversize or malformed frame.
2. Request and response envelopes carry the protocol major and minor version
   and a request id. The payload and the error code belong to the caller.
3. A `Handler` trait receives one decoded request and returns one response or
   a caller error. The caller's error type supplies its code and its
   retryable flag.
4. The server checks the peer's effective uid before it reads, and applies a
   read deadline and a write deadline to each connection.
5. A blocking client in `std` sends one request with one deadline, checks the
   server's uid, and checks the reply's request id.

## Capabilities

### New Capabilities

- `transport`: frames, envelopes, the handler trait, the peer check, and the
  client.

### Modified Capabilities

None.

## Impact

1. New dependencies in `unixd`: [`tokio`][tokio] (`net`, `io-util`, `time`,
   `rt`), [`tokio-util`][tokio-util] (`codec`), [`serde`][serde],
   [`serde_json`][serde-json], and [`rustix`][rustix] (`process`).
2. The first consumer's error-code table moves into the consumer.

[map]: ../../../docs/research/extraction-map.md
[rustix]: https://crates.io/crates/rustix
[serde]: https://crates.io/crates/serde
[serde-json]: https://crates.io/crates/serde_json
[tokio]: https://crates.io/crates/tokio
[tokio-util]: https://crates.io/crates/tokio-util
