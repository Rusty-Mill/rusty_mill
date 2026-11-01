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
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Wake, Waker};
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

/// A waker whose last drop runs `on_drop`, as a user-defined one may.
struct OnDrop<F: Fn() + Send + Sync>(F);

#[allow(
    clippy::manual_noop_waker,
    reason = "the destructor is the point; `Waker::noop` has none"
)]
impl<F: Fn() + Send + Sync> Wake for OnDrop<F> {
    fn wake(self: Arc<Self>) {}
}

impl<F: Fn() + Send + Sync> Drop for OnDrop<F> {
    fn drop(&mut self) {
        (self.0)();
    }
}

fn on_drop(f: impl Fn() + Send + Sync + 'static) -> Waker {
    Waker::from(Arc::new(OnDrop(f)))
}

/// A waker whose clone runs `on_clone`, as a custom `RawWaker` may.
fn clone_hook(on_clone: impl Fn() + Send + Sync + 'static) -> Waker {
    type Hook = Box<dyn Fn() + Send + Sync>;
    unsafe fn clone(data: *const ()) -> RawWaker {
        // SAFETY: `data` is a live `Arc<Hook>` from `Arc::into_raw`.
        unsafe { (*data.cast::<Hook>())() };
        // SAFETY: as above; the new `RawWaker` owns one more count.
        unsafe { Arc::increment_strong_count(data.cast::<Hook>()) };
        RawWaker::new(data, &VTABLE)
    }
    unsafe fn release(data: *const ()) {
        // SAFETY: each `RawWaker` owns one count of the `Arc`.
        unsafe { Arc::decrement_strong_count(data.cast::<Hook>()) };
    }
    unsafe fn wake_by_ref(_: *const ()) {}
    static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, release, wake_by_ref, release);

    let hook: Arc<Hook> = Arc::new(Box::new(on_clone));
    let raw = RawWaker::new(Arc::into_raw(hook).cast(), &VTABLE);
    // SAFETY: `VTABLE` upholds the `RawWaker` contract over the `Arc`.
    unsafe { Waker::from_raw(raw) }
}

/// Parks `wait` with a waker whose destructor runs `reenter`, then has
/// the wait list drop it twice: displaced by a re-poll, then removed by
/// a cancel. `reenter` takes the primitive's lock, so either drop made
/// under that lock deadlocks.
fn drops_unlocked<F: Future>(
    mut wait: impl FnMut() -> Pin<Box<F>>,
    reenter: impl Fn() + Clone + Send + Sync + 'static,
) {
    let mut displaced = wait();
    assert!(poll(displaced.as_mut(), &on_drop(reenter.clone())).is_pending());
    assert!(poll(displaced.as_mut(), Waker::noop()).is_pending());
    let mut removed = wait();
    assert!(poll(removed.as_mut(), &on_drop(reenter)).is_pending());
    drop(removed);
}

#[test]
fn set_once_drops_wakers_outside_its_lock() {
    bounded(|| {
        let cell = Arc::new(SetOnce::<u8>::new());
        let reenter = cell.clone();
        drops_unlocked(
            || Box::pin(cell.wait()),
            move || {
                let _ = reenter.initialized();
            },
        );
    });
}

#[test]
fn once_cell_drops_wakers_outside_its_lock() {
    bounded(|| {
        let cell = Arc::new(OnceCell::<u8>::new());
        let _init = initializing(&cell);
        let reenter = cell.clone();
        drops_unlocked(
            || Box::pin(cell.get_or_init(|| async { 1 })),
            move || {
                let _ = reenter.initialized();
            },
        );
    });
}

#[test]
fn barrier_drops_wakers_outside_its_lock() {
    bounded(|| {
        let barrier = Arc::new(Barrier::new(100));
        let reenter = barrier.clone();
        // Barrier has no lock-taking accessor, so the destructor arrives
        // once more: its first poll takes the lock. 100 arrivals are
        // never reached.
        drops_unlocked(
            || Box::pin(barrier.wait()),
            move || {
                let mut arrival = Box::pin(reenter.wait());
                let _ = poll(arrival.as_mut(), Waker::noop());
            },
        );
    });
}

#[test]
fn set_once_clones_wakers_outside_its_lock() {
    bounded(|| {
        let cell = Arc::new(SetOnce::<u8>::new());
        let reenter = cell.clone();
        let waker = clone_hook(move || {
            let _ = reenter.initialized();
        });
        let mut wait = Box::pin(cell.wait());
        assert!(poll(wait.as_mut(), &waker).is_pending());
        assert!(poll(wait.as_mut(), &waker).is_pending());
    });
}
