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
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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
        let mut run = this.run;
        for &byte in fresh {
            if byte == b'\n' {
                run = 0;
            } else {
                run += 1;
                if run > MAX_LINE_BYTES {
                    this.exceeded.store(true, Ordering::Release);
                    // A failed read must not report bytes as read (`AsyncRead`'s
                    // contract), so the ones just filled are handed back.
                    buf.set_filled(before);
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        cap_error(),
                    )));
                }
            }
        }
        this.run = run;
        Poll::Ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Arc;
    use std::task::Waker;

    use super::*;
    use tokio::io::AsyncReadExt;

    enum Step {
        Data(Vec<u8>),
        Pending,
        Error(io::ErrorKind),
        Eof,
    }

    struct ScriptedReader {
        steps: VecDeque<Step>,
        polls: usize,
    }

    impl ScriptedReader {
        fn new(steps: impl IntoIterator<Item = Step>) -> Self {
            Self {
                steps: steps.into_iter().collect(),
                polls: 0,
            }
        }
    }

    impl AsyncRead for ScriptedReader {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            self.polls += 1;
            match self.steps.pop_front().unwrap_or(Step::Eof) {
                Step::Data(mut bytes) => {
                    let len = bytes.len().min(buf.remaining());
                    buf.put_slice(&bytes[..len]);
                    if len < bytes.len() {
                        bytes.drain(..len);
                        self.steps.push_front(Step::Data(bytes));
                    }
                    Poll::Ready(Ok(()))
                }
                Step::Pending => {
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
                Step::Error(kind) => Poll::Ready(Err(io::Error::new(kind, "scripted error"))),
                Step::Eof => Poll::Ready(Ok(())),
            }
        }
    }

    fn poll_once<R: AsyncRead + Unpin>(
        reader: &mut R,
        storage: &mut [u8],
        prefilled: &[u8],
    ) -> (Poll<io::Result<()>>, usize, Vec<u8>) {
        let mut cx = Context::from_waker(Waker::noop());
        let mut buf = ReadBuf::new(storage);
        buf.put_slice(prefilled);
        let result = Pin::new(reader).poll_read(&mut cx, &mut buf);
        (result, buf.filled().len(), buf.filled().to_vec())
    }

    async fn read_all(steps: impl IntoIterator<Item = Step>) -> (io::Result<Vec<u8>>, bool) {
        let exceeded = Arc::new(AtomicBool::new(false));
        let mut capped = LineCapped::new(ScriptedReader::new(steps), Arc::clone(&exceeded));
        let mut out = Vec::new();
        let result = capped.read_to_end(&mut out).await.map(|_| out);
        (result, exceeded.load(Ordering::Acquire))
    }

    fn assert_cap_error(result: io::Result<Vec<u8>>, exceeded: bool) {
        match result {
            Err(error) => assert_eq!(error.kind(), io::ErrorKind::InvalidData),
            Ok(bytes) => panic!("expected cap error after {} bytes", bytes.len()),
        }
        assert!(exceeded);
    }

    fn assert_poll_error(result: Poll<io::Result<()>>, expected: io::ErrorKind) {
        match result {
            Poll::Ready(Err(error)) => assert_eq!(error.kind(), expected),
            _ => panic!("expected a ready error"),
        }
    }

    #[tokio::test]
    async fn several_normal_lines_pass_through_unchanged() {
        let (result, exceeded) = read_all([
            Step::Data(b"one\ntwo\n".to_vec()),
            Step::Pending,
            Step::Data(b"three".to_vec()),
        ])
        .await;
        assert_eq!(result.unwrap(), b"one\ntwo\nthree");
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
        let expected_len = payload.len();
        let (result, exceeded) = read_all([Step::Data(payload)]).await;
        assert_eq!(result.unwrap().len(), expected_len);
        assert!(!exceeded);
    }

    #[test]
    fn exact_cap_completed_line_is_accepted_in_one_poll() {
        let exceeded = Arc::new(AtomicBool::new(false));
        let mut payload = vec![b'a'; MAX_LINE_BYTES as usize];
        payload.push(b'\n');
        let inner = ScriptedReader::new([Step::Data(payload.clone())]);
        let mut capped = LineCapped::new(inner, Arc::clone(&exceeded));
        let prefix = b"existing";
        let mut storage = vec![0; prefix.len() + payload.len()];

        let (result, filled, contents) = poll_once(&mut capped, &mut storage, prefix);
        assert!(matches!(result, Poll::Ready(Ok(()))));
        assert_eq!(capped.inner.polls, 1);
        let mut expected = prefix.to_vec();
        expected.extend_from_slice(&payload);
        assert_eq!(filled, expected.len());
        assert_eq!(&contents[..prefix.len()], prefix);
        assert_eq!(contents, expected);
        assert!(!exceeded.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn exact_cap_completed_line_is_accepted_across_polls() {
        let line = vec![b'a'; MAX_LINE_BYTES as usize];
        let mut expected = line.clone();
        expected.push(b'\n');
        let (result, exceeded) = read_all([Step::Data(line), Step::Data(b"\n".to_vec())]).await;
        assert_eq!(result.unwrap(), expected);
        assert!(!exceeded);
    }

    #[tokio::test]
    async fn exact_cap_prefix_then_extra_byte_and_newline_fails_cross_poll() {
        let (result, exceeded) = read_all([
            Step::Data(vec![b'a'; MAX_LINE_BYTES as usize]),
            Step::Data(b"x\n".to_vec()),
        ])
        .await;
        assert_cap_error(result, exceeded);
    }

    #[test]
    fn cap_plus_one_then_newline_fails_in_one_poll() {
        let exceeded = Arc::new(AtomicBool::new(false));
        let mut payload = vec![b'a'; MAX_LINE_BYTES as usize + 1];
        payload.push(b'\n');
        let inner = ScriptedReader::new([Step::Data(payload.clone())]);
        let mut capped = LineCapped::new(inner, Arc::clone(&exceeded));
        let prefix = b"existing";
        let mut storage = vec![0; prefix.len() + payload.len()];

        let (result, filled, contents) = poll_once(&mut capped, &mut storage, prefix);
        assert_poll_error(result, io::ErrorKind::InvalidData);
        assert_eq!(capped.inner.polls, 1);
        assert_eq!(filled, prefix.len());
        assert_eq!(contents, prefix);
        assert!(exceeded.load(Ordering::Acquire));
    }

    #[test]
    fn oversized_first_line_before_short_second_fails_in_one_poll() {
        let exceeded = Arc::new(AtomicBool::new(false));
        let mut payload = vec![b'a'; MAX_LINE_BYTES as usize + 1];
        payload.extend_from_slice(b"\nok\n");
        let inner = ScriptedReader::new([Step::Data(payload.clone())]);
        let mut capped = LineCapped::new(inner, Arc::clone(&exceeded));
        let prefix = b"existing";
        let mut storage = vec![0; prefix.len() + payload.len()];

        let (result, filled, contents) = poll_once(&mut capped, &mut storage, prefix);
        assert_poll_error(result, io::ErrorKind::InvalidData);
        assert_eq!(capped.inner.polls, 1);
        assert_eq!(filled, prefix.len());
        assert_eq!(contents, prefix);
        assert!(exceeded.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn oversized_unfinished_suffix_fails_same_and_cross_poll() {
        let (result, exceeded) =
            read_all([Step::Data(vec![b'a'; MAX_LINE_BYTES as usize + 1])]).await;
        assert_cap_error(result, exceeded);

        let (result, exceeded) = read_all([
            Step::Data(b"ok\n".to_vec()),
            Step::Data(vec![b'a'; MAX_LINE_BYTES as usize]),
            Step::Data(b"x".to_vec()),
        ])
        .await;
        assert_cap_error(result, exceeded);
    }

    #[test]
    fn refusal_is_sticky_and_does_not_poll_inner_again() {
        let exceeded = Arc::new(AtomicBool::new(false));
        let inner = ScriptedReader::new([
            Step::Data(vec![b'a'; MAX_LINE_BYTES as usize + 1]),
            Step::Data(b"later".to_vec()),
        ]);
        let mut capped = LineCapped::new(inner, Arc::clone(&exceeded));
        let mut storage = vec![0; MAX_LINE_BYTES as usize + 16];

        let (first, filled, _) = poll_once(&mut capped, &mut storage, &[]);
        assert_poll_error(first, io::ErrorKind::InvalidData);
        assert_eq!(filled, 0);
        assert_eq!(capped.inner.polls, 1);

        let (second, filled, _) = poll_once(&mut capped, &mut storage, &[]);
        assert_poll_error(second, io::ErrorKind::InvalidData);
        assert_eq!(filled, 0);
        assert_eq!(capped.inner.polls, 1);
        assert!(exceeded.load(Ordering::Acquire));
    }

    #[test]
    fn prefilled_read_buf_survives_refusal_byte_for_byte() {
        let exceeded = Arc::new(AtomicBool::new(false));
        let inner = ScriptedReader::new([Step::Data(vec![b'a'; MAX_LINE_BYTES as usize + 1])]);
        let mut capped = LineCapped::new(inner, Arc::clone(&exceeded));
        let prefix = b"caller's existing bytes";
        let mut storage = vec![0; prefix.len() + MAX_LINE_BYTES as usize + 1];

        let (result, filled, contents) = poll_once(&mut capped, &mut storage, prefix);
        assert_poll_error(result, io::ErrorKind::InvalidData);
        assert_eq!(filled, prefix.len());
        assert_eq!(contents, prefix);
        assert!(exceeded.load(Ordering::Acquire));
    }

    #[test]
    fn pending_eof_inner_error_and_zero_capacity_are_forwarded() {
        let exceeded = Arc::new(AtomicBool::new(false));
        let inner = ScriptedReader::new([
            Step::Pending,
            Step::Data(b"ok".to_vec()),
            Step::Eof,
            Step::Error(io::ErrorKind::BrokenPipe),
        ]);
        let mut capped = LineCapped::new(inner, Arc::clone(&exceeded));
        let mut empty = [];
        let (pending, filled, _) = poll_once(&mut capped, &mut empty, &[]);
        assert!(pending.is_pending());
        assert_eq!(filled, 0);

        let (zero_capacity, filled, _) = poll_once(&mut capped, &mut empty, &[]);
        assert!(zero_capacity.is_ready());
        assert_eq!(filled, 0);
        assert_eq!(capped.run, 0);

        let mut storage = [0; 2];
        let (data, filled, contents) = poll_once(&mut capped, &mut storage, &[]);
        assert!(data.is_ready());
        assert_eq!(filled, 2);
        assert_eq!(contents, b"ok");

        let (eof, filled, _) = poll_once(&mut capped, &mut storage, &[]);
        assert!(eof.is_ready());
        assert_eq!(filled, 0);

        let (error, filled, _) = poll_once(&mut capped, &mut storage, &[]);
        assert_poll_error(error, io::ErrorKind::BrokenPipe);
        assert_eq!(filled, 0);
        assert!(!exceeded.load(Ordering::Acquire));
    }
}
