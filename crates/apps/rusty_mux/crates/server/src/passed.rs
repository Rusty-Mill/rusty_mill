//! Descriptors a client passes with a launch (C75). Only a Unix socket the
//! client opened itself carries them, on Linux, as `SCM_RIGHTS` sent with
//! the frame they belong to. They are owned from the moment they arrive,
//! so every refusal and failed spawn closes them by drop.

#[cfg(target_os = "linux")]
pub(crate) use linux::Reader;

/// One passed descriptor. None exist off Linux.
#[cfg(target_os = "linux")]
pub(crate) type Fd = std::os::fd::OwnedFd;
#[cfg(not(target_os = "linux"))]
pub(crate) enum Fd {}

/// What came with one frame.
#[derive(Default)]
pub(crate) enum Passed {
    #[default]
    None,
    Fds(Vec<Fd>),
    /// Descriptors came that can't be trusted to be the ones sent: more
    /// than fit, or not at the start of a frame. Those that came are
    /// already closed.
    Bad(&'static str),
}

/// Descriptors a read brought, and where in the stream that read was.
pub(crate) struct Arrival {
    pub(crate) fds: Vec<Fd>,
    pub(crate) truncated: bool,
    /// The stream offsets the read covered: `[start, end)`.
    pub(crate) start: u64,
    pub(crate) end: u64,
}

/// A connection's read half that can bring descriptors.
pub(crate) trait Descriptors {
    /// The descriptors the last read brought, and whether more came than
    /// fit; `None` if it brought none.
    fn take(&mut self) -> Option<(Vec<Fd>, bool)> {
        None
    }
}

impl Descriptors for rusty_tokio::io::OwnedUnixReadHalf {}
impl Descriptors for rusty_tokio::io::OwnedReadHalf {}
#[cfg(target_os = "linux")]
impl Descriptors
    for rusty_tokio::io::SplitReadHalf<rusty_tls::AsyncTlsServerStream<rusty_tokio::io::TcpStream>>
{
}

/// Which descriptors, if any, belong to the frame at stream offsets
/// `[start, end)`. The kernel ends a read with the segment that carried
/// descriptors, and ctl sends a frame and its descriptors in one
/// `sendmsg`, so they belong to the frame holding the read's last byte,
/// which must start inside that read. Frames before it get none.
pub(crate) fn claim(pending: &mut Option<Arrival>, start: u64, end: u64) -> Passed {
    let Some(arrival) = pending.as_ref() else {
        return Passed::None;
    };
    if end < arrival.end {
        return Passed::None;
    }
    let Some(arrival) = pending.take() else {
        return Passed::None;
    };
    if arrival.truncated {
        return Passed::Bad("more descriptors than a launch takes");
    }
    if start < arrival.start {
        return Passed::Bad("descriptors inside a frame");
    }
    Passed::Fds(arrival.fds)
}

#[cfg(target_os = "linux")]
mod linux {
    use std::io;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::pin::Pin;
    use std::task::{Context, Poll, ready};

    use rusty_tokio::io::{AsyncRead, Interest, OwnedUnixReadHalf, ReadBuf};

    use super::{Descriptors, Fd};

    /// A Unix socket's read half that takes descriptors as it reads.
    pub(crate) struct Reader {
        half: OwnedUnixReadHalf,
        got: Option<(Vec<Fd>, bool)>,
    }

    impl Reader {
        pub(crate) fn new(half: OwnedUnixReadHalf) -> Self {
            Self { half, got: None }
        }
    }

    impl Descriptors for Reader {
        fn take(&mut self) -> Option<(Vec<Fd>, bool)> {
            self.got.take()
        }
    }

    impl AsyncRead for Reader {
        fn poll_read(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            let this = self.get_mut();
            let stream = this.half.as_ref();
            loop {
                ready!(stream.poll_read_ready(cx))?;
                let read = stream.try_io(Interest::READABLE, || {
                    rusty_libc::socket::recv_with_fds(
                        stream.as_raw_fd(),
                        buf.initialize_unfilled(),
                        0,
                    )
                    .map_err(|e| io::Error::from_raw_os_error(e.0))
                });
                let received = match read {
                    Ok(received) => received,
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => return Poll::Ready(Err(e)),
                };
                // SAFETY: the kernel just gave us these descriptors and
                // nothing else holds them; each is wrapped exactly once.
                let fds: Vec<Fd> = received
                    .fds()
                    .iter()
                    .map(|&fd| unsafe { OwnedFd::from_raw_fd(fd) })
                    .collect();
                if !fds.is_empty() || received.truncated {
                    this.got = Some((fds, received.truncated));
                }
                buf.advance(received.bytes);
                return Poll::Ready(Ok(()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arrival(start: u64, end: u64, truncated: bool) -> Option<Arrival> {
        Some(Arrival {
            fds: Vec::new(),
            truncated,
            start,
            end,
        })
    }

    #[test]
    fn the_frame_ending_the_read_claims_them() {
        let mut pending = arrival(10, 40, false);
        assert!(matches!(claim(&mut pending, 10, 25), Passed::None));
        assert!(pending.is_some(), "an earlier frame leaves them");
        assert!(matches!(claim(&mut pending, 25, 40), Passed::Fds(_)));
        assert!(pending.is_none());
    }

    #[test]
    fn a_frame_running_past_the_read_claims_them() {
        let mut pending = arrival(0, 100, false);
        assert!(matches!(claim(&mut pending, 0, 300), Passed::Fds(_)));
    }

    #[test]
    fn a_frame_begun_before_the_read_is_refused() {
        let mut pending = arrival(10, 40, false);
        assert!(matches!(claim(&mut pending, 5, 40), Passed::Bad(_)));
        assert!(pending.is_none(), "closed, not kept");
    }

    #[test]
    fn truncated_is_refused() {
        let mut pending = arrival(0, 10, true);
        assert!(matches!(claim(&mut pending, 0, 10), Passed::Bad(_)));
    }

    /// A second arrival before the first is claimed closes the first, and
    /// the frame that began before the second read is then refused.
    #[cfg(target_os = "linux")]
    #[test]
    fn an_unclaimed_arrival_replaced_is_closed() {
        use std::io::Read;
        use std::os::unix::net::UnixStream;

        let (sent, mut peer) = UnixStream::pair().expect("socket pair");
        let mut pending = Some(Arrival {
            fds: vec![Fd::from(sent)],
            truncated: false,
            start: 0,
            end: 10,
        });
        assert!(matches!(claim(&mut pending, 0, 5), Passed::None), "not yet");
        pending = arrival(10, 30, false);
        let mut byte = [0u8; 1];
        assert_eq!(peer.read(&mut byte).expect("read"), 0, "first is closed");
        assert!(matches!(claim(&mut pending, 0, 30), Passed::Bad(_)));
        assert!(pending.is_none());
    }

    #[test]
    fn nothing_pending_is_none() {
        assert!(matches!(claim(&mut None, 0, 10), Passed::None));
    }
}
