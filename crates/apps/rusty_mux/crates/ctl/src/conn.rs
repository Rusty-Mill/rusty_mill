//! A blocking framed connection to the server.

use std::io::{self, Read, Write};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
#[cfg(any(unix, windows))]
use std::path::Path;

#[cfg(windows)]
use platform_windows::WindowsUnixStream as UnixStream;
use rusty_mux_proto::{Message, decode, encode};

#[cfg(any(unix, windows))]
use crate::stop::POLL;
use crate::stop::{Stop, is_wait};

/// A stream to the server at `path`. When `stop` is bounded, connecting
/// gives up with it too (on Linux; elsewhere only once connected), and the
/// stream times out every [`POLL`] so [`Conn`] notices a stop. On Windows
/// only reads time out, so a write to a server that stopped reading is not
/// bounded.
#[cfg(any(unix, windows))]
pub(crate) fn connect(path: &Path, stop: &Stop) -> io::Result<UnixStream> {
    if !stop.is_bounded() {
        return connect_blocking(path);
    }
    let stream = connect_bounded(path, stop)?;
    stream.set_read_timeout(Some(POLL))?;
    #[cfg(unix)]
    stream.set_write_timeout(Some(POLL))?;
    Ok(stream)
}

#[cfg(unix)]
fn connect_blocking(path: &Path) -> io::Result<UnixStream> {
    UnixStream::connect(path)
}

/// Never `InvalidData`, which would read as a protocol failure (exit 1)
/// instead of an unreachable server (exit 4).
#[cfg(windows)]
fn connect_blocking(path: &Path) -> io::Result<UnixStream> {
    UnixStream::connect(path).map_err(|e| io::Error::other(format!("{e:?}")))
}

/// Connects without blocking, retrying while the server's accept backlog
/// is full, which `std`'s connect would wait out for ever.
#[cfg(target_os = "linux")]
fn connect_bounded(path: &Path, stop: &Stop) -> io::Result<UnixStream> {
    use std::os::fd::{FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;

    use rusty_libc::Errno;
    use rusty_libc::socket::{
        AF_UNIX, SOCK_CLOEXEC, SOCK_NONBLOCK, SOCK_STREAM, SockAddrUn, connect_un, socket,
    };

    stop.check()?;
    let addr = SockAddrUn::new(path.as_os_str().as_bytes()).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "socket path too long or empty")
    })?;
    let fd = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC | SOCK_NONBLOCK, 0)?;
    // SAFETY: `fd` is a socket just opened here and owned by nothing else.
    let stream = UnixStream::from(unsafe { OwnedFd::from_raw_fd(fd) });
    loop {
        match connect_un(fd, &addr) {
            Ok(()) => break,
            Err(Errno::EAGAIN | Errno::EINTR) => {
                stop.check()?;
                std::thread::sleep(POLL);
            }
            Err(errno) => return Err(errno.into()),
        }
    }
    stream.set_nonblocking(false)?;
    Ok(stream)
}

#[cfg(any(windows, all(unix, not(target_os = "linux"))))]
fn connect_bounded(path: &Path, stop: &Stop) -> io::Result<UnixStream> {
    stop.check()?;
    connect_blocking(path)
}

/// A connection whose reads and writes give up when `stop` says so; the
/// stream must time out (see [`crate::stop::POLL`]) for a silent server to
/// be noticed.
pub(crate) struct Conn<S> {
    stream: S,
    buf: Vec<u8>,
    stop: Stop,
}

impl<S: Read + Write> Conn<S> {
    pub(crate) fn new(stream: S, stop: Stop) -> Self {
        Self {
            stream,
            buf: Vec::new(),
            stop,
        }
    }

    pub(crate) fn send(&mut self, message: &Message) -> io::Result<()> {
        let frame = encode(message).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        let mut rest = frame.as_slice();
        while !rest.is_empty() {
            self.stop.check()?;
            match self.stream.write(rest) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(n) => rest = &rest[n..],
                Err(e) if is_wait(&e) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// The next message; `UnexpectedEof` when the server closes,
    /// `InvalidData` for a frame that does not decode, and the
    /// [`Stop::check`] error once stopped.
    pub(crate) fn recv(&mut self) -> io::Result<Message> {
        let mut chunk = [0u8; 65536];
        loop {
            let decoded =
                decode(&self.buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            if let Some((message, used)) = decoded {
                self.buf.drain(..used);
                return Ok(message);
            }
            self.stop.check()?;
            let n = match self.stream.read(&mut chunk) {
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(n) => n,
                Err(e) if is_wait(&e) => continue,
                Err(e) => return Err(e),
            };
            self.buf.extend_from_slice(&chunk[..n]);
        }
    }
}
