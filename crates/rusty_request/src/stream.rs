//! The connection type a request is actually sent over: a plain TCP
//! socket for `http://`, or the same socket wrapped in TLS (via
//! `rusty_tls`) for `https://`. An enum rather than making the send path
//! generic over the transport -- nothing else in this crate is generic,
//! and threading a type parameter through `pool`/`http1`/`client` for a
//! two-variant choice would be more machinery than the choice itself.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use rusty_tls::AsyncTlsStream;
use rusty_tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// The socket `Conn` actually wraps: `rusty_tokio`'s own `TcpStream`, or
/// (only with the `tokio` feature compiled in, and only when the task
/// that dialed it was running on a real tokio runtime -- see `crate::rt`)
/// a real tokio `TcpStream` bridged through
/// [`crate::tokio_compat::TokioIo`] so it still satisfies `rusty_tls`/
/// `rusty_http`'s `rusty_tokio`-shaped generic bounds. Which variant a
/// given connection is gets fixed at dial time by the runtime that dialed
/// it; every read/write after that just forwards to that variant, so
/// `Conn`/`AsyncTlsStream`/`AsyncTransport` never have to know. An enum
/// (not a feature-selected type alias, as it used to be) is what lets
/// both backends coexist in one build, which is what makes the `tokio`
/// feature additive rather than a global switch.
pub(crate) enum RawStream {
    RustyTokio(rusty_tokio::io::TcpStream),
    #[cfg(feature = "tokio")]
    Tokio(crate::tokio_compat::TokioIo<tokio::net::TcpStream>),
}

impl AsyncRead for RawStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            RawStream::RustyTokio(s) => Pin::new(s).poll_read(cx, buf),
            #[cfg(feature = "tokio")]
            RawStream::Tokio(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for RawStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            RawStream::RustyTokio(s) => Pin::new(s).poll_write(cx, buf),
            #[cfg(feature = "tokio")]
            RawStream::Tokio(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            RawStream::RustyTokio(s) => Pin::new(s).poll_flush(cx),
            #[cfg(feature = "tokio")]
            RawStream::Tokio(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            RawStream::RustyTokio(s) => Pin::new(s).poll_shutdown(cx),
            #[cfg(feature = "tokio")]
            RawStream::Tokio(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

pub(crate) enum Conn {
    Plain(RawStream),
    /// Boxed so the (larger) TLS variant doesn't grow every `Conn` --
    /// most requests in a mixed http/https workload are still plain TCP.
    Tls(Box<AsyncTlsStream<RawStream>>),
}

impl AsyncRead for Conn {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Conn::Plain(s) => Pin::new(s).poll_read(cx, buf),
            Conn::Tls(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for Conn {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Conn::Plain(s) => Pin::new(s).poll_write(cx, buf),
            Conn::Tls(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Conn::Plain(s) => Pin::new(s).poll_flush(cx),
            Conn::Tls(s) => Pin::new(s.as_mut()).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Conn::Plain(s) => Pin::new(s).poll_shutdown(cx),
            Conn::Tls(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
        }
    }
}
