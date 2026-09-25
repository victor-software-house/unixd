//! Per-user daemon runtime. See `docs/design/daemon-and-cache.md`.
//!
//! This release has the transport. One connection carries one request and
//! one response, each a newline-delimited JSON frame under
//! [`Limits::max_frame_bytes`]. The server checks that the peer runs as its
//! own user before it reads, and a [`Handler`] owns the payloads and the error
//! codes. [`Client`] is blocking, so a CLI needs no async runtime to call a
//! daemon.

mod client;
mod envelope;
mod limits;
mod server;
mod sys;

pub use client::{Client, ClientError};
pub use envelope::{Fault, Outcome, Request, Response, Version};
pub use limits::Limits;
pub use server::{Failure, Handler, serve_connection};
