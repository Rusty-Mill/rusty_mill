//! Adapting between `tokio`'s async I/O traits and `rusty_tokio`'s.
//!
//! `rusty_tls`' async server adapter is written against `rusty_tokio`'s
//! `AsyncRead`/`AsyncWrite`, while this gateway runs on `tokio` and hands its
//! connections to `hyper`. The two traits are shape-identical but distinct
//! types, so a TLS stream has to be adapted twice: the TCP socket goes *into*
//! `rusty_tls` as a `rusty_tokio` stream, and the decrypted stream comes back
//! *out* as a `tokio` one.
//!
//! # This costs nothing per byte
//!
//! Both runtimes' `ReadBuf` is an initialized `&mut [u8]` plus a filled
//! cursor, and both expose the unfilled tail as a plain slice. So the inner
//! reader writes **directly into the outer buffer's spare capacity** and the
//! adapter only forwards the count — no intermediate buffer, no copy, and no
//! `unsafe`, which this workspace forbids anyway.
//!
//! The write direction is a pure pass-through: `poll_write`, `poll_flush` and
//! `poll_shutdown` have identical signatures on both sides.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

/// A `tokio` stream presented to `rusty_tokio`.
pub struct ToRusty<T>(pub T);

impl<T: tokio::io::AsyncRead + Unpin> rusty_tokio::io::AsyncRead for ToRusty<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut rusty_tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        // Borrow the destination's spare capacity and let tokio fill it in
        // place; only the count crosses the boundary.
        let mut inner = tokio::io::ReadBuf::new(buf.unfilled_mut());
        ready!(Pin::new(&mut this.0).poll_read(cx, &mut inner))?;
        let filled = inner.filled().len();
        buf.advance(filled);
        Poll::Ready(Ok(()))
    }
}

impl<T: tokio::io::AsyncWrite + Unpin> rusty_tokio::io::AsyncWrite for ToRusty<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().0).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_shutdown(cx)
    }
}

/// A `rusty_tokio` stream presented to `tokio`.
pub struct ToTokio<T>(pub T);

impl<T: rusty_tokio::io::AsyncRead + Unpin> tokio::io::AsyncRead for ToTokio<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        // `initialize_unfilled` is the safe way to get at tokio's spare
        // capacity: it zeroes the tail once and remembers, so repeated reads
        // do not re-zero. The alternative is `unsafe`.
        let mut inner = rusty_tokio::io::ReadBuf::new(buf.initialize_unfilled());
        ready!(Pin::new(&mut this.0).poll_read(cx, &mut inner))?;
        let filled = inner.filled().len();
        buf.advance(filled);
        Poll::Ready(Ok(()))
    }
}

impl<T: rusty_tokio::io::AsyncWrite + Unpin> tokio::io::AsyncWrite for ToTokio<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().0).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    /// A tokio-side duplex pair, bridged out to `rusty_tokio` and back.
    ///
    /// Round-tripping through both adapters is the assertion that matters:
    /// each one alone could hide a filled/advance mistake that the other
    /// cancels out.
    #[tokio::test]
    async fn bytes_survive_a_round_trip_through_both_adapters() {
        let (client, server) = tokio::io::duplex(64 * 1024);

        // server side: tokio -> rusty_tokio -> tokio
        let mut bridged = ToTokio(ToRusty(server));

        let payload: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
        let expected = payload.clone();

        let writer = tokio::spawn(async move {
            let mut client = client;
            client.write_all(&payload).await.expect("write should succeed");
            client.shutdown().await.expect("shutdown should succeed");
        });

        let mut received = Vec::new();
        bridged
            .read_to_end(&mut received)
            .await
            .expect("read should succeed");
        writer.await.expect("writer task should not panic");

        assert_eq!(
            received, expected,
            "a payload larger than one read must come back byte-identical"
        );
    }

    #[tokio::test]
    async fn a_partial_read_advances_only_what_was_filled() {
        // The bug this guards: advancing by the buffer's capacity rather than
        // the bytes actually written would hand the caller uninitialized tail.
        let (mut client, server) = tokio::io::duplex(64);
        let mut bridged = ToTokio(ToRusty(server));

        client.write_all(b"four").await.expect("write should succeed");

        let mut buf = [0xAAu8; 32];
        let n = bridged.read(&mut buf).await.expect("read should succeed");
        assert_eq!(n, 4);
        assert_eq!(&buf[..4], b"four");
    }

    #[tokio::test]
    async fn writes_pass_through_in_both_directions() {
        let (mut client, server) = tokio::io::duplex(1024);
        let mut bridged = ToTokio(ToRusty(server));

        bridged
            .write_all(b"from the bridge")
            .await
            .expect("write should succeed");
        bridged.flush().await.expect("flush should succeed");

        let mut buf = vec![0u8; 15];
        client
            .read_exact(&mut buf)
            .await
            .expect("read should succeed");
        assert_eq!(&buf, b"from the bridge");
    }
}
