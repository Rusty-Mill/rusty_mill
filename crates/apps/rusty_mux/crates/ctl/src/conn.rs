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
/// a timed-out send leaves the connection unusable, so writes time out
/// once, at the deadline (see [`write_timeout`]); a stop with no deadline
/// leaves them unbounded.
#[cfg(any(unix, windows))]
pub(crate) fn connect(path: &Path, stop: &Stop) -> io::Result<UnixStream> {
    if !stop.is_bounded() {
        return connect_blocking(path);
    }
    let stream = connect_bounded(path, stop)?;
    stream.set_read_timeout(Some(POLL))?;
    #[cfg(unix)]
    stream.set_write_timeout(Some(POLL))?;
    #[cfg(windows)]
    stream.set_write_timeout(write_timeout(stop))?;
    Ok(stream)
}

/// The Windows write timeout for `stop`: the time left to its deadline,
/// at least 1 ms (Winsock reads 0 as "never"), or none without a deadline.
#[cfg_attr(not(windows), allow(dead_code))]
fn write_timeout(stop: &Stop) -> Option<std::time::Duration> {
    stop.left()
        .map(|left| left.max(std::time::Duration::from_millis(1)))
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
                // Winsock's timed-out send may have sent part of the
                // frame: the connection is done, so report the stop.
                Err(e) if cfg!(windows) && e.kind() == io::ErrorKind::TimedOut => {
                    return Err(self.stop.check().err().unwrap_or(e));
                }
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
        loop {
            if let Some(message) = self.poll()? {
                return Ok(message);
            }
        }
    }

    /// As [`Conn::recv`], but `None` once a read times out with no whole
    /// message, so a caller can do other work between reads.
    pub(crate) fn poll(&mut self) -> io::Result<Option<Message>> {
        let mut chunk = [0u8; 65536];
        loop {
            let decoded =
                decode(&self.buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            if let Some((message, used)) = decoded {
                self.buf.drain(..used);
                return Ok(Some(message));
            }
            self.stop.check()?;
            let n = match self.stream.read(&mut chunk) {
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(n) => n,
                Err(e) if is_wait(&e) => return Ok(None),
                Err(e) => return Err(e),
            };
            self.buf.extend_from_slice(&chunk[..n]);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn a_write_timeout_needs_a_deadline() {
        assert_eq!(write_timeout(&Stop::default().or_when(|| false)), None);
        let left = write_timeout(&Stop::after(Duration::from_secs(5))).expect("a deadline");
        assert!(left > Duration::from_secs(4) && left <= Duration::from_secs(5));
    }

    #[test]
    fn a_passed_deadline_still_times_out() {
        // 0 would be Winsock's "never", and std refuses it.
        assert_eq!(
            write_timeout(&Stop::after(Duration::ZERO)),
            Some(Duration::from_millis(1))
        );
    }

    /// A stream whose every write times out, counting the attempts.
    struct Stuck(usize);

    impl Read for Stuck {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Ok(0)
        }
    }

    impl Write for Stuck {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            self.0 += 1;
            Err(io::ErrorKind::TimedOut.into())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// On Windows a timed-out write ends the send at once; elsewhere it is
    /// a poll tick, retried until the stop.
    #[test]
    fn a_timed_out_write_is_retried_only_off_windows() {
        let stop = Stop::after(Duration::from_millis(200));
        let mut conn = Conn::new(Stuck(0), stop);
        let err = conn
            .send(&Message::Shutdown { req: 1 })
            .expect_err("every write times out");
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        if cfg!(windows) {
            assert_eq!(conn.stream.0, 1);
        } else {
            assert!(conn.stream.0 > 1);
        }
    }
}
