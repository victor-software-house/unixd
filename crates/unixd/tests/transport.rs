//! The transport over a real socket: round trips, refusals, deadlines, and
//! the client's checks.

#![expect(
    clippy::unwrap_used,
    reason = "setup helpers outside #[test] functions panic on failure like the tests do"
)]

use std::fmt;
use std::io::{Read as _, Write as _};
use std::os::unix::net::{UnixListener as StdListener, UnixStream as StdStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use tempfile::TempDir;
use tokio::net::UnixListener;
use unixd::{Client, ClientError, Failure, Handler, Limits, Version, serve_connection};

const V1: Version = Version { major: 1, minor: 0 };

#[derive(Debug)]
struct Missing;

impl fmt::Display for Missing {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("no such thing")
    }
}

impl Failure for Missing {
    fn code(&self) -> &'static str {
        "not_found"
    }
    fn retryable(&self) -> bool {
        false
    }
    fn details(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({ "status": 404 }))
    }
}

/// Echoes its request, except `missing`, which fails.
struct Echo;

impl Handler for Echo {
    type Request = String;
    type Response = String;
    type Error = Missing;

    fn handle(&self, request: String) -> impl Future<Output = Result<String, Missing>> + Send {
        std::future::ready(if request == "missing" {
            Err(Missing)
        } else {
            Ok(request)
        })
    }
}

/// Serves connections one at a time, so a stalled one delays the next.
fn start(limits: Limits) -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("d.sock");
    let listener = UnixListener::bind(&path).unwrap();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            serve_connection(stream, &Echo, V1, limits).await;
        }
    });
    (dir, path)
}

async fn call(client: Client, body: &str) -> Result<String, ClientError> {
    let body = body.to_owned();
    tokio::task::spawn_blocking(move || client.call(&body))
        .await
        .unwrap()
}

/// Linux resets a connection that its server closes with input unread, so a
/// read error ends the reply as end of file does.
fn read_all(mut stream: StdStream) -> Vec<u8> {
    let mut bytes = Vec::new();
    let _ = stream.read_to_end(&mut bytes);
    bytes
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_round_trips() {
    let (_dir, path) = start(Limits::default());
    assert_eq!(
        call(Client::new(&path, V1), "hello").await.unwrap(),
        "hello"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_handler_error_reaches_the_client_with_its_code() {
    let (_dir, path) = start(Limits::default());
    let Err(ClientError::Remote(fault)) = call(Client::new(&path, V1), "missing").await else {
        panic!("expected a remote error");
    };
    assert_eq!(fault.code, "not_found");
    assert!(!fault.retryable);
    assert_eq!(fault.message, "no such thing");
    assert_eq!(fault.details, Some(serde_json::json!({ "status": 404 })));
}

#[tokio::test(flavor = "multi_thread")]
async fn another_major_version_is_refused() {
    let (_dir, path) = start(Limits::default());
    let v2 = Version { major: 2, minor: 0 };
    let Err(ClientError::Remote(fault)) = call(Client::new(&path, v2), "hello").await else {
        panic!("expected a remote error");
    };
    assert_eq!(fault.code, "unsupported_version");
    assert_eq!(
        fault.details,
        Some(serde_json::json!({ "requested_major": 2, "supported_major": 1 }))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_oversize_frame_closes_the_connection_without_a_reply() {
    let limits = Limits {
        max_frame_bytes: 1024,
        ..Limits::default()
    };
    let (_dir, path) = start(limits);
    let replies = tokio::task::spawn_blocking(move || {
        let mut stream = StdStream::connect(&path).unwrap();
        let _ = stream.write_all(&vec![b'x'; 64 * 1024]);
        let _ = stream.shutdown(std::net::Shutdown::Write);
        read_all(stream)
    })
    .await
    .unwrap();
    assert_eq!(replies, b"");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_silent_client_is_dropped_after_the_read_timeout() {
    let limits = Limits {
        read_timeout: Duration::from_millis(300),
        ..Limits::default()
    };
    let (_dir, path) = start(limits);
    let silent = StdStream::connect(&path).unwrap();
    let started = Instant::now();
    assert_eq!(call(Client::new(&path, V1), "next").await.unwrap(), "next");
    assert!(started.elapsed() >= Duration::from_millis(250));
    let replies = tokio::task::spawn_blocking(move || read_all(silent))
        .await
        .unwrap();
    assert_eq!(replies, b"");
}

#[test]
fn no_daemon_is_unavailable_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let started = Instant::now();
    let result = Client::new(dir.path().join("none.sock"), V1)
        .with_deadline(Duration::from_secs(5))
        .call::<_, String>(&"hello");
    assert!(
        matches!(result, Err(ClientError::Unavailable)),
        "{result:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn a_reply_to_another_request_is_invalid() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("d.sock");
    let listener = StdListener::bind(&path).unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        stream.read_to_end(&mut request).unwrap();
        stream
            .write_all(b"{\"v\":{\"major\":1,\"minor\":0},\"id\":\"other\",\"ok\":\"hello\"}\n")
            .unwrap();
    });
    let result = Client::new(&path, V1).call::<_, String>(&"hello");
    server.join().unwrap();
    assert!(
        matches!(result, Err(ClientError::InvalidFrame)),
        "{result:?}"
    );
}

#[test]
fn a_reply_over_the_cap_is_refused_in_one_pass() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("d.sock");
    let listener = StdListener::bind(&path).unwrap();
    let cap = Limits::default().max_frame_bytes;
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        stream.read_to_end(&mut request).unwrap();
        let _ = stream.write_all(&vec![b'x'; cap + 1]);
    });
    let started = Instant::now();
    let result = Client::new(&path, V1)
        .with_deadline(Duration::from_secs(30))
        .call::<_, String>(&"hello");
    assert!(
        matches!(result, Err(ClientError::FrameTooLarge)),
        "{result:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(3));
    server.join().unwrap();
}

#[test]
fn limits_read_from_a_config_with_defaults_for_the_rest() {
    let limits: Limits = serde_json::from_str(r#"{"read_timeout": "2s"}"#).unwrap();
    assert_eq!(limits.read_timeout, Duration::from_secs(2));
    assert_eq!(limits.max_frame_bytes, Limits::default().max_frame_bytes);
    assert!(serde_json::from_str::<Limits>(r#"{"read_timeuot": "2s"}"#).is_err());
}
