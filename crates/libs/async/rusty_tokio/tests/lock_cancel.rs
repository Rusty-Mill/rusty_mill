//! Cancellation safety of `Mutex` and `Semaphore` waiters (#1064): a
//! dropped `lock()`/`acquire()` future must neither swallow the wakeup
//! meant for the waiters behind it nor keep permits it was granted.
//! Polled by hand, so every interleaving is deterministic.

use rusty_tokio::sync::{Mutex, Semaphore};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

struct Flag(AtomicBool);

impl Wake for Flag {
    fn wake(self: Arc<Self>) {
        self.0.store(true, Ordering::SeqCst);
    }
}

impl Flag {
    fn woken(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

fn flag() -> (Arc<Flag>, Waker) {
    let flag = Arc::new(Flag(AtomicBool::new(false)));
    (flag.clone(), Waker::from(flag))
}

fn poll<F: Future>(fut: Pin<&mut F>, waker: &Waker) -> Poll<F::Output> {
    fut.poll(&mut Context::from_waker(waker))
}

#[test]
fn mutex_cancelled_head_waiter_passes_the_release_on() {
    let mutex = Mutex::new(0);
    let held = mutex.try_lock().expect("free");
    let (_, b_waker) = flag();
    let (c_flag, c_waker) = flag();
    let mut b = Box::pin(mutex.lock());
    let mut c = Box::pin(mutex.lock());
    assert!(poll(b.as_mut(), &b_waker).is_pending());
    assert!(poll(c.as_mut(), &c_waker).is_pending());

    drop(b);
    drop(held);
    assert!(c_flag.woken(), "next live waiter woken");
    assert!(poll(c.as_mut(), &c_waker).is_ready());
}

#[test]
fn mutex_head_waiter_cancelled_after_its_wake_passes_it_on() {
    let mutex = Mutex::new(0);
    let held = mutex.try_lock().expect("free");
    let (b_flag, b_waker) = flag();
    let (c_flag, c_waker) = flag();
    let mut b = Box::pin(mutex.lock());
    let mut c = Box::pin(mutex.lock());
    assert!(poll(b.as_mut(), &b_waker).is_pending());
    assert!(poll(c.as_mut(), &c_waker).is_pending());

    drop(held);
    assert!(b_flag.woken() && !c_flag.woken(), "only the head is woken");
    drop(b);
    assert!(c_flag.woken(), "the dropped head passed its wake on");
    assert!(poll(c.as_mut(), &c_waker).is_ready());
}

#[test]
fn mutex_cancelled_middle_waiter_is_skipped() {
    let mutex = Mutex::new(0);
    let held = mutex.try_lock().expect("free");
    let wakers = [flag(), flag(), flag()];
    let mut b = Box::pin(mutex.lock());
    let mut c = Box::pin(mutex.lock());
    let mut d = Box::pin(mutex.lock());
    assert!(poll(b.as_mut(), &wakers[0].1).is_pending());
    assert!(poll(c.as_mut(), &wakers[1].1).is_pending());
    assert!(poll(d.as_mut(), &wakers[2].1).is_pending());

    drop(c);
    drop(held);
    let Poll::Ready(b_guard) = poll(b.as_mut(), &wakers[0].1) else {
        panic!("head waiter acquires");
    };
    drop(b_guard);
    assert!(
        wakers[2].0.woken(),
        "the waiter behind the cancelled one is woken"
    );
    assert!(poll(d.as_mut(), &wakers[2].1).is_ready());
}

#[test]
fn mutex_repolled_waiter_is_woken_through_its_latest_waker_only() {
    let mutex = Mutex::new(0);
    let held = mutex.try_lock().expect("free");
    let (old_flag, old_waker) = flag();
    let (new_flag, new_waker) = flag();
    let mut b = Box::pin(mutex.lock());
    assert!(poll(b.as_mut(), &old_waker).is_pending());
    assert!(poll(b.as_mut(), &new_waker).is_pending());
    assert!(poll(b.as_mut(), &new_waker).is_pending());

    drop(held);
    assert!(new_flag.woken(), "latest waker woken");
    assert!(!old_flag.woken(), "no stale queue entry");
    assert!(poll(b.as_mut(), &new_waker).is_ready());
}

#[test]
fn semaphore_cancelled_queued_waiter_does_not_block_the_queue() {
    let sem = Semaphore::new(1);
    let held = sem.try_acquire().expect("free");
    let (_, b_waker) = flag();
    let (c_flag, c_waker) = flag();
    let mut b = Box::pin(sem.acquire());
    let mut c = Box::pin(sem.acquire());
    assert!(poll(b.as_mut(), &b_waker).is_pending());
    assert!(poll(c.as_mut(), &c_waker).is_pending());

    drop(b);
    drop(held);
    assert!(c_flag.woken(), "next live waiter granted");
    assert!(matches!(poll(c.as_mut(), &c_waker), Poll::Ready(Ok(_))));
}

#[test]
fn semaphore_granted_waiter_dropped_before_its_poll_returns_the_permits() {
    let sem = Semaphore::new(1);
    let held = sem.try_acquire().expect("free");
    let (b_flag, b_waker) = flag();
    let mut b = Box::pin(sem.acquire());
    assert!(poll(b.as_mut(), &b_waker).is_pending());

    drop(held);
    assert!(b_flag.woken(), "granted");
    drop(b);
    assert_eq!(sem.available_permits(), 1, "no permit leaked");
}

#[test]
fn semaphore_cancelled_large_head_lets_a_smaller_waiter_through() {
    let sem = Semaphore::new(2);
    let held = sem.try_acquire().expect("free");
    let (_, b_waker) = flag();
    let (c_flag, c_waker) = flag();
    let mut b = Box::pin(sem.acquire_many(2));
    let mut c = Box::pin(sem.acquire());
    assert!(poll(b.as_mut(), &b_waker).is_pending());
    assert!(poll(c.as_mut(), &c_waker).is_pending(), "FIFO: behind b");

    drop(b);
    assert!(c_flag.woken(), "granted the free permit");
    assert!(matches!(poll(c.as_mut(), &c_waker), Poll::Ready(Ok(_))));
    drop(held);
}

#[test]
fn semaphore_repolled_waiter_is_woken_through_its_latest_waker() {
    let sem = Semaphore::new(1);
    let held = sem.try_acquire().expect("free");
    let (_, old_waker) = flag();
    let (new_flag, new_waker) = flag();
    let mut b = Box::pin(sem.acquire());
    assert!(poll(b.as_mut(), &old_waker).is_pending());
    assert!(poll(b.as_mut(), &new_waker).is_pending());

    drop(held);
    assert!(new_flag.woken(), "latest waker woken");
    assert!(matches!(poll(b.as_mut(), &new_waker), Poll::Ready(Ok(_))));
}

#[test]
fn semaphore_close_still_fails_a_queued_waiter() {
    let sem = Semaphore::new(0);
    let (flag, waker) = flag();
    let mut b = Box::pin(sem.acquire());
    assert!(poll(b.as_mut(), &waker).is_pending());

    sem.close();
    assert!(flag.woken());
    assert!(matches!(poll(b.as_mut(), &waker), Poll::Ready(Err(_))));
    drop(b);
    sem.add_permits(1);
    assert_eq!(sem.available_permits(), 1, "a closed claim keeps nothing");
}
