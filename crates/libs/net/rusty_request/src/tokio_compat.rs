//! Compat shim behind the `tokio` feature.
//!
//! `rusty_tls::AsyncTlsStream` and `rusty_http::async_tokio::AsyncTransport`
//! are both generic over `rusty_tokio`'s own `AsyncRead`/`AsyncWrite`
//! traits -- neither crate has grown a real-tokio adapter of its own yet
//! (unlike `rusty_http`, which already has a `tokio` feature alongside
//! its `rusty-tokio` one). Rather than block this crate's own real-tokio
//! support on that upstream work, [`TokioIo`] bridges a real tokio
//! transport into the trait shape those two crates already accept.
//!
//! `TokioIo::poll_read`/`poll_write`/`poll_flush`/`poll_shutdown` do
//! nothing but forward to the wrapped value's own real-tokio poll
//! methods, translating `rusty_tokio::io::ReadBuf` to/from `tokio::io::
//! ReadBuf` on the read side. No `rusty_tokio` reactor, executor, or
//! timer ever runs here -- that crate contributes only the trait and
//! `ReadBuf` *definitions* [`TokioIo`] implements, never anything that
//! executes. Every actual read/write/connect is real tokio's own, driven
//! by whatever runtime the caller is already inside -- exactly the
//! "don't run two runtimes" property this feature exists for.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

/// Wraps a real tokio `AsyncRead + AsyncWrite` value so it also
/// implements `rusty_tokio::io::{AsyncRead, AsyncWrite}` -- see the
/// module docs for why that's safe and doesn't start a second runtime.
pub(crate) struct TokioIo<T>(T);

impl<T> TokioIo<T> {
    pub(crate) fn new(inner: T) -> Self {
        TokioIo(inner)
    }
}

impl<T: tokio::io::AsyncRead + Unpin> rusty_tokio::io::AsyncRead for TokioIo<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut rusty_tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let mut tokio_buf = tokio::io::ReadBuf::new(buf.unfilled_mut());
        match Pin::new(&mut this.0).poll_read(cx, &mut tokio_buf) {
            Poll::Ready(Ok(())) => {
                let n = tokio_buf.filled().len();
                buf.advance(n);
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<T: tokio::io::AsyncWrite + Unpin> rusty_tokio::io::AsyncWrite for TokioIo<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        Pin::new(&mut this.0).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        Pin::new(&mut this.0).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        Pin::new(&mut this.0).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_tokio::io::{AsyncRead as _, AsyncWrite as _, ReadBuf};

    /// A minimal real-tokio `AsyncRead + AsyncWrite` in-memory pair, just
    /// enough to prove [`TokioIo`] round-trips reads and writes through
    /// real tokio's poll methods rather than doing anything itself.
    struct MemoryDuplex {
        to_read: std::collections::VecDeque<u8>,
        written: Vec<u8>,
    }

    impl tokio::io::AsyncRead for MemoryDuplex {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            let n = std::cmp::min(buf.remaining(), self.to_read.len());
            for _ in 0..n {
                buf.put_slice(&[self.to_read.pop_front().unwrap()]);
            }
            Poll::Ready(Ok(()))
        }
    }

    impl tokio::io::AsyncWrite for MemoryDuplex {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<io::Result<usize>> {
            self.written.extend_from_slice(buf);
            Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    fn noop_context() -> Context<'static> {
        use std::task::{RawWaker, RawWakerVTable, Waker};
        const VTABLE: RawWakerVTable = RawWakerVTable::new(
            |_| RawWaker::new(std::ptr::null(), &VTABLE),
            |_| {},
            |_| {},
            |_| {},
        );
        let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
        Context::from_waker(Box::leak(Box::new(waker)))
    }

    #[test]
    fn read_delegates_to_the_real_tokio_impl() {
        let mut io = TokioIo::new(MemoryDuplex {
            to_read: b"hello".iter().copied().collect(),
            written: Vec::new(),
        });
        let mut cx = noop_context();
        let mut backing = [0u8; 8];
        let mut buf = ReadBuf::new(&mut backing);
        match Pin::new(&mut io).poll_read(&mut cx, &mut buf) {
            Poll::Ready(Ok(())) => {}
            other => panic!("expected Ready(Ok(())), got {other:?}"),
        }
        assert_eq!(buf.filled(), b"hello");
    }

    #[test]
    fn write_delegates_to_the_real_tokio_impl() {
        let mut io = TokioIo::new(MemoryDuplex {
            to_read: std::collections::VecDeque::new(),
            written: Vec::new(),
        });
        let mut cx = noop_context();
        match Pin::new(&mut io).poll_write(&mut cx, b"world") {
            Poll::Ready(Ok(5)) => {}
            other => panic!("expected Ready(Ok(5)), got {other:?}"),
        }
        assert_eq!(io.0.written, b"world");
    }
}
