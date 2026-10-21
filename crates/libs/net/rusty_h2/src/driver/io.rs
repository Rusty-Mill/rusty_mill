//! The task that owns the transport: it writes what the shared state has
//! queued, reads and applies frames, and decides when the connection ends.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use rusty_tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use super::shared::{lock, Handle, OUT_HIGH};
use super::Error;
use crate::error::{ErrorCode, H2Error};
use crate::frame::header::{FrameHeader, FRAME_HEADER_LEN};
use crate::frame::Frame;
use crate::CONNECTION_PREFACE;

const READ_CHUNK: usize = 64 * 1024;

/// Errors a transport reports when the peer is gone: it closed, or reset the
/// connection. Windows says `ConnectionAborted` where Unix says
/// `ConnectionReset` or `BrokenPipe`.
fn peer_gone(e: &std::io::Error) -> bool {
    use std::io::ErrorKind::{BrokenPipe, ConnectionAborted, ConnectionReset, NotConnected};
    matches!(
        e.kind(),
        NotConnected | BrokenPipe | ConnectionReset | ConnectionAborted
    )
}

/// The connection's task: spawn it, or await it to run the connection to
/// its end. Resolves when the connection closes.
#[must_use = "the connection only makes progress while this is polled"]
pub struct ConnectionDriver<T> {
    pub(super) shared: Handle,
    pub(super) io: T,
    pub(super) rbuf: Vec<u8>,
    pub(super) rpos: usize,
    /// Bytes of the client preface still to be checked (server only).
    pub(super) preface_seen: usize,
    pub(super) preface_needed: bool,
    pub(super) eof: bool,
    /// The outcome, once decided; the driver then only flushes and stops.
    pub(super) finishing: Option<Result<(), Error>>,
    pub(super) max_frame: u32,
    /// Reading paused because too much output was waiting.
    pub(super) stalled: bool,
    /// Bytes were written since the transport was last flushed.
    pub(super) needs_flush: bool,
    /// Output taken from the shared state and being written.
    pub(super) wbuf: Vec<u8>,
    pub(super) wpos: usize,
    /// Bytes the transport has accepted, and of those, flushed.
    pub(super) accepted: u64,
    pub(super) flushed: u64,
}

impl<T> Drop for ConnectionDriver<T> {
    /// A driver that is dropped, aborted or never polled ends the
    /// connection: every handle and pending operation is woken and fails.
    fn drop(&mut self) {
        lock(&self.shared).close_all();
    }
}

