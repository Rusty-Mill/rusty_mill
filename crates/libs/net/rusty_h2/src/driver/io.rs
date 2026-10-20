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
    /// is a transport failure.
    fn flush(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        loop {
            let mut sh = lock(&self.shared);
            if sh.out_pos == sh.out.len() {
                sh.out.clear();
                sh.out_pos = 0;
                return Poll::Ready(Ok(()));
            }
            let start = sh.out_pos;
            let polled = Pin::new(&mut self.io).poll_write(cx, &sh.out[start..]);
            match polled {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(Error::Io(e))),
                Poll::Ready(Ok(0)) => {
                    return Poll::Ready(Err(Error::Io(std::io::ErrorKind::WriteZero.into())))
                }
                Poll::Ready(Ok(n)) => {
                    sh.out_pos += n;
                    if sh.backlog() < OUT_HIGH / 2 {
                        sh.wake_waiters();
                    }
                }
            }
        }
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
        loop {
            match parse(&self.rbuf[self.rpos..], self.max_frame) {
                Ok(Some((frame, used))) => {
                    self.rpos += used;
                    lock(&self.shared).on_frame(frame)?;
                }
                Ok(None) => break,
                Err(e) => return Err(Error::Protocol(e)),
            }
        }
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
        lock(&this.shared).driver_waker = Some(cx.waker().clone());
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
            if this.finishing.is_none() {
                this.fail(e);
            }
            return Poll::Ready(this.finishing.take().unwrap_or(Ok(())));
        }
        if this.finishing.is_some() && matches!(written, Poll::Ready(Ok(()))) {
            let _ = Pin::new(&mut this.io).poll_shutdown(cx);
            return Poll::Ready(this.finishing.take().unwrap_or(Ok(())));
        }
        // Output that kept us from reading has now gone out: read again.
        if this.stalled && written.is_ready() {
            cx.waker().wake_by_ref();
        }
        Poll::Pending
    }
}
