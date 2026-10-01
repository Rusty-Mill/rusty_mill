//! The 16 MiB line cap for MCP's newline-delimited stdio transports (the
//! stdio server and the stdio client), applied beneath `rmcp`.
//!
//! `rmcp`'s line reader has no maximum: an unterminated line from a
//! misbehaving peer or subprocess grows its buffer without bound (review
//! round 5). [`LineCapped`] wraps the reader handed to `rmcp` and fails the
//! read once a line passes [`MAX_LINE_BYTES`], so growth stops at the
//! source. The shared flag lets the caller tell that failure apart from an
//! ordinary end of stream, which `rmcp` reports the same way.

use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, ReadBuf};

/// 16 MiB ceiling per line. A single JSON-RPC message larger than this is
/// almost certainly a protocol bug or a misbehaving peer; failing the read
/// beats growing the buffer without bound.
pub(crate) const MAX_LINE_BYTES: u64 = 16 * 1024 * 1024;

/// An [`AsyncRead`] that fails once a line exceeds [`MAX_LINE_BYTES`].
pub(crate) struct LineCapped<R> {
    inner: R,
    /// Bytes read since the last newline.
    run: u64,
    exceeded: Arc<AtomicBool>,
}

impl<R> LineCapped<R> {
    /// Wraps `inner`; `exceeded` is set when the cap is hit.
    pub(crate) fn new(inner: R, exceeded: Arc<AtomicBool>) -> Self {
        Self {
            inner,
            run: 0,
            exceeded,
        }
    }
}

/// The error a capped read fails with, and that callers report.
pub(crate) fn cap_error() -> String {
    format!("MCP line exceeds {MAX_LINE_BYTES}-byte cap")
}

impl<R: AsyncRead + Unpin> AsyncRead for LineCapped<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.exceeded.load(Ordering::Acquire) {
            return Poll::Ready(Err(io::Error::new(io::ErrorKind::InvalidData, cap_error())));
        }
        let before = buf.filled().len();
        let this = &mut *self;
        let polled = Pin::new(&mut this.inner).poll_read(cx, buf);
        if !matches!(polled, Poll::Ready(Ok(()))) {
            return polled;
        }
        let fresh = &buf.filled()[before..];
        this.run = match fresh.iter().rposition(|&b| b == b'\n') {
            Some(last) => (fresh.len() - last - 1) as u64,
            None => this.run + fresh.len() as u64,
        };
        if this.run > MAX_LINE_BYTES {
            this.exceeded.store(true, Ordering::Release);
            // A failed read must not report bytes as read (`AsyncRead`'s
            // contract), so the ones just filled are handed back.
            buf.set_filled(before);
            return Poll::Ready(Err(io::Error::new(io::ErrorKind::InvalidData, cap_error())));
        }
        Poll::Ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn read_all(payload: Vec<u8>) -> (io::Result<Vec<u8>>, bool) {
        let (mut writer, reader) = tokio::io::duplex(1 << 16);
        let feed = tokio::spawn(async move {
            let _ = writer.write_all(&payload).await;
        });
        let exceeded = Arc::new(AtomicBool::new(false));
        let mut capped = LineCapped::new(reader, Arc::clone(&exceeded));
        let mut out = Vec::new();
        let result = capped.read_to_end(&mut out).await.map(|_| out);
        let _ = feed.await;
        (result, exceeded.load(Ordering::Acquire))
    }

    #[tokio::test]
    async fn lines_under_the_cap_pass_through_unchanged() {
        let (result, exceeded) = read_all(b"one\ntwo\n".to_vec()).await;
        assert_eq!(result.unwrap(), b"one\ntwo\n");
        assert!(!exceeded);
    }

    #[tokio::test]
    async fn newlines_reset_the_count_so_long_streams_of_short_lines_pass() {
        let line = vec![b'a'; 1 << 20];
        let mut payload = Vec::new();
        for _ in 0..20 {
            payload.extend_from_slice(&line);
            payload.push(b'\n');
        }
        let (result, exceeded) = read_all(payload).await;
        assert_eq!(result.unwrap().len(), 20 * ((1 << 20) + 1));
        assert!(!exceeded);
    }

    #[tokio::test]
    async fn an_unterminated_line_past_the_cap_fails_and_flags() {
        let (result, exceeded) = read_all(vec![b'a'; MAX_LINE_BYTES as usize + 1]).await;
        assert!(result.is_err());
        assert!(exceeded);
    }
}
