//! Throwaway proof for UXD-001. The service manager passes the listening
//! socket on stdin; this takes it with safe `std` calls, answers one line per
//! connection with its own process id, and exits after an idle timeout
//! (`UNIXD_IDLE_SECS`, default 5) without touching the socket path.

use std::io::{self, Read, Write};
use std::os::fd::AsFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::time::{Duration, Instant};

/// The whole request and the reply must finish within this, so a client that
/// stalls or trickles bytes cannot hold the only accept loop past it.
const IO_TIMEOUT: Duration = Duration::from_secs(2);

/// The longest request line read before the reply.
const MAX_LINE: usize = 4096;

/// The most characters echoed back. The reply stays under 1 KiB, well inside
/// the socket send buffer, so writing it never waits on a slow reader.
const MAX_ECHO: usize = 256;

fn main() -> io::Result<()> {
    std::env::set_current_dir("/")?;
    let idle = std::env::var("UNIXD_IDLE_SECS")
        .ok()
        .and_then(|seconds| seconds.parse().ok())
        .map_or(Duration::from_secs(5), Duration::from_secs);
    let listener = UnixListener::from(io::stdin().as_fd().try_clone_to_owned()?);
    listener.set_nonblocking(true)?;
    let pid = std::process::id();
    let mut last = Instant::now();
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                if let Err(error) = serve(&stream, pid) {
                    eprintln!("activation: connection dropped: {error}");
                }
                last = Instant::now();
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if last.elapsed() >= idle {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => return Err(error),
        }
    }
}

/// Answers one connection. Its errors belong to that client alone and never
/// end the process. Each read and the reply wait only for the time left
/// before one deadline, because a socket timeout alone restarts on every
/// call.
fn serve(stream: &UnixStream, pid: u32) -> io::Result<()> {
    let deadline = Instant::now() + IO_TIMEOUT;
    let left = || {
        deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
            .ok_or_else(|| io::Error::from(io::ErrorKind::TimedOut))
    };
    stream.set_nonblocking(false)?;
    let mut request = Vec::new();
    let mut chunk = [0_u8; 512];
    while !request.contains(&b'\n') && request.len() < MAX_LINE {
        stream.set_read_timeout(Some(left()?))?;
        let read = (&*stream).read(&mut chunk)?;
        if read == 0 {
            break;
        }
        request.extend_from_slice(&chunk[..read]);
    }
    let text = String::from_utf8_lossy(&request);
    let line: String = text
        .lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(MAX_ECHO)
        .collect();
    stream.set_write_timeout(Some(left()?))?;
    writeln!(&*stream, "pid={pid} echo={line}")
}
