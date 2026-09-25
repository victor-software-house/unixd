use std::io::{self, Read as _, Write as _};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use std::{error, fmt, process};

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::{Fault, Limits, Outcome, Request, Response, Version, sys};

/// A blocking client for one daemon socket. Each [`Client::call`] opens one
/// connection and sends one request.
#[derive(Clone, Debug)]
pub struct Client {
    socket: PathBuf,
    version: Version,
    limits: Limits,
    deadline: Duration,
}

impl Client {
    /// A client for the daemon at `socket` that speaks `version`, with the
    /// default [`Limits`] and a 5 second deadline.
    pub fn new(socket: impl Into<PathBuf>, version: Version) -> Self {
        Self {
            socket: socket.into(),
            version,
            limits: Limits::default(),
            deadline: Duration::from_secs(5),
        }
    }

    /// Sets the one deadline for connect, write, and read together.
    #[must_use]
    pub fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = deadline;
        self
    }

    /// Sets the frame cap. The client uses only
    /// [`Limits::max_frame_bytes`].
    #[must_use]
    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Sends `body` and returns the handler's response.
    ///
    /// # Errors
    ///
    /// [`ClientError::Unavailable`] when no daemon listens, without waiting
    /// for the deadline; [`ClientError::Remote`] with the handler's error;
    /// and the other variants for the transport failures they name.
    pub fn call<B: Serialize, R: DeserializeOwned>(&self, body: &B) -> Result<R, ClientError> {
        let deadline = Instant::now() + self.deadline;
        let request = Request {
            v: self.version,
            id: request_id(),
            body,
        };
        let mut frame = serde_json::to_vec(&request).map_err(|_| ClientError::Encode)?;
        frame.push(b'\n');
        if frame.len() > self.limits.max_frame_bytes {
            return Err(ClientError::FrameTooLarge);
        }
        let stream = sys::connect(&self.socket, left(deadline)?).map_err(connect_error)?;
        if !sys::trusted_listener(sys::peer_uid(&stream).map_err(io_error)?) {
            return Err(ClientError::PeerMismatch);
        }
        write_frame(&stream, deadline, &frame)?;
        stream.shutdown(Shutdown::Write).map_err(io_error)?;
        let line = read_frame(&stream, deadline, self.limits.max_frame_bytes)?;
        let response: Response<R> =
            serde_json::from_slice(&line).map_err(|_| ClientError::InvalidFrame)?;
        if response.id != request.id {
            return Err(ClientError::InvalidFrame);
        }
        match response.outcome {
            Outcome::Ok(_) if response.v.major != self.version.major => {
                Err(ClientError::InvalidFrame)
            }
            Outcome::Ok(response) => Ok(response),
            Outcome::Err(fault) => Err(ClientError::Remote(fault)),
        }
    }
}

/// Why a [`Client::call`] failed.
#[derive(Debug)]
#[non_exhaustive]
pub enum ClientError {
    /// No daemon listens at the socket path.
    Unavailable,
    /// The deadline passed.
    Timeout,
    /// The socket belongs to another user. The client sees the credentials of
    /// whoever listens on the socket: the daemon's user under systemd, and
    /// root under launchd, which creates every agent's socket itself. On
    /// macOS the `0700` directory around the socket is what keeps other
    /// users out.
    PeerMismatch,
    /// The request or the response is larger than the frame cap.
    FrameTooLarge,
    /// The request body could not be serialized.
    Encode,
    /// The daemon closed the connection without a response, as it does for a
    /// request it refuses.
    Closed,
    /// The response is not a response envelope for this request.
    InvalidFrame,
    /// The handler's error, or one of the transport's own codes.
    Remote(Fault),
    /// Any other socket error.
    Io(io::Error),
}

impl fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("no daemon is listening"),
            Self::Timeout => formatter.write_str("the daemon did not answer in time"),
            Self::PeerMismatch => formatter.write_str("the daemon runs as another user"),
            Self::FrameTooLarge => formatter.write_str("a frame is larger than the cap"),
            Self::Encode => formatter.write_str("the request could not be serialized"),
            Self::Closed => formatter.write_str("the daemon closed the connection"),
            Self::InvalidFrame => formatter.write_str("the daemon sent an invalid response"),
            Self::Remote(fault) => write!(formatter, "{}: {}", fault.code, fault.message),
            Self::Io(error) => write!(formatter, "socket error: {error}"),
        }
    }
}

impl error::Error for ClientError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

/// Writes the frame. Each write waits only for the time left, as reads do,
/// and a write a signal interrupted is tried again.
fn write_frame(stream: &UnixStream, deadline: Instant, frame: &[u8]) -> Result<(), ClientError> {
    let mut writer = stream;
    let mut rest = frame;
    while !rest.is_empty() {
        stream
            .set_write_timeout(Some(left(deadline)?))
            .map_err(io_error)?;
        let written = match writer.write(rest) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            written => written.map_err(io_error)?,
        };
        if written == 0 {
            return Err(ClientError::Closed);
        }
        rest = &rest[written..];
    }
    Ok(())
}

/// Reads until the first newline. Each read waits only for the time left, so
/// a daemon that trickles bytes cannot stretch the deadline, and only the new
/// bytes are searched, so a reply near the cap costs one pass.
fn read_frame(stream: &UnixStream, deadline: Instant, max: usize) -> Result<Vec<u8>, ClientError> {
    let mut reader = stream;
    let mut frame = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        stream
            .set_read_timeout(Some(left(deadline)?))
            .map_err(io_error)?;
        let read = match reader.read(&mut chunk) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            read => read.map_err(io_error)?,
        };
        if read == 0 {
            return Err(if frame.is_empty() {
                ClientError::Closed
            } else {
                ClientError::InvalidFrame
            });
        }
        let start = frame.len();
        frame.extend_from_slice(&chunk[..read]);
        if let Some(offset) = chunk[..read].iter().position(|byte| *byte == b'\n') {
            let newline = start + offset;
            if newline >= max {
                return Err(ClientError::FrameTooLarge);
            }
            frame.truncate(newline);
            return Ok(frame);
        }
        if frame.len() >= max {
            return Err(ClientError::FrameTooLarge);
        }
    }
}

fn left(deadline: Instant) -> Result<Duration, ClientError> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|left| !left.is_zero())
        .ok_or(ClientError::Timeout)
}

fn connect_error(error: io::Error) -> ClientError {
    match error.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => ClientError::Unavailable,
        _ => io_error(error),
    }
}

fn io_error(error: io::Error) -> ClientError {
    match error.kind() {
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => ClientError::Timeout,
        _ => ClientError::Io(error),
    }
}

/// Unique within this process, which is all a reply check needs.
fn request_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "{:x}-{:x}",
        process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}