fn parse(buf: &[u8], max: u32) -> Result<Option<(Frame, usize)>, H2Error> {
    if buf.len() < FRAME_HEADER_LEN {
        return Ok(None);
    }
    let header = FrameHeader::decode(&buf[..FRAME_HEADER_LEN])?;
    if header.length > max {
        return Err(H2Error::Connection(
            ErrorCode::FrameSizeError,
            "frame larger than SETTINGS_MAX_FRAME_SIZE",
        ));
    }
    let total = FRAME_HEADER_LEN + header.length as usize;
    if buf.len() < total {
        return Ok(None);
    }
    Ok(Some((
        Frame::decode(&header, &buf[FRAME_HEADER_LEN..total])?,
        total,
    )))
}

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> ConnectionDriver<T> {
    /// Writes queued output until the transport would block. `Ready(Err)`
    /// is a transport failure. The transport is only ever polled with the
    /// shared lock released: registering its waker may destroy a previous
    /// one, whose owner can lock the shared state.
    fn flush(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        loop {
            if self.wpos == self.wbuf.len() {
                self.wbuf.clear();
                self.wpos = 0;
                let mut sh = lock(&self.shared);
                if sh.out.is_empty() {
                    sh.inflight = 0;
                    break;
                }
                std::mem::swap(&mut sh.out, &mut self.wbuf);
                sh.inflight = self.wbuf.len();
            }
            match Pin::new(&mut self.io).poll_write(cx, &self.wbuf[self.wpos..]) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(Error::Io(e))),
                Poll::Ready(Ok(0)) => {
                    return Poll::Ready(Err(Error::Io(std::io::ErrorKind::WriteZero.into())))
                }
                Poll::Ready(Ok(n)) => {
                    self.wpos += n;
                    self.accepted += n as u64;
                    self.needs_flush = true;
                    let mut sh = lock(&self.shared);
                    sh.inflight = self.wbuf.len() - self.wpos;
                    if sh.backlog() < OUT_HIGH / 2 {
                        sh.wake_waiters();
                    }
                }
            }
        }
        // Queued bytes are only with the transport; a buffered one keeps
        // them until it is flushed.
        if self.needs_flush {
            match Pin::new(&mut self.io).poll_flush(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(Error::Io(e))),
                Poll::Ready(Ok(())) => {
                    self.needs_flush = false;
                    self.flushed = self.accepted;
                }
            }
        }
        Poll::Ready(Ok(()))
    }

    /// Reads and applies what is available. Returns once the transport
    /// would block, input ends, or a fatal error occurs.
    fn read(&mut self, cx: &mut Context<'_>) -> Result<(), Error> {
        loop {
            self.consume_frames()?;
            if self.eof {
                return Ok(());
            }
            self.stalled = lock(&self.shared).backlog() >= OUT_HIGH;
            if self.stalled {
                return Ok(());
            }
            let mut chunk = [0u8; READ_CHUNK];
            let mut buf = ReadBuf::new(&mut chunk);
            match Pin::new(&mut self.io).poll_read(cx, &mut buf) {
                Poll::Pending => return Ok(()),
                Poll::Ready(Err(e)) if self.reset_after_goodbye(&e) => {
                    // As if the input had ended: everything it carried has
                    // been applied.
                    self.eof = true;
                    return Ok(());
                }
                Poll::Ready(Err(e)) => return Err(Error::Io(e)),
                Poll::Ready(Ok(())) if buf.filled().is_empty() => {
                    self.eof = true;
                    return Ok(());
                }
                Poll::Ready(Ok(())) => self.rbuf.extend_from_slice(buf.filled()),
            }
        }
    }

    fn consume_frames(&mut self) -> Result<(), Error> {
        if self.preface_needed {
            let have = &self.rbuf[self.rpos..];
            let n = have.len().min(CONNECTION_PREFACE.len());
            if have[..n] != CONNECTION_PREFACE[..n] {
                return Err(Error::Protocol(H2Error::Connection(
                    ErrorCode::ProtocolError,
                    "the connection did not start with the HTTP/2 preface",
                )));
            }
            if n < CONNECTION_PREFACE.len() {
                return Ok(());
            }
            self.rpos += n;
            self.preface_needed = false;
            self.preface_seen = n;
            lock(&self.shared).conn.mark_preface_seen();
        }
        // One lock for everything this read delivered: the application sees
        // the frames of a read as a unit, never half applied.
        let mut sh = lock(&self.shared);
        loop {
            match parse(&self.rbuf[self.rpos..], self.max_frame) {
                Ok(Some((frame, used))) => {
                    self.rpos += used;
                    sh.on_frame(frame)?;
                }
                Ok(None) => break,
                Err(e) => return Err(Error::Protocol(e)),
            }
        }
        drop(sh);
        if self.rpos > 0 && self.rpos == self.rbuf.len() {
            self.rbuf.clear();
            self.rpos = 0;
        } else if self.rpos > READ_CHUNK {
            self.rbuf.drain(..self.rpos);
            self.rpos = 0;
        }
        Ok(())
    }

    /// Decides whether the connection should now end.
    fn decide(&mut self) {
        if self.finishing.is_some() {
            return;
        }
        let mut sh = lock(&self.shared);
        if self.eof {
            let clean = sh.active == 0;
            sh.close_all();
            self.finishing = Some(if clean {
                Ok(())
            } else {
                Err(Error::Io(std::io::ErrorKind::UnexpectedEof.into()))
            });
            return;
        }
        let wound_down = sh.closing || sh.conn.goaway_received().is_some();
        if wound_down && sh.active == 0 && sh.accept_queue.is_empty() {
            sh.send_goaway(ErrorCode::NoError);
            sh.close_all();
            self.finishing = Some(Ok(()));
        }
    }

    /// Whether everything read has been applied and nothing is half received:
    /// no frame is cut short, and no header block is waiting for its
    /// CONTINUATION. (Open calls are counted apart: such a block opens none.)
    fn input_complete(&self) -> bool {
        self.rpos == self.rbuf.len() && !lock(&self.shared).conn.header_block_pending()
    }

    /// Whether `e` from a write, flush or shutdown is only the peer having hung
    /// up first: it reached the end of its input or announced GOAWAY, the error
    /// says it is gone, and its input was whole. A peer that left in the middle
    /// of a frame or header block did not say goodbye.
    fn peer_closed_first(&self, e: &std::io::Error) -> bool {
        peer_gone(e)
            && (self.eof || lock(&self.shared).conn.goaway_received().is_some())
            && self.input_complete()
    }

    /// Whether a failed write or flush lost nothing the peer needed: it is the
    /// peer having hung up first, and every frame carrying application data
    /// (headers, body, trailers, resets) was already flushed, so only
    /// housekeeping like SETTINGS, acks or the closing GOAWAY was lost. A
    /// GOAWAY does not acknowledge delivery (RFC 9113 §6.8), so an
    /// undelivered response is a failure.
    fn only_housekeeping_was_lost(&self, e: &Error) -> bool {
        let Error::Io(io) = e else { return false };
        self.peer_closed_first(io) && self.flushed >= lock(&self.shared).app_end
    }

    /// Whether a failed read is only the peer resetting the connection after
    /// it said goodbye, so it can end the input like EOF. The same terminal
    /// state `decide` needs for a clean end: the peer announced GOAWAY, its
    /// input was whole, and no call is open; and no application data of ours
    /// is still undelivered (`flushed >= app_end`), since a generic transport
    /// does not promise that its writes fail too. Any other error, an earlier
    /// failure, or a reset without a goodbye still fails the connection.
    fn reset_after_goodbye(&self, e: &std::io::Error) -> bool {
        if !peer_gone(e) || !self.input_complete() {
            return false;
        }
        let sh = lock(&self.shared);
        sh.conn.goaway_received().is_some() && sh.active == 0 && self.flushed >= sh.app_end
    }

    fn fail(&mut self, e: Error) {
        let mut sh = lock(&self.shared);
        if let Error::Protocol(H2Error::Connection(code, _)) = &e {
            sh.send_goaway(*code);
        }
        sh.close_all();
        self.finishing = Some(Err(e));
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Future for ConnectionDriver<T> {
    type Output = Result<(), Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = &mut *self;
        {
            let mut sh = lock(&this.shared);
            let old = sh.driver_waker.replace(cx.waker().clone());
            sh.trash.extend(old);
        }
        if this.finishing.is_none() {
            if let Err(e) = this.read(cx) {
                this.fail(e);
            }
            this.decide();
        }
        // Write after reading: what the frames just applied asked to send
        // (acks, window updates) goes out in the same turn.
        let written = this.flush(cx);
        if let Poll::Ready(Err(e)) = written {
            // An earlier failure stands; otherwise a failed final write or
            // flush fails the connection, unless the peer had already gone.
            let benign = this.only_housekeeping_was_lost(&e);
            let outcome = match this.finishing.take() {
                Some(Err(first)) => Err(first),
                Some(Ok(())) if benign => Ok(()),
                _ => {
                    lock(&this.shared).close_all();
                    Err(e)
                }
            };
            return Poll::Ready(outcome);
        }
        if this.finishing.is_some() && matches!(written, Poll::Ready(Ok(()))) {
            // Finish only once the transport has shut down; a failure to do
            // so is reported unless the connection had already failed.
            let closed = match Pin::new(&mut this.io).poll_shutdown(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(r) => r,
            };
            let outcome = this.finishing.take().unwrap_or(Ok(()));
            return Poll::Ready(match (outcome, closed) {
                // A peer that already hung up leaves nothing to shut down.
                (Ok(()), Err(e)) if !this.peer_closed_first(&e) => Err(Error::Io(e)),
                (outcome, _) => outcome,
            });
        }
        // Output that kept us from reading has now gone out: read again.
        if this.stalled && written.is_ready() {
            cx.waker().wake_by_ref();
        }
        Poll::Pending
    }
}
