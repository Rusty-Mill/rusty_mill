//! `Barrier::wait`, `OnceCell::get_or_init` and `SetOnce::wait` keep one
//! waker per waiting future (#579): a re-poll replaces it, and a dropped
//! future takes it out, instead of each pending poll adding another
//! that stays until the round or initialisation completes. Polled by
//! hand, with a waker whose `Arc` count shows how many clones are kept.

use rusty_tokio::sync::{Barrier, OnceCell, SetOnce};
use std::future::{pending, Future};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::time::Duration;

#[derive(Default)]
struct Count(AtomicUsize);

impl Wake for Count {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn counting() -> (Arc<Count>, Waker) {
    let count = Arc::new(Count::default());
    let waker = Waker::from(count.clone());
    (count, waker)
}

/// Clones of `count`'s waker kept by anyone but the test, which holds
/// the `Arc` and one `Waker`.
fn stored(count: &Arc<Count>) -> usize {
    Arc::strong_count(count) - 2
}

fn poll<F: Future>(fut: Pin<&mut F>, waker: &Waker) -> Poll<F::Output> {
    fut.poll(&mut Context::from_waker(waker))
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
fn barrier_repolled_wait_keeps_one_waker() {
    let barrier = Barrier::new(2);
    let (count, waker) = counting();
    let mut wait = Box::pin(barrier.wait());
    for _ in 0..1000 {
        assert!(poll(wait.as_mut(), &waker).is_pending());
    }
    assert_eq!(stored(&count), 1);
}

#[test]
fn barrier_cancelled_wait_leaves_no_waker() {
    let barrier = Barrier::new(1001);
    let (count, waker) = counting();
    for _ in 0..1000 {
        let mut wait = Box::pin(barrier.wait());
        assert!(poll(wait.as_mut(), &waker).is_pending());
    }
    assert_eq!(stored(&count), 0);
}

/// The round still completes, and wakes only the newest waker of the
/// waiter still waiting.
#[test]
fn barrier_repolled_wait_is_woken_through_its_newest_waker_only() {
    let barrier = Barrier::new(2);
    let (old, old_waker) = counting();
    let (new, new_waker) = counting();
    let mut wait = Box::pin(barrier.wait());
    assert!(poll(wait.as_mut(), &old_waker).is_pending());
    assert!(poll(wait.as_mut(), &new_waker).is_pending());
    let mut leader = Box::pin(barrier.wait());
    let Poll::Ready(result) = poll(leader.as_mut(), Waker::noop()) else {
        panic!("the last arrival completes the round");
    };
    assert!(result.is_leader());
    assert_eq!(old.0.load(Ordering::SeqCst), 0);
    assert_eq!(new.0.load(Ordering::SeqCst), 1);
    assert!(poll(wait.as_mut(), &new_waker).is_ready());
}

/// A cell with an initializer in flight that never finishes, so every
/// other `get_or_init` waits on it.
fn initializing(cell: &OnceCell<u8>) -> Pin<Box<impl Future<Output = &u8> + '_>> {
    let mut init = Box::pin(cell.get_or_init(pending::<u8>));
    assert!(poll(init.as_mut(), Waker::noop()).is_pending());
    init
}

#[test]
fn once_cell_repolled_wait_keeps_one_waker() {
    let cell = OnceCell::new();
    let _init = initializing(&cell);
    let (count, waker) = counting();
    let mut wait = Box::pin(cell.get_or_init(|| async { 1 }));
    for _ in 0..1000 {
        assert!(poll(wait.as_mut(), &waker).is_pending());
    }
    assert_eq!(stored(&count), 1);
}

#[test]
fn once_cell_cancelled_wait_leaves_no_waker() {
    let cell = OnceCell::new();
    let _init = initializing(&cell);
    let (count, waker) = counting();
    for _ in 0..1000 {
        let mut wait = Box::pin(cell.get_or_init(|| async { 1 }));
        assert!(poll(wait.as_mut(), &waker).is_pending());
    }
    assert_eq!(stored(&count), 0);
}

/// An abandoned initializer still hands over to a waiter, which is
/// woken through its newest waker and initialises the cell itself.
#[test]
fn once_cell_waiter_takes_over_an_abandoned_initializer() {
    let cell = OnceCell::new();
    let init = initializing(&cell);
    let (old, old_waker) = counting();
    let (new, new_waker) = counting();
    let mut wait = Box::pin(cell.get_or_init(|| async { 7 }));
    assert!(poll(wait.as_mut(), &old_waker).is_pending());
    assert!(poll(wait.as_mut(), &new_waker).is_pending());
    drop(init);
    assert_eq!(old.0.load(Ordering::SeqCst), 0);
    assert_eq!(new.0.load(Ordering::SeqCst), 1);
    assert_eq!(poll(wait.as_mut(), &new_waker), Poll::Ready(&7));
}

#[test]
fn set_once_repolled_wait_keeps_one_waker() {
    let cell = SetOnce::<u8>::new();
    let (count, waker) = counting();
    let mut wait = Box::pin(cell.wait());
    for _ in 0..1000 {
        assert!(poll(wait.as_mut(), &waker).is_pending());
    }
    assert_eq!(stored(&count), 1);
}

#[test]
fn set_once_cancelled_wait_leaves_no_waker() {
    let cell = SetOnce::<u8>::new();
    let (count, waker) = counting();
    for _ in 0..1000 {
        let mut wait = Box::pin(cell.wait());
        assert!(poll(wait.as_mut(), &waker).is_pending());
    }
    assert_eq!(stored(&count), 0);
}

#[test]
fn set_once_repolled_wait_is_woken_through_its_newest_waker_only() {
    let cell = SetOnce::<u8>::new();
    let (old, old_waker) = counting();
    let (new, new_waker) = counting();
    let mut wait = Box::pin(cell.wait());
    assert!(poll(wait.as_mut(), &old_waker).is_pending());
    assert!(poll(wait.as_mut(), &new_waker).is_pending());
    cell.set(3).expect("unset");
    assert_eq!(old.0.load(Ordering::SeqCst), 0);
    assert_eq!(new.0.load(Ordering::SeqCst), 1);
    assert_eq!(poll(wait.as_mut(), &new_waker), Poll::Ready(&3));
}

/// A waker that owns the `SetOnce`'s `Arc`: dropping it may drop the
/// cell, and its `wake` reads it, so neither may happen under the
/// cell's lock.
struct OwnsCell(Arc<SetOnce<u8>>);

impl Wake for OwnsCell {
    fn wake(self: Arc<Self>) {
        let _ = self.0.initialized();
    }
}

#[test]
fn set_once_drops_and_wakes_wakers_outside_its_lock() {
    bounded(|| {
        let cell = Arc::new(SetOnce::<u8>::new());
        // Displaced by a re-poll, then removed by a cancel.
        let mut wait = Box::pin(cell.wait());
        let owning = Waker::from(Arc::new(OwnsCell(cell.clone())));
        assert!(poll(wait.as_mut(), &owning).is_pending());
        drop(owning);
        assert!(poll(wait.as_mut(), Waker::noop()).is_pending());
        let owning = Waker::from(Arc::new(OwnsCell(cell.clone())));
        assert!(poll(wait.as_mut(), &owning).is_pending());
        drop(owning);
        drop(wait);
        // Woken by `set`.
        let mut wait = Box::pin(cell.wait());
        let owning = Waker::from(Arc::new(OwnsCell(cell.clone())));
        assert!(poll(wait.as_mut(), &owning).is_pending());
        drop(owning);
        cell.set(1).expect("unset");
        assert_eq!(poll(wait.as_mut(), Waker::noop()), Poll::Ready(&1));
    });
}
