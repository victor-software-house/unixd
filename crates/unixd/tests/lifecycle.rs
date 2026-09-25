//! The serve loop in process: signals, the shutdown request, the idle timer,
//! and the drain bound. The signal test sends `SIGTERM` to its own process,
//! which drains every daemon in it, so these tests need a process each, as
//! nextest runs them, or `--test-threads=1`.

#![expect(
    clippy::unwrap_used,
    reason = "setup helpers outside #[test] functions panic on failure like the tests do"
)]

use std::convert::Infallible;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tempfile::TempDir;
use tokio::net::UnixListener;
use tokio::task::JoinHandle;
use unixd::{
    Client, ClientError, Failure, Handler, Lifecycle, Limits, Shutdown, Version, serve_listener,
};

const V1: Version = Version { major: 1, minor: 0 };

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

/// Sleeps for the number of milliseconds it is sent, then answers. `shutdown`
/// starts a drain.
struct Sleeper(Shutdown);

impl Handler for Sleeper {
    type Request = String;
    type Response = String;
    type Error = Never;

    async fn handle(&self, request: String) -> Result<String, Never> {
        if request == "shutdown" {
            self.0.request();
        } else if let Ok(millis) = request.parse() {
            tokio::time::sleep(Duration::from_millis(millis)).await;
        }
        Ok(request)
    }
}

struct Daemon {
    _dir: TempDir,
    path: PathBuf,
    shutdown: Shutdown,
    task: JoinHandle<std::io::Result<()>>,
}

fn start(lifecycle: Lifecycle) -> Daemon {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("d.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let shutdown = Shutdown::new();
    let handler = Arc::new(Sleeper(shutdown.clone()));
    let task = tokio::spawn(serve_listener(
        listener,
        handler,
        V1,
        Limits::default(),
        lifecycle,
        shutdown.clone(),
    ));
    Daemon {
        _dir: dir,
        path,
        shutdown,
        task,
    }
}

fn call(path: &PathBuf, body: &str) -> JoinHandle<Result<String, ClientError>> {
    let client = Client::new(path, V1).with_deadline(Duration::from_secs(10));
    let body = body.to_owned();
    tokio::task::spawn_blocking(move || client.call(&body))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_signal_during_a_request_still_delivers_its_response() {
    let daemon = start(Lifecycle::default());
    let reply = call(&daemon.path, "500");
    tokio::time::sleep(Duration::from_millis(150)).await;
    rustix::process::kill_process(rustix::process::getpid(), rustix::process::Signal::TERM)
        .unwrap();
    assert_eq!(reply.await.unwrap().unwrap(), "500");
    daemon.task.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shutdown_request_is_answered_before_the_drain() {
    let daemon = start(Lifecycle::default());
    assert_eq!(
        call(&daemon.path, "shutdown").await.unwrap().unwrap(),
        "shutdown"
    );
    daemon.task.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_idle_timer_waits_for_open_requests() {
    let daemon = start(Lifecycle {
        idle_timeout: Duration::from_millis(300),
        ..Lifecycle::default()
    });
    let started = Instant::now();
    assert_eq!(call(&daemon.path, "900").await.unwrap().unwrap(), "900");
    daemon.task.await.unwrap().unwrap();
    assert!(started.elapsed() >= Duration::from_millis(1100));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_drain_cuts_connections_after_its_timeout() {
    let daemon = start(Lifecycle {
        drain_timeout: Duration::from_millis(200),
        ..Lifecycle::default()
    });
    let reply = call(&daemon.path, "5000");
    tokio::time::sleep(Duration::from_millis(150)).await;
    let started = Instant::now();
    daemon.shutdown.request();
    daemon.task.await.unwrap().unwrap();
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(matches!(reply.await.unwrap(), Err(ClientError::Closed)));
}

#[test]
fn lifecycle_reads_from_a_config_with_defaults_for_the_rest() {
    let lifecycle: Lifecycle = serde_json::from_str(r#"{"idle_timeout": "0s"}"#).unwrap();
    assert_eq!(lifecycle.idle_timeout, Duration::ZERO);
    assert_eq!(lifecycle.drain_timeout, Lifecycle::default().drain_timeout);
}
