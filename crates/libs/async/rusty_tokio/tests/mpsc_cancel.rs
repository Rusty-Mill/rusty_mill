//! Cancellation and progress for bounded `mpsc` senders: a cancelled
//! `send`/`reserve` must not swallow the wake that freed capacity, or a
//! surviving sender stays parked while the channel has room.

use rusty_tokio::sync::mpsc;
use std::future::Future;
use std::pin::pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

/// A waker that records whether it was woken.
struct Flag(AtomicBool);

impl Wake for Flag {
    fn wake(self: Arc<Self>) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn flag() -> (Arc<Flag>, Waker) {
    let f = Arc::new(Flag(AtomicBool::new(false)));
    (f.clone(), Waker::from(f))
}

fn woken(f: &Flag) -> bool {
    f.0.load(Ordering::SeqCst)
}

fn poll_once<F: Future>(fut: std::pin::Pin<&mut F>, waker: &Waker) -> Poll<F::Output> {
    fut.poll(&mut Context::from_waker(waker))
}

/// Receive one value, which must already be queued.
fn recv_now<T>(rx: &mut mpsc::Receiver<T>) -> T {
    let (_, w) = flag();
    match poll_once(pin!(rx.recv()), &w) {
        Poll::Ready(Some(v)) => v,
        _ => panic!("a value was queued"),
    }
}

#[test]
fn a_cancelled_waiting_send_does_not_swallow_the_wake() {
    let (tx, mut rx) = mpsc::channel(1);
    tx.try_send(0).unwrap();

    let (_, wa) = flag();
    let mut cancelled = Box::pin(tx.send(1));
    assert!(poll_once(cancelled.as_mut(), &wa).is_pending());
    drop(cancelled);

    let (fb, wb) = flag();
    let mut survivor = pin!(tx.send(2));
    assert!(poll_once(survivor.as_mut(), &wb).is_pending());

    assert_eq!(recv_now(&mut rx), 0);
    assert!(woken(&fb), "the surviving sender must be woken");
    assert!(matches!(
        poll_once(survivor.as_mut(), &wb),
        Poll::Ready(Ok(()))
    ));
    assert_eq!(recv_now(&mut rx), 2);
}

#[test]
fn a_cancelled_waiting_reserve_does_not_swallow_the_wake() {
    let (tx, mut rx) = mpsc::channel(1);
    tx.try_send(0).unwrap();

    let (_, wa) = flag();
    let mut cancelled = Box::pin(tx.reserve());
    assert!(poll_once(cancelled.as_mut(), &wa).is_pending());
    drop(cancelled);

    let (fb, wb) = flag();
    let mut survivor = pin!(tx.reserve());
    assert!(poll_once(survivor.as_mut(), &wb).is_pending());

    assert_eq!(recv_now(&mut rx), 0);
    assert!(woken(&fb), "the surviving reserve must be woken");
    match poll_once(survivor.as_mut(), &wb) {
        Poll::Ready(Ok(permit)) => permit.send(3),
        _ => panic!("capacity was free"),
    }
    assert_eq!(recv_now(&mut rx), 3);
}

/// The wake reached a sender that was then cancelled before it could
/// use it: the wake passes on to the next waiter.
#[test]
fn a_woken_then_cancelled_send_passes_the_wake_on() {
    let (tx, mut rx) = mpsc::channel(1);
    tx.try_send(0).unwrap();

    let (fa, wa) = flag();
    let mut first = Box::pin(tx.send(1));
    assert!(poll_once(first.as_mut(), &wa).is_pending());
    let (fb, wb) = flag();
    let mut second = pin!(tx.send(2));
    assert!(poll_once(second.as_mut(), &wb).is_pending());

    assert_eq!(recv_now(&mut rx), 0);
    assert!(woken(&fa));
    assert!(!woken(&fb), "one slot frees, one sender wakes");
    drop(first);
    assert!(woken(&fb), "the cancelled sender's wake must pass on");
    assert!(matches!(
        poll_once(second.as_mut(), &wb),
        Poll::Ready(Ok(()))
    ));
}

/// Re-polling a waiting sender (a spurious wake) must not queue it twice,
/// or one freed slot could be spent waking the same sender again.
#[test]
fn a_repolled_waiting_send_is_queued_once() {
    let (tx, mut rx) = mpsc::channel(1);
    tx.try_send(0).unwrap();

    let (_, wa) = flag();
    let mut first = pin!(tx.send(1));
    for _ in 0..3 {
        assert!(poll_once(first.as_mut(), &wa).is_pending());
    }
    let (fb, wb) = flag();
    let mut second = pin!(tx.send(2));
    assert!(poll_once(second.as_mut(), &wb).is_pending());

    assert_eq!(recv_now(&mut rx), 0);
    assert!(matches!(
        poll_once(first.as_mut(), &wa),
        Poll::Ready(Ok(()))
    ));
    assert_eq!(recv_now(&mut rx), 1);
    assert!(woken(&fb), "the second sender is next in line");
}
