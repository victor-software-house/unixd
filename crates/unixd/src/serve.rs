use std::io;
use std::os::fd::AsFd as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde::Deserialize;
use tokio::net::UnixListener;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::{Handler, Limits, Version, serve_connection};

/// When the daemon exits. A consumer embeds these in its own config next to
/// [`Limits`], and writes durations as `10m` or `5s`; an unknown field is an
/// error.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Lifecycle {
    /// Exit after this long with no open connection. 10 minutes; `0s`
    /// disables it.
    #[serde(deserialize_with = "humantime_serde::deserialize")]
    pub idle_timeout: Duration,
    /// How long a drain waits for open connections before it cuts them. 5
    /// seconds.
    #[serde(deserialize_with = "humantime_serde::deserialize")]
    pub drain_timeout: Duration,
}

impl Default for Lifecycle {
    fn default() -> Self {
        Self {
            idle_timeout: Duration::from_mins(10),
            drain_timeout: Duration::from_secs(5),
        }
    }
}

/// Starts a drain from inside a handler, for a shutdown request. The
/// response to that request is still written, because a drain waits for open
/// connections.
#[derive(Clone, Debug, Default)]
pub struct Shutdown(CancellationToken);

impl Shutdown {
    /// A handle no drain has used yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts the drain.
    pub fn request(&self) {
        self.0.cancel();
    }
}

/// Serves the daemon until a drain ends it: changes the working directory to
/// `/`, takes the listening socket the service manager passed on stdin, and
/// runs [`serve_listener`].
///
/// # Errors
///
/// An I/O error when stdin is not a listening socket, or when accepting fails
/// for a reason other than a connection the peer gave up.
pub async fn serve<H: Handler + 'static>(
    handler: Arc<H>,
    version: Version,
    limits: Limits,
    lifecycle: Lifecycle,
    shutdown: Shutdown,
) -> io::Result<()> {
    std::env::set_current_dir("/")?;
    let listener = std::os::unix::net::UnixListener::from(io::stdin().as_fd().try_clone_to_owned()?);
    listener.set_nonblocking(true)?;
    serve_listener(
        UnixListener::from_std(listener)?,
        handler,
        version,
        limits,
        lifecycle,
        shutdown,
    )
    .await
}

/// Accepts connections on `listener` and serves each on its own task until
/// `SIGINT`, `SIGTERM`, [`Shutdown::request`], or the idle timeout starts a
/// drain. A drain stops accepting, waits up to
/// [`Lifecycle::drain_timeout`] for open connections, cuts the rest, and
/// returns. It never unlinks or shuts down the socket, which the service
/// manager owns.
///
/// # Errors
///
/// An I/O error when the signal handlers cannot be installed, or when
/// accepting fails for a reason other than a connection the peer gave up.
pub async fn serve_listener<H: Handler + 'static>(
    listener: UnixListener,
    handler: Arc<H>,
    version: Version,
    limits: Limits,
    lifecycle: Lifecycle,
    shutdown: Shutdown,
) -> io::Result<()> {
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    let tracker = TaskTracker::new();
    let cut = CancellationToken::new();
    let open = Arc::new(AtomicUsize::new(0));
    let closed = Arc::new(Notify::new());
    let result = loop {
        let idle = async {
            if open.load(Ordering::SeqCst) == 0 && !lifecycle.idle_timeout.is_zero() {
                tokio::time::sleep(lifecycle.idle_timeout).await;
            } else {
                std::future::pending::<()>().await;
            }
        };
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    open.fetch_add(1, Ordering::SeqCst);
                    let handler = Arc::clone(&handler);
                    let cut = cut.clone();
                    let open = Arc::clone(&open);
                    let closed = Arc::clone(&closed);
                    tracker.spawn(async move {
                        tokio::select! {
                            () = serve_connection(stream, &*handler, version, limits) => {}
                            () = cut.cancelled() => {}
                        }
                        open.fetch_sub(1, Ordering::SeqCst);
                        closed.notify_one();
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::ConnectionAborted => {}
                Err(error) => break Err(error),
            },
            () = closed.notified() => {}
            () = idle => break Ok(()),
            () = shutdown.0.cancelled() => break Ok(()),
            _ = terminate.recv() => break Ok(()),
            _ = interrupt.recv() => break Ok(()),
        }
    };
    drop(listener);
    tracker.close();
    if tokio::time::timeout(lifecycle.drain_timeout, tracker.wait())
        .await
        .is_err()
    {
        cut.cancel();
        tracker.wait().await;
    }
    result
}
