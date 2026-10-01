//! The async server adapter (feature `rusty-tokio`): [`AsyncTlsServerStream`]
//! drives the same sans-IO `rustls::ServerConnection`
//! [`crate::TlsServerStream`] does, but over `rusty_tokio`'s readiness-based
//! `AsyncRead`/`AsyncWrite` instead of a blocking `Read + Write` — the
//! server-side counterpart to [`crate::AsyncTlsStream`], built via
//! [`TlsAcceptor::accept_async`](crate::TlsAcceptor::accept_async).

use std::io::{self, Read, Write};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{ready, Context, Poll};

use rustls::{ServerConfig, ServerConnection};
use rusty_tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::error::Error;

/// A TLS server connection layered over any `rusty_tokio`
/// `AsyncRead + AsyncWrite` stream — the async counterpart to
/// [`crate::TlsServerStream`]. Built via
/// [`TlsAcceptor::accept_async`](crate::TlsAcceptor::accept_async); keeps
/// rustls types out of the public API the same way the other adapters do.
pub struct AsyncTlsServerStream<S> {
    conn: ServerConnection,
    io: S,
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncTlsServerStream<S> {
    pub(crate) fn new(config: Arc<ServerConfig>, io: S) -> Result<Self, Error> {
        let conn = ServerConnection::new(config)?;
        Ok(Self { conn, io })
    }

    /// Whether the TLS handshake has not yet completed.
    pub fn is_handshaking(&self) -> bool {
        self.conn.is_handshaking()
    }

    /// Borrow the underlying stream.
    pub fn get_ref(&self) -> &S {
        &self.io
    }

    /// Mutably borrow the underlying stream. See
    /// [`TlsStream::get_mut`](crate::TlsStream::get_mut)'s docs — the same
    /// caution applies.
    pub fn get_mut(&mut self) -> &mut S {
        &mut self.io
    }

    /// Consume `self`, returning the underlying stream. See
    /// [`TlsStream::into_inner`](crate::TlsStream::into_inner)'s docs.
    pub fn into_inner(self) -> S {
        self.io
    }

    /// Completes the TLS handshake (or fails), without requiring the caller
    /// to send or expect application data first. The async counterpart to
    /// [`TlsServerStream::complete_handshake`](crate::TlsServerStream::complete_handshake).
    pub async fn complete_handshake(&mut self) -> Result<(), Error> {
        std::future::poll_fn(|cx| self.poll_handshake(cx)).await?;
        Ok(())
    }

    /// The protocol negotiated via ALPN, if any. See
    /// [`TlsStream::negotiated_alpn_protocol`](crate::TlsStream::negotiated_alpn_protocol)'s
    /// docs for when this is (and isn't) populated. Set on the acceptor via
    /// [`TlsAcceptor::new_with_alpn`](crate::TlsAcceptor::new_with_alpn) —
    /// nothing extra to configure here, since `accept_async` reuses
    /// whatever `ServerConfig` the acceptor was built with.
    pub fn negotiated_alpn_protocol(&self) -> Option<&[u8]> {
        self.conn.alpn_protocol()
    }

    /// Drive the handshake until it completes, then write out its last
    /// flight. Stops as soon as `is_handshaking()` goes false: reading on
    /// past that point (`wants_read()` stays true on an idle connection)
    /// would wait for bytes the peer has no reason to send, which is the
    /// lost wakeup `tests/async_lost_wakeup.rs` pins. The condition the
    /// sync adapter's `rustls::Connection::complete_io` uses too, and the
    /// same `UnexpectedEof` if the peer closes mid-handshake.
    fn poll_handshake(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        while self.conn.is_handshaking() {
            ready!(self.poll_write_tls(cx))?;
            if !self.conn.is_handshaking() {
                break;
            }
            if ready!(self.poll_read_tls(cx))? == 0 {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "TLS handshake: the peer closed the connection",
                )));
            }
        }
        self.poll_write_tls(cx)
    }

    /// Write out every pending TLS record.
    fn poll_write_tls(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        while self.conn.wants_write() {
            let mut adapter = PollAdapter {
                io: Pin::new(&mut self.io),
                cx,
            };
            match self.conn.write_tls(&mut adapter) {
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Poll::Pending,
                Err(e) => return Poll::Ready(Err(e)),
            }
        }
        Poll::Ready(Ok(()))
    }

    /// Read one chunk of TLS input and process it; `Ready(Ok(0))` at the
    /// underlying stream's EOF, which rustls' reader then reports (a clean
    /// `close_notify` as `Ok(0)`, otherwise `UnexpectedEof`).
    fn poll_read_tls(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<usize>> {
        let mut adapter = PollAdapter {
            io: Pin::new(&mut self.io),
            cx,
        };
        match self.conn.read_tls(&mut adapter) {
            Ok(n) => {
                if let Err(e) = self.conn.process_new_packets() {
                    return Poll::Ready(Err(io::Error::new(io::ErrorKind::InvalidData, e)));
                }
                Poll::Ready(Ok(n))
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Poll::Pending,
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRead for AsyncTlsServerStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        loop {
            match this.conn.reader().read(buf.unfilled_mut()) {
                Ok(n) => {
                    buf.advance(n);
                    return Poll::Ready(Ok(()));
                }
                // No plaintext yet: send anything owed (a handshake flight,
                // a key update), then wait for the peer's next records.
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    ready!(this.poll_write_tls(cx))?;
                    ready!(this.poll_read_tls(cx))?;
                }
                Err(e) => return Poll::Ready(Err(e)),
            }
        }
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncWrite for AsyncTlsServerStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        // Finish the handshake before accepting new plaintext — the same
        // ordering `rustls::Stream::write`'s `complete_prior_io` enforces
        // for the sync adapter.
        ready!(this.poll_handshake(cx))?;
        let n = this.conn.writer().write(buf)?;
        // Best-effort send: `Pending` here doesn't mean the `n` bytes
        // weren't accepted, only that they're queued for the next poll
        // to push out; an error resurfaces on that poll.
        let _ = this.poll_write_tls(cx);
        Poll::Ready(Ok(n))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.conn.writer().flush()?;
        ready!(this.poll_handshake(cx))?;
        this.poll_write_tls(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.conn.send_close_notify();
        ready!(this.poll_write_tls(cx))?;
        Pin::new(&mut this.io).poll_shutdown(cx)
    }
}

/// Adapts a `Pin<&mut S> where S: AsyncRead + AsyncWrite` plus a poll
/// `Context` into `std::io::Read`/`Write`, so rustls' synchronous
/// `read_tls`/`write_tls` can drive it. See
/// [`crate::AsyncTlsStream`]'s identically-shaped private type for the full
/// explanation — kept as its own copy here rather than shared, matching
/// this crate's existing convention of two structurally-similar-but-distinct
/// adapters (see `ARCHITECTURE.md`'s Structure section) rather than forcing
/// a shared abstraction across client and server.
struct PollAdapter<'a, 'cx, S> {
    io: Pin<&'a mut S>,
    cx: &'a mut Context<'cx>,
}

impl<S: AsyncRead> Read for PollAdapter<'_, '_, S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut read_buf = ReadBuf::new(buf);
        match self.io.as_mut().poll_read(self.cx, &mut read_buf) {
            Poll::Ready(Ok(())) => Ok(read_buf.filled().len()),
            Poll::Ready(Err(e)) => Err(e),
            Poll::Pending => Err(io::ErrorKind::WouldBlock.into()),
        }
    }
}

impl<S: AsyncWrite> Write for PollAdapter<'_, '_, S> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.io.as_mut().poll_write(self.cx, buf) {
            Poll::Ready(Ok(n)) => Ok(n),
            Poll::Ready(Err(e)) => Err(e),
            Poll::Pending => Err(io::ErrorKind::WouldBlock.into()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.io.as_mut().poll_flush(self.cx) {
            Poll::Ready(Ok(())) => Ok(()),
            Poll::Ready(Err(e)) => Err(e),
            Poll::Pending => Err(io::ErrorKind::WouldBlock.into()),
        }
    }
}
