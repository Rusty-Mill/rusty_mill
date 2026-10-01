//! The async adapter (feature `rusty-tokio`): [`AsyncTlsStream`] drives the
//! same sans-IO `rustls::ClientConnection` [`crate::TlsStream`] does, but
//! over `rusty_tokio`'s readiness-based `AsyncRead`/`AsyncWrite` instead of
//! a blocking `Read + Write`. Per the mission this crate implements, the
//! async adapter lives here, behind this feature — `rusty_tokio` itself
//! stays TLS-free.

use std::io::{self, Read, Write};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{ready, Context, Poll};

use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use rustls::{ClientConfig, ClientConnection, HandshakeKind};
use rusty_tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::error::Error;
use crate::trust::{
    build_client_config, build_client_config_with_alpn, build_client_config_with_identity,
    TrustPolicy,
};

/// A TLS client connection layered over any `rusty_tokio`
/// `AsyncRead + AsyncWrite` stream (typically [`rusty_tokio::io::TcpStream`]).
///
/// The async counterpart to [`crate::TlsStream`] — same constructor shape,
/// same [`TrustPolicy`], same rule that `S` must already be connected. See
/// [`crate::TlsStream`]'s docs for why.
pub struct AsyncTlsStream<S> {
    conn: ClientConnection,
    io: S,
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncTlsStream<S> {
    /// Wrap `io` in a TLS client connection to `server_name`, trusted
    /// according to `policy`. Performs no I/O itself — the handshake runs
    /// lazily, driven by the first `poll_read`/`poll_write`, exactly like
    /// [`crate::TlsStream::new`].
    pub fn new(io: S, server_name: &str, policy: &TrustPolicy) -> Result<Self, Error> {
        let config = build_client_config(policy)?;
        let name = ServerName::try_from(server_name.to_string())
            .map_err(|_| Error::InvalidServerName(server_name.to_string()))?;
        let conn = ClientConnection::new(config, name)?;
        Ok(Self { conn, io })
    }

    /// Wrap `io` in a TLS client connection using an already-built
    /// `config` — what [`TlsConnector`](crate::TlsConnector) uses so
    /// repeated connections share the same session-resumption cache,
    /// rather than each getting a fresh, empty one.
    pub(crate) fn from_config(
        config: Arc<ClientConfig>,
        io: S,
        server_name: &str,
    ) -> Result<Self, Error> {
        let name = ServerName::try_from(server_name.to_string())
            .map_err(|_| Error::InvalidServerName(server_name.to_string()))?;
        let conn = ClientConnection::new(config, name)?;
        Ok(Self { conn, io })
    }

    /// Like [`AsyncTlsStream::new`], but presents `client_cert_chain_der`
    /// (leaf first, DER-encoded) and `client_key_der` (the leaf's private
    /// key — PKCS#8, PKCS#1, or SEC1, auto-detected) to the server as a
    /// client certificate — the async counterpart to
    /// [`TlsStream::new_with_client_identity`](crate::TlsStream::new_with_client_identity).
    pub fn new_with_client_identity(
        io: S,
        server_name: &str,
        policy: &TrustPolicy,
        client_cert_chain_der: Vec<Vec<u8>>,
        client_key_der: Vec<u8>,
    ) -> Result<Self, Error> {
        let cert_chain: Vec<CertificateDer<'static>> = client_cert_chain_der
            .into_iter()
            .map(CertificateDer::from)
            .collect();
        let key = PrivateKeyDer::try_from(client_key_der)
            .map_err(|reason| Error::InvalidPrivateKey(reason.to_string()))?;
        let config = build_client_config_with_identity(policy, cert_chain, key)?;
        let name = ServerName::try_from(server_name.to_string())
            .map_err(|_| Error::InvalidServerName(server_name.to_string()))?;
        let conn = ClientConnection::new(config, name)?;
        Ok(Self { conn, io })
    }

    /// Like [`AsyncTlsStream::new`], but offers `alpn_protocols` (each
    /// entry a wire-format protocol ID, e.g. `b"h2"`) during the handshake
    /// — the async counterpart to
    /// [`TlsStream::new_with_alpn`](crate::TlsStream::new_with_alpn).
    pub fn new_with_alpn(
        io: S,
        server_name: &str,
        policy: &TrustPolicy,
        alpn_protocols: Vec<Vec<u8>>,
    ) -> Result<Self, Error> {
        let config = build_client_config_with_alpn(policy, alpn_protocols)?;
        let name = ServerName::try_from(server_name.to_string())
            .map_err(|_| Error::InvalidServerName(server_name.to_string()))?;
        let conn = ClientConnection::new(config, name)?;
        Ok(Self { conn, io })
    }

    /// The protocol negotiated via ALPN, if any. See
    /// [`TlsStream::negotiated_alpn_protocol`](crate::TlsStream::negotiated_alpn_protocol)'s
    /// docs for when this is (and isn't) populated.
    pub fn negotiated_alpn_protocol(&self) -> Option<&[u8]> {
        self.conn.alpn_protocol()
    }

    /// Whether this connection resumed a previous TLS session rather than
    /// performing a full handshake. See
    /// [`TlsStream::resumed_session`](crate::TlsStream::resumed_session)'s
    /// docs for when this is (and isn't) meaningful.
    pub fn resumed_session(&self) -> bool {
        matches!(self.conn.handshake_kind(), Some(HandshakeKind::Resumed))
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

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRead for AsyncTlsStream<S> {
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

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncWrite for AsyncTlsStream<S> {
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
/// `read_tls`/`write_tls` can drive it: a `Poll::Pending` from the
/// underlying stream becomes `io::ErrorKind::WouldBlock`, which
/// `AsyncTlsStream::poll_write_tls`/`poll_read_tls` translate back into `Poll::Pending`
/// for its own caller. The waker registration that makes that `Pending`
/// meaningful already happened inside the `poll_read`/`poll_write` call
/// below, before it returned — nothing extra to wire up here.
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
