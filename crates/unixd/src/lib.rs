//! Per-user daemon runtime. See `docs/design/daemon-and-cache.md`.
//!
//! The platform service manager owns the socket: [`install`] writes a
//! `LaunchAgent` on macOS or a systemd socket and service pair on Linux, and
//! the manager starts the daemon on the first connection. [`serve`] takes the
//! socket from stdin and serves until a signal, a [`Shutdown`] request, or the
//! idle timeout drains it.
//!
//! One connection carries one request and one response, each a
//! newline-delimited JSON frame under [`Limits::max_frame_bytes`]. The server
//! checks that the peer runs as its own user before it reads, and a
//! [`Handler`] owns the payloads and the error codes. [`Client`] is blocking,
//! so a CLI needs no async runtime to call a daemon.

mod client;
mod envelope;
mod install;
mod limits;
mod serve;
mod server;
mod sys;

pub use client::{Client, ClientError};
pub use envelope::{Fault, Outcome, Request, Response, Version};
pub use install::{InstallError, Service, install, uninstall};
pub use limits::Limits;
pub use serve::{Lifecycle, Shutdown, serve, serve_listener};
pub use server::{Failure, Handler, serve_connection};
