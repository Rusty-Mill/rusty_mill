//! A tokio stream under `rusty_tokio`'s I/O traits, which is what
//! `rusty_h2`'s connection driver reads and writes through. The driver needs
//! no `rusty_tokio` runtime, so this crate keeps running it on tokio.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use tokio::io::{AsyncRead, AsyncWrite};

pub(crate) struct RustyIo<T>(pub(crate) T);

impl<T: AsyncRead + Unpin> rusty_tokio::io::AsyncRead for RustyIo<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut rusty_tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let mut theirs = tokio::io::ReadBuf::new(buf.unfilled_mut());
        ready!(Pin::new(&mut self.0).poll_read(cx, &mut theirs))?;
        let n = theirs.filled().len();
        buf.advance(n);
        Poll::Ready(Ok(()))
    }
}

impl<T: AsyncWrite + Unpin> rusty_tokio::io::AsyncWrite for RustyIo<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}
