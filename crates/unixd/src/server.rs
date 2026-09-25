use std::fmt::Display;
use std::future::Future;

use futures_util::StreamExt as _;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::UnixStream;
use tokio::time::timeout;
use tokio_util::codec::{FramedRead, LinesCodec};

use crate::{Fault, Limits, Outcome, Request, Response, Version, sys};

/// An error a [`Handler`] returns. Its code and retryable flag go on the wire
/// as they are, and its `Display` text becomes the message.
pub trait Failure: Display {
    /// A stable code the client matches on.
    fn code(&self) -> &str;
    /// Whether the same request may succeed later.
    fn retryable(&self) -> bool;
    /// Structured context for [`Fault::details`]. None by default.
    fn details(&self) -> Option<Value> {
        None
    }
}

/// Answers one decoded request. The daemon's own types define the payloads
/// and the errors; the transport only moves them.
pub trait Handler: Send + Sync {
    /// The request body.
    type Request: DeserializeOwned + Send;
    /// The response body.
    type Response: Serialize + Send;
    /// The error, sent as a [`Fault`].
    type Error: Failure + Send;

    /// Answers `request`.
    fn handle(
        &self,
        request: Self::Request,
    ) -> impl Future<Output = Result<Self::Response, Self::Error>> + Send;
}

/// Serves one connection: one request, one response, then close.
///
/// A peer running as another user, a request that does not arrive whole
/// within [`Limits::read_timeout`], a frame over [`Limits::max_frame_bytes`],
/// and a frame that is not a request envelope all close the connection with
/// no reply. Nothing is logged, so a stranger learns nothing.
pub async fn serve_connection<H: Handler>(
    stream: UnixStream,
    handler: &H,
    version: Version,
    limits: Limits,
) {
    serve_as(stream, handler, version, limits, sys::own_uid()).await;
}

async fn serve_as<H: Handler>(
    mut stream: UnixStream,
    handler: &H,
    version: Version,
    limits: Limits,
    uid: u32,
) {
    if sys::peer_uid(&stream).ok() != Some(uid) {
        return;
    }
    let read = read_frame(&mut stream, limits.max_frame_bytes);
    let Ok(Some(line)) = timeout(limits.read_timeout, read).await else {
        return;
    };
    let Ok(request) = serde_json::from_str::<Request<Value>>(&line) else {
        return;
    };
    let outcome = respond(handler, version, request.v, request.body).await;
    let response = Response {
        v: version,
        id: request.id,
        outcome,
    };
    let Ok(mut frame) = serde_json::to_vec(&response) else {
        return;
    };
    frame.push(b'\n');
    let write = async {
        stream.write_all(&frame).await?;
        stream.shutdown().await
    };
    let _ = timeout(limits.write_timeout, write).await;
}

async fn respond<H: Handler>(
    handler: &H,
    version: Version,
    sent: Version,
    body: Value,
) -> Outcome<H::Response> {
    if sent.major != version.major {
        return Outcome::Err(Fault {
            code: "unsupported_version".to_owned(),
            message: format!(
                "this server speaks major version {}, not {}",
                version.major, sent.major
            ),
            retryable: false,
            details: Some(serde_json::json!({
                "requested_major": sent.major,
                "supported_major": version.major,
            })),
        });
    }
    let Ok(request) = serde_json::from_value(body) else {
        return Outcome::Err(Fault {
            code: "invalid_request".to_owned(),
            message: "the request body is not one this server answers".to_owned(),
            retryable: false,
            details: None,
        });
    };
    match handler.handle(request).await {
        Ok(response) => Outcome::Ok(response),
        Err(error) => Outcome::Err(Fault {
            code: error.code().to_owned(),
            message: error.to_string(),
            retryable: error.retryable(),
            details: error.details(),
        }),
    }
}

/// Reads one line of at most `max` bytes, newline included, and never reads
/// past `max`. A longer line, a read error, and a connection closed before
/// any byte all read as `None`.
async fn read_frame(reader: impl AsyncRead + Unpin, max: usize) -> Option<String> {
    let limited = reader.take(u64::try_from(max).unwrap_or(u64::MAX));
    let codec = LinesCodec::new_with_max_length(max.saturating_sub(1));
    FramedRead::new(limited, codec).next().await?.ok()
}

#[cfg(test)]
mod tests {
    use std::fmt;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    const V1: Version = Version { major: 1, minor: 0 };

    struct Never;

    impl Display for Never {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("never")
        }
    }

    impl Failure for Never {
        fn code(&self) -> &'static str {
            "never"
        }
        fn retryable(&self) -> bool {
            false
        }
    }

    #[derive(Default)]
    struct Recorder(AtomicBool);

    impl Handler for Recorder {
        type Request = Value;
        type Response = Value;
        type Error = Never;

        fn handle(&self, request: Value) -> impl Future<Output = Result<Value, Never>> + Send {
            self.0.store(true, Ordering::SeqCst);
            std::future::ready(Ok(request))
        }
    }

    #[tokio::test]
    async fn an_oversize_frame_is_never_read_past_the_cap() {
        let mut source = tokio::io::repeat(b'x').take(64 * 1024);
        assert_eq!(read_frame(&mut source, 1024).await, None);
        assert!(64 * 1024 - source.limit() <= 1024);
    }

    #[tokio::test]
    async fn a_frame_of_exactly_the_cap_is_read() {
        let frame = format!("{}\n", "x".repeat(1023));
        assert_eq!(
            read_frame(frame.as_bytes(), 1024)
                .await
                .map(|line| line.len()),
            Some(1023)
        );
    }

    #[tokio::test]
    async fn another_users_connection_closes_before_any_read() {
        let (server, mut client) = UnixStream::pair().unwrap();
        let recorder = Recorder::default();
        let request = serde_json::to_vec(&Request {
            v: V1,
            id: "1".to_owned(),
            body: Value::Null,
        })
        .unwrap();
        client.write_all(&request).await.unwrap();
        client.write_all(b"\n").await.unwrap();
        let other = sys::own_uid().wrapping_add(1);
        serve_as(server, &recorder, V1, Limits::default(), other).await;
        let mut reply = Vec::new();
        let _ = client.read_to_end(&mut reply).await;
        assert_eq!(reply, b"");
        assert!(!recorder.0.load(Ordering::SeqCst));
    }
}
