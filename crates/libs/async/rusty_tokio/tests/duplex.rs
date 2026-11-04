use rusty_tokio::io::{self, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use rusty_tokio::Runtime;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::time::Duration;

#[test]
fn basic_roundtrip_both_directions() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let (mut a, mut b) = io::duplex(64);

        a.write_all(b"hello from a").await.unwrap();
        let mut buf = [0u8; 12];
        b.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"hello from a");

        b.write_all(b"hello from b").await.unwrap();
        let mut buf = [0u8; 12];
        a.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"hello from b");
    });
}

#[test]
fn write_blocks_until_the_peer_reads_some_of_it_back() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let (mut a, mut b) = io::duplex(4);

        // Fills the entire buffer -- nothing free for a fifth byte.
        a.write_all(b"abcd").await.unwrap();

        // A further write must block: there's no room, and nobody's
        // reading yet.
        let blocked = rusty_tokio::time::timeout(Duration::from_millis(100), async {
            a.write_all(b"e").await.unwrap();
        })
        .await;
        assert!(
            blocked.is_err(),
            "write_all should still be pending with a full buffer and no reader"
        );

        // The timed-out write above never got anywhere -- `timeout`
        // dropped it before it wrote a single byte, so `'e'` is simply
        // lost, the same as any other cancelled future. Draining two
        // bytes here frees up room in the buffer (now holding `"cd"`)
        // and wakes whatever's waiting to write next.
        let mut buf = [0u8; 2];
        b.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"ab");

        // Confirm the stream is still fully usable after that: a fresh
        // write now has room and succeeds immediately.
        a.write_all(b"f").await.unwrap();
        let mut rest = [0u8; 3];
        b.read_exact(&mut rest).await.unwrap();
        assert_eq!(&rest, b"cdf");
    });
}

#[test]
fn dropping_one_side_gives_the_other_an_eof_after_draining_whats_left() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let (mut a, mut b) = io::duplex(64);
        a.write_all(b"last words").await.unwrap();
        drop(a);

        let mut contents = Vec::new();
        b.read_to_end(&mut contents).await.unwrap();
        assert_eq!(contents, b"last words");

        // A further read past EOF keeps reporting EOF (0 bytes), not an
        // error or a hang.
        let mut buf = [0u8; 1];
        let n = b.read(&mut buf).await.unwrap();
        assert_eq!(n, 0);
    });
}

#[test]
fn writing_after_the_peers_read_half_dropped_fails_fast() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let (mut a, b) = io::duplex(64);
        drop(b);

        let err = rusty_tokio::time::timeout(Duration::from_millis(100), a.write_all(b"anyone?"))
            .await
            .expect("a write against a dropped peer must fail immediately, not hang")
            .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::BrokenPipe);
    });
}

#[test]
fn shutdown_half_closes_without_dropping_the_whole_stream() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let (mut a, mut b) = io::duplex(64);

        a.write_all(b"done writing").await.unwrap();
        a.shutdown().await.unwrap();

        // b sees EOF after draining what was already sent.
        let mut contents = Vec::new();
        b.read_to_end(&mut contents).await.unwrap();
        assert_eq!(contents, b"done writing");

        // a's own read half is unaffected by shutting down its write
        // side -- b can still reply, and a can still read it.
        b.write_all(b"got it").await.unwrap();
        let mut reply = [0u8; 6];
        a.read_exact(&mut reply).await.unwrap();
        assert_eq!(&reply, b"got it");
    });
}

/// A waker that counts how often it was woken.
#[derive(Default)]
struct Counter(AtomicUsize);

impl Wake for Counter {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn counter() -> (Arc<Counter>, Waker) {
    let count = Arc::new(Counter::default());
    let waker = Waker::from(count.clone());
    (count, waker)
}

fn wakes(count: &Counter) -> usize {
    count.0.load(Ordering::SeqCst)
}

fn poll_write(half: &mut io::DuplexStream, waker: &Waker) -> Poll<std::io::Result<usize>> {
    Pin::new(half).poll_write(&mut Context::from_waker(waker), b"x")
}

/// A one-byte duplex pair whose `a` -> `b` buffer is already full.
fn full_pair() -> (io::DuplexStream, io::DuplexStream) {
    let (mut a, b) = io::duplex(1);
    let (_, idle) = counter();
    assert!(matches!(poll_write(&mut a, &idle), Poll::Ready(Ok(1))));
    (a, b)
}

#[test]
fn a_blocked_writer_re_polled_with_a_new_waker_is_woken_through_that_one() {
    let (mut a, mut b) = full_pair();
    let (first, first_waker) = counter();
    let (second, second_waker) = counter();
    assert!(poll_write(&mut a, &first_waker).is_pending());
    assert!(poll_write(&mut a, &second_waker).is_pending());

    let mut byte = [0u8; 1];
    let mut buf = ReadBuf::new(&mut byte);
    let (_, idle) = counter();
    let read = Pin::new(&mut b).poll_read(&mut Context::from_waker(&idle), &mut buf);
    assert!(matches!(read, Poll::Ready(Ok(()))));

    assert_eq!(wakes(&second), 1, "the latest waker is woken");
    assert_eq!(wakes(&first), 0, "the replaced waker is not");
    assert!(matches!(
        poll_write(&mut a, &second_waker),
        Poll::Ready(Ok(1))
    ));
}

#[test]
fn dropping_the_reader_wakes_a_blocked_writer_which_then_fails() {
    let (mut a, b) = full_pair();
    let (count, waker) = counter();
    assert!(poll_write(&mut a, &waker).is_pending());

    drop(b);
    assert_eq!(wakes(&count), 1, "dropping the reader wakes the writer");
    match poll_write(&mut a, &waker) {
        Poll::Ready(Err(err)) => assert_eq!(err.kind(), std::io::ErrorKind::BrokenPipe),
        other => panic!("expected BrokenPipe after the reader dropped, got {other:?}"),
    }
}
