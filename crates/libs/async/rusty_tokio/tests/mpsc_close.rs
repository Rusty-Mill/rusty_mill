//! `Receiver::close`/`UnboundedReceiver::close` (card #406), as tokio's:
//! later sends fail, waiting senders wake to see that, and what was
//! already queued, or is sent through a permit taken before, still arrives.

use rusty_tokio::sync::mpsc::{self, error::TryRecvError};
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

#[test]
fn close_drains_what_was_queued_then_ends() {
    let (tx, mut rx) = mpsc::channel(4);
    tx.try_send(1).unwrap();
    tx.try_send(2).unwrap();
    rx.close();
    assert!(tx.try_send(3).unwrap_err().is_closed());
    assert_eq!(rx.try_recv(), Ok(1));
    assert_eq!(rx.blocking_recv(), Some(2));
    assert_eq!(rx.try_recv(), Err(TryRecvError::Disconnected));
    assert_eq!(
        rx.blocking_recv(),
        None,
        "senders are alive, but the channel is closed"
    );
}

#[test]
fn close_wakes_a_sender_waiting_for_capacity_which_then_fails() {
    let (tx, mut rx) = mpsc::channel(1);
    tx.try_send(1).unwrap();
    let (flag, waker) = flag();
    let mut send = pin!(tx.send(2));
    assert!(poll_once(send.as_mut(), &waker).is_pending());

    rx.close();
    assert!(woken(&flag), "close wakes the waiting sender");
    match poll_once(send.as_mut(), &waker) {
        Poll::Ready(Err(err)) => assert_eq!(err.0, 2),
        other => panic!("expected the send to fail after close, got {other:?}"),
    }
    assert_eq!(rx.try_recv(), Ok(1));
}

#[test]
fn a_permit_taken_before_close_can_still_send() {
    let (tx, mut rx) = mpsc::channel(2);
    let permit = tx.try_reserve().unwrap();
    rx.close();
    assert_eq!(
        rx.try_recv(),
        Err(TryRecvError::Empty),
        "the permit may still send"
    );
    permit.send(7);
    assert_eq!(rx.try_recv(), Ok(7));
    assert_eq!(rx.try_recv(), Err(TryRecvError::Disconnected));
}

#[test]
fn an_unused_permit_dropped_after_close_ends_a_waiting_recv() {
    let (tx, mut rx) = mpsc::channel::<u8>(2);
    let permit = tx.try_reserve().unwrap();
    rx.close();
    let (flag, waker) = flag();
    {
        let mut recv = pin!(rx.recv());
        assert!(poll_once(recv.as_mut(), &waker).is_pending());
        drop(permit);
        assert!(
            woken(&flag),
            "the last permit going unused wakes the receiver"
        );
        assert_eq!(poll_once(recv.as_mut(), &waker), Poll::Ready(None));
    }
    drop(tx);
}

#[test]
fn unbounded_close_drains_what_was_queued_then_ends() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    tx.send(1).unwrap();
    rx.close();
    assert_eq!(tx.send(2).unwrap_err().0, 2);
    assert_eq!(rx.try_recv(), Ok(1));
    assert_eq!(rx.try_recv(), Err(TryRecvError::Disconnected));
    assert_eq!(rx.blocking_recv(), None);
}
