//! Throwaway proof for UXD-001. The service manager passes the listening
//! socket on stdin; this takes it with safe `std` calls, answers one line per
//! connection with its own process id, and exits after an idle timeout
//! (`UNIXD_IDLE_SECS`, default 5) without touching the socket path.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::fd::AsFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::time::{Duration, Instant};

/// A client that stalls or sends an endless line cannot hold the only accept
/// loop past this, so the idle exit still fires.
const IO_TIMEOUT: Duration = Duration::from_secs(2);

/// The longest request line read before the reply.
const MAX_LINE: u64 = 4096;

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
/// end the process.
fn serve(stream: &UnixStream, pid: u32) -> io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    let mut line = String::new();
    BufReader::new(stream.take(MAX_LINE)).read_line(&mut line)?;
    writeln!(&*stream, "pid={pid} echo={}", line.trim_end())
}
