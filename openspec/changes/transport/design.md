# Design

The evidence for each decision is in the [transport research][research].

## Decisions

| # | Decision | Alternative | Why the alternative lost |
|--:|:--|:--|:--|
| 1 | `LinesCodec::new_with_max_length`, then `serde_json::from_str`. | A hand-written `take(max + 1).read_until` loop, as the consumer has. | The codec already stops at the cap and yields `None` after the error; the loop would make EOF, `\r`, and the exact-limit case our code. |
| 2 | Close with no reply on an oversize or malformed frame. | Reply with an error envelope, as the consumer does. | The design says close. A reply to a peer that ignores the cap spends a write on a broken client. |
| 3 | Our own serde envelopes with `v`, `id`, and a tagged payload. | [jsonrpsee][jsonrpsee] or [tarpc][tarpc]. | jsonrpsee has no Unix transport, and tarpc frames with a binary length prefix. |
| 4 | The major version in the socket name and in every envelope. | The version in the envelope only. | A client and a daemon on different majors then never share a socket. |
| 5 | The caller's error type implements a trait that gives a code string and a retryable flag. | A fixed error table in `unixd`. | A fixed table ties the crate to one program, which is the gap this slice closes. |
| 6 | A blocking `std` client. | An async client on Tokio. | A CLI sends one request per run; an async client would force a runtime on it. |
| 7 | A read deadline and a write deadline per connection. | None, as the consumer has. | A silent client otherwise holds a handler until drain. |
| 8 | The client also checks the server's uid. | Trust the socket path. | The consumer does it, and it costs one call. |

## Overrides and disable paths

| Automatic behaviour | Override | Disable path |
|:--|:--|:--|
| Frame cap | `max_frame_bytes` in the server and client config | none; a cap is required |
| Per-connection deadlines | `read_timeout` and `write_timeout` | none; a daemon needs them |
| Peer uid check | none | none; it is the security boundary |

## Risks

1. `LinesCodec` holds up to about twice the cap while reading one line. The
   research inferred this and did not measure it.

[jsonrpsee]: https://crates.io/crates/jsonrpsee
[research]: ../../../docs/research/transport-and-lifecycle.md
[tarpc]: https://crates.io/crates/tarpc
