//! A `DuplexStream` waker may own the other half of its pair, as a task
//! that holds both halves does. Waking or dropping such a waker under a
//! pipe lock runs that half's `Drop`, which takes the same lock: these
//! cover every path that wakes or replaces a waker. Polled by hand, so
//! every interleaving is deterministic.

use rusty_tokio::io::{self, AsyncRead, AsyncWrite, DuplexStream, ReadBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};
use std::time::Duration;

/// A slot the test polls a half through, and a waker can empty.
type Slot = Arc<Mutex<Option<DuplexStream>>>;

/// A waker that drops whatever `slot` holds when it is woken or dropped.
struct Owns(Slot);

#[allow(
    clippy::manual_noop_waker,
    reason = "the destructor is the point; `Waker::noop` has none"
)]
impl Wake for Owns {
    fn wake(self: Arc<Self>) {}
}

impl Drop for Owns {
    fn drop(&mut self) {
        let half = self.0.lock().expect("slot lock").take();
        drop(half);
    }
}

fn slot(half: DuplexStream) -> Slot {
    Arc::new(Mutex::new(Some(half)))
}

fn owning(slot: &Slot) -> Waker {
    Waker::from(Arc::new(Owns(slot.clone())))
}

/// Parks a read on the half in `slot` with `waker`.
fn park_read(slot: &Slot, waker: &Waker) {
    let mut guard = slot.lock().expect("slot lock");
    let half = guard.as_mut().expect("half still in its slot");
    let mut bytes = [0u8; 1];
    let mut buf = ReadBuf::new(&mut bytes);
    let polled = Pin::new(half).poll_read(&mut Context::from_waker(waker), &mut buf);
    assert!(polled.is_pending(), "nothing written yet");
}

/// Runs `f` on its own thread, so a deadlock fails the test instead of
/// hanging it.
fn bounded(f: impl FnOnce() + Send + 'static) {
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        f();
        let _ = done.send(());
    });
    finished
        .recv_timeout(Duration::from_secs(10))
        .expect("finished without deadlocking or panicking");
}

#[test]
fn dropping_a_half_whose_peer_reader_owns_the_peer() {
    bounded(|| {
        let (a, b) = io::duplex(8);
        let b = slot(b);
        park_read(&b, &owning(&b));
        drop(a);
        assert!(b.lock().expect("slot lock").is_none(), "wake dropped b");
    });
}

#[test]
fn a_write_that_wakes_a_reader_owning_the_reader() {
    bounded(|| {
        let (mut a, b) = io::duplex(8);
        let b = slot(b);
        park_read(&b, &owning(&b));
        let written = Pin::new(&mut a).poll_write(&mut Context::from_waker(Waker::noop()), b"x");
        assert!(matches!(written, Poll::Ready(Ok(1))));
        assert!(b.lock().expect("slot lock").is_none(), "wake dropped b");
    });
}

#[test]
fn a_read_that_wakes_a_writer_owning_the_writer() {
    bounded(|| {
        let (a, mut b) = io::duplex(1);
        let a = slot(a);
        {
            let mut guard = a.lock().expect("slot lock");
            let half = Pin::new(guard.as_mut().expect("a in its slot"));
            let noop = &mut Context::from_waker(Waker::noop());
            assert!(matches!(half.poll_write(noop, b"x"), Poll::Ready(Ok(1))));
            let half = Pin::new(guard.as_mut().expect("a in its slot"));
            let full = half.poll_write(&mut Context::from_waker(&owning(&a)), b"y");
            assert!(full.is_pending(), "buffer full");
        }
        let mut bytes = [0u8; 1];
        let mut buf = ReadBuf::new(&mut bytes);
        let read = Pin::new(&mut b).poll_read(&mut Context::from_waker(Waker::noop()), &mut buf);
        assert!(matches!(read, Poll::Ready(Ok(()))));
        assert!(a.lock().expect("slot lock").is_none(), "wake dropped a");
    });
}

#[test]
fn shutdown_that_wakes_a_reader_owning_the_reader() {
    bounded(|| {
        let (mut a, b) = io::duplex(8);
        let b = slot(b);
        park_read(&b, &owning(&b));
        let shut = Pin::new(&mut a).poll_shutdown(&mut Context::from_waker(Waker::noop()));
        assert!(matches!(shut, Poll::Ready(Ok(()))));
        assert!(b.lock().expect("slot lock").is_none(), "wake dropped b");
    });
}

#[test]
fn re_registering_drops_a_waker_owning_the_peer() {
    bounded(|| {
        let (a, b) = io::duplex(8);
        let (a, b) = (slot(a), slot(b));
        park_read(&b, &owning(&a));
        park_read(&b, Waker::noop());
        assert!(
            a.lock().expect("slot lock").is_none(),
            "replaced waker dropped a"
        );
    });
}
