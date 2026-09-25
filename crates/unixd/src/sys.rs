//! The only platform branches in the transport.

use std::io;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use rustix::net::sockopt::{self, Timeout};
use rustix::net::{AddressFamily, SocketAddrUnix, SocketType};

pub(crate) fn own_uid() -> u32 {
    rustix::process::geteuid().as_raw()
}

#[cfg(target_os = "linux")]
pub(crate) fn peer_uid(socket: impl AsFd) -> io::Result<u32> {
    Ok(sockopt::socket_peercred(socket)?.uid.as_raw())
}

#[cfg(target_os = "macos")]
pub(crate) fn peer_uid(socket: impl AsFd) -> io::Result<u32> {
    let (uid, _) = nix::unistd::getpeereid(socket)?;
    Ok(uid.as_raw())
}

/// Connects with a send timeout set first. Linux applies it to a connect that
/// waits on a full backlog; macOS refuses such a connect at once.
pub(crate) fn connect(path: &Path, timeout: Duration) -> io::Result<UnixStream> {
    let socket = stream_socket()?;
    sockopt::set_socket_timeout(&socket, Timeout::Send, Some(timeout))?;
    rustix::net::connect(&socket, &SocketAddrUnix::new(path)?)?;
    Ok(UnixStream::from(socket))
}

#[cfg(target_os = "linux")]
fn stream_socket() -> io::Result<OwnedFd> {
    use rustix::net::SocketFlags;
    Ok(rustix::net::socket_with(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::CLOEXEC,
        None,
    )?)
}

#[cfg(target_os = "macos")]
fn stream_socket() -> io::Result<OwnedFd> {
    use rustix::io::FdFlags;
    let socket = rustix::net::socket(AddressFamily::UNIX, SocketType::STREAM, None)?;
    rustix::io::fcntl_setfd(&socket, FdFlags::CLOEXEC)?;
    Ok(socket)
}
