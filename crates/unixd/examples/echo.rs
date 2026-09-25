//! A daemon that answers each request with its own process id and the
//! request text. `shutdown` drains it. The install tests run it under launchd
//! and systemd; the first argument is the idle timeout in seconds, 5 by
//! default.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use std::{fmt, io};

use serde::Serialize;
use unixd::{Failure, Handler, Lifecycle, Limits, Shutdown, Version, serve};

const VERSION: Version = Version { major: 1, minor: 0 };

#[derive(Serialize)]
struct Echoed {
    pid: u32,
    text: String,
}

struct Never(Infallible);

impl fmt::Display for Never {
    fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {}
    }
}

impl Failure for Never {
    fn code(&self) -> &'static str {
        match self.0 {}
    }
    fn retryable(&self) -> bool {
        match self.0 {}
    }
}

struct Echo(Shutdown);

impl Handler for Echo {
    type Request = String;
    type Response = Echoed;
    type Error = Never;

    fn handle(&self, text: String) -> impl Future<Output = Result<Echoed, Never>> + Send {
        if text == "shutdown" {
            self.0.request();
        }
        std::future::ready(Ok(Echoed {
            pid: std::process::id(),
            text,
        }))
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> io::Result<()> {
    let idle = std::env::args()
        .nth(1)
        .and_then(|seconds| seconds.parse().ok())
        .unwrap_or(5);
    let lifecycle = Lifecycle {
        idle_timeout: Duration::from_secs(idle),
        ..Lifecycle::default()
    };
    let shutdown = Shutdown::new();
    let echo = Arc::new(Echo(shutdown.clone()));
    serve(echo, VERSION, Limits::default(), lifecycle, shutdown).await
}
