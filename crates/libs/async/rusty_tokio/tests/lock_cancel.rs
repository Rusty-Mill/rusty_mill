//! Cancellation safety of `Mutex`, `Semaphore` and `RwLock` waiters
//! (#1064, #403): a dropped `lock()`/`acquire()`/`read()`/`write()` future
//! must neither swallow the wakeup meant for the waiters behind it nor
//! keep permits it was granted.
//! Polled by hand, so every interleaving is deterministic.

use rusty_tokio::sync::{Mutex, RwLock, Semaphore};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Wake, Waker};
use std::time::Duration;

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

/// A waker's destructor may re-enter the lock, so `lock()` must drop the
/// wakers it removes or replaces only after releasing its internal state.
#[test]
fn mutex_drops_removed_and_replaced_wakers_outside_its_state_lock() {
    let reentrant = |mutex: &Arc<Mutex<()>>| {
        let mutex = mutex.clone();
        on_drop(move || drop(mutex.try_lock()))
    };
    bounded(move || {
        let mutex = Arc::new(Mutex::new(()));
        let held = mutex.try_lock_owned().expect("free");
        let (_, plain) = flag();

        let mut cancelled = Box::pin(mutex.lock());
        assert!(poll(cancelled.as_mut(), &reentrant(&mutex)).is_pending());
        drop(cancelled);

        let mut repolled = Box::pin(mutex.lock());
        assert!(poll(repolled.as_mut(), &reentrant(&mutex)).is_pending());
        assert!(poll(repolled.as_mut(), &plain).is_pending());
        drop(repolled);

        let mut acquiring = Box::pin(mutex.lock());
        assert!(poll(acquiring.as_mut(), &reentrant(&mutex)).is_pending());
        drop(held);
        assert!(poll(acquiring.as_mut(), &plain).is_ready());
    });
}

/// As for `Mutex`: a cancelled or re-polled `acquire()` drops the waker
/// it gives up only after releasing the semaphore's state.
#[test]
fn semaphore_drops_removed_and_replaced_wakers_outside_its_state_lock() {
    let reentrant = |sem: &Arc<Semaphore>| {
        let sem = sem.clone();
        on_drop(move || sem.add_permits(1))
    };
    bounded(move || {
        let sem = Arc::new(Semaphore::new(0));
        let (_, plain) = flag();

        let mut cancelled = Box::pin(sem.acquire_many(100));
        assert!(poll(cancelled.as_mut(), &reentrant(&sem)).is_pending());
        drop(cancelled);

        let mut repolled = Box::pin(sem.acquire_many(100));
        assert!(poll(repolled.as_mut(), &reentrant(&sem)).is_pending());
        assert!(poll(repolled.as_mut(), &plain).is_pending());
        drop(repolled);
        assert_eq!(sem.available_permits(), 2, "both destructors ran");
    });
}

/// A waker whose clone runs `on_clone` once armed. That is legal for a
/// custom `RawWaker`, and only an executor's own wakers can do it.
struct CloneHook {
    armed: AtomicBool,
    on_clone: Box<dyn Fn() + Send + Sync>,
}

fn clone_hook(on_clone: impl Fn() + Send + Sync + 'static) -> (Arc<CloneHook>, Waker) {
    unsafe fn clone(data: *const ()) -> RawWaker {
        // SAFETY: `data` is a live `Arc<CloneHook>` from `Arc::into_raw`.
        let hook = unsafe { &*data.cast::<CloneHook>() };
        if hook.armed.load(Ordering::SeqCst) {
            (hook.on_clone)();
        }
        // SAFETY: as above; the new `RawWaker` owns one more count.
        unsafe { Arc::increment_strong_count(data.cast::<CloneHook>()) };
        RawWaker::new(data, &VTABLE)
    }
    unsafe fn release(data: *const ()) {
        // SAFETY: each `RawWaker` owns one count of the `Arc`.
        unsafe { Arc::decrement_strong_count(data.cast::<CloneHook>()) };
    }
    unsafe fn wake_by_ref(_: *const ()) {}
    static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, release, wake_by_ref, release);

    let hook = Arc::new(CloneHook {
        armed: AtomicBool::new(false),
        on_clone: Box::new(on_clone),
    });
    let raw = RawWaker::new(Arc::into_raw(hook.clone()).cast(), &VTABLE);
    // SAFETY: `VTABLE` upholds the `RawWaker` contract over the `Arc`.
    (hook, unsafe { Waker::from_raw(raw) })
}

/// A waker's clone may also re-enter the lock, so a queued waker is
/// never cloned while the mutex's state is held: not to wake it on
/// release, not to pass a cancelled head's wake on, not to re-poll.
#[test]
fn mutex_never_clones_a_waker_under_its_state_lock() {
    bounded(|| {
        let mutex = Arc::new(Mutex::new(()));
        let reenter = |mutex: &Arc<Mutex<()>>| {
            let mutex = mutex.clone();
            clone_hook(move || drop(mutex.try_lock()))
        };
        let (_, plain) = flag();

        let held = mutex.try_lock_owned().expect("free");
        let (hook, waker) = reenter(&mutex);
        let mut released_to = Box::pin(mutex.lock());
        assert!(poll(released_to.as_mut(), &waker).is_pending());
        hook.armed.store(true, Ordering::SeqCst);
        drop(held);
        assert!(poll(released_to.as_mut(), &plain).is_ready());

        let held = mutex.try_lock_owned().expect("free");
        let mut head = Box::pin(mutex.lock());
        let (hook, waker) = reenter(&mutex);
        let mut passed_to = Box::pin(mutex.lock());
        assert!(poll(head.as_mut(), &plain).is_pending());
        assert!(poll(passed_to.as_mut(), &waker).is_pending());
        hook.armed.store(true, Ordering::SeqCst);
        drop(held);
        drop(head);
        assert!(poll(passed_to.as_mut(), &plain).is_ready());

        let held = mutex.try_lock_owned().expect("free");
        let (hook, waker) = reenter(&mutex);
        let mut repolled = Box::pin(mutex.lock());
        assert!(poll(repolled.as_mut(), &plain).is_pending());
        hook.armed.store(true, Ordering::SeqCst);
        assert!(poll(repolled.as_mut(), &waker).is_pending());
        drop(held);
    });
}

/// As for `Mutex`: a re-polled `acquire()` clones its new waker only
/// with the semaphore's state released.
#[test]
fn semaphore_never_clones_a_waker_under_its_state_lock() {
    bounded(|| {
        let sem = Arc::new(Semaphore::new(0));
        let reenter = sem.clone();
        let (hook, waker) = clone_hook(move || drop(reenter.try_acquire()));
        let (_, plain) = flag();

        let mut repolled = Box::pin(sem.acquire());
        assert!(poll(repolled.as_mut(), &plain).is_pending());
        hook.armed.store(true, Ordering::SeqCst);
        assert!(poll(repolled.as_mut(), &waker).is_pending());
        sem.add_permits(1);
        assert!(matches!(
            poll(repolled.as_mut(), &waker),
            Poll::Ready(Ok(_))
        ));
    });
}

// RwLock (#403): the same rules as `Mutex` above. A waker's clone or
// drop may re-enter the lock, so neither happens with its state held.

fn rwlock_reentrant_on_drop(lock: &Arc<RwLock<()>>) -> Waker {
    let lock = lock.clone();
    on_drop(move || drop(lock.try_read()))
}

#[test]
fn rwlock_drops_removed_and_replaced_wakers_outside_its_state_lock() {
    bounded(|| {
        let lock = Arc::new(RwLock::new(()));
        let held = lock.clone().try_write_owned().expect("free");
        let (_, plain) = flag();

        let mut cancelled = Box::pin(lock.write());
        assert!(poll(cancelled.as_mut(), &rwlock_reentrant_on_drop(&lock)).is_pending());
        drop(cancelled);

        let mut repolled = Box::pin(lock.read());
        assert!(poll(repolled.as_mut(), &rwlock_reentrant_on_drop(&lock)).is_pending());
        assert!(poll(repolled.as_mut(), &plain).is_pending());
        drop(repolled);

        let mut acquiring = Box::pin(lock.read());
        assert!(poll(acquiring.as_mut(), &rwlock_reentrant_on_drop(&lock)).is_pending());
        drop(held);
        assert!(poll(acquiring.as_mut(), &plain).is_ready());
    });
}

#[test]
fn rwlock_never_clones_a_waker_under_its_state_lock() {
    bounded(|| {
        let lock = Arc::new(RwLock::new(()));
        let reenter = |lock: &Arc<RwLock<()>>| {
            let lock = lock.clone();
            clone_hook(move || drop(lock.try_read()))
        };
        let (_, plain) = flag();

        // Woken on release.
        let held = lock.clone().try_write_owned().expect("free");
        let (hook, waker) = reenter(&lock);
        let mut released_to = Box::pin(lock.read());
        assert!(poll(released_to.as_mut(), &waker).is_pending());
        hook.armed.store(true, Ordering::SeqCst);
        drop(held);
        let Poll::Ready(guard) = poll(released_to.as_mut(), &plain) else {
            panic!("the reader acquires once the writer releases");
        };
        drop(guard);

        // Woken when the waiter ahead of it is cancelled.
        let held = lock.clone().try_read_owned().expect("free");
        let mut head = Box::pin(lock.write());
        let (hook, waker) = reenter(&lock);
        let mut passed_to = Box::pin(lock.read());
        assert!(poll(head.as_mut(), &plain).is_pending());
        assert!(poll(passed_to.as_mut(), &waker).is_pending());
        hook.armed.store(true, Ordering::SeqCst);
        drop(head);
        drop(held);
        let Poll::Ready(guard) = poll(passed_to.as_mut(), &plain) else {
            panic!("the reader behind the cancelled writer acquires");
        };
        drop(guard);

        // Re-polled with a new waker.
        let held = lock.clone().try_write_owned().expect("free");
        let (hook, waker) = reenter(&lock);
        let mut repolled = Box::pin(lock.write());
        assert!(poll(repolled.as_mut(), &plain).is_pending());
        hook.armed.store(true, Ordering::SeqCst);
        assert!(poll(repolled.as_mut(), &waker).is_pending());
        drop(held);
    });
}

/// A waker whose destructor panics must not leave the lock unusable:
/// the destructor runs outside the state lock, so nothing is poisoned.
#[test]
fn rwlock_survives_a_panicking_waker_destructor() {
    let lock = Arc::new(RwLock::new(()));
    let held = lock.try_write().expect("free");
    let mut cancelled = Box::pin(lock.read());
    assert!(poll(cancelled.as_mut(), &on_drop(|| panic!("waker drop"))).is_pending());
    let dropped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(cancelled)));
    assert!(
        dropped.is_err(),
        "the destructor's panic reaches the dropper"
    );
    drop(held);
    let reader = lock.try_read().expect("the lock still works");
    assert!(lock.try_write().is_none(), "and still excludes");
    drop(reader);
    assert!(lock.try_write().is_some(), "and still releases");
}

/// A waiter that acquires while its stored waker's destructor panics
/// still ends up holding a guard, which releases the lock as the panic
/// unwinds; the lock is not left held with no guard. Run over each of
/// `read`, `write`, `read_owned` and `write_owned`, which each order the
/// drop themselves.
fn acquire_with_a_panicking_waker_destructor<F: Future>(
    lock: &Arc<RwLock<()>>,
    acquire: impl FnOnce() -> Pin<Box<F>>,
) {
    let held = lock.try_write().expect("free");
    let mut waiter = acquire();
    assert!(poll(waiter.as_mut(), &on_drop(|| panic!("waker drop"))).is_pending());
    drop(held);
    let (_, plain) = flag();
    let acquired = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = poll(waiter.as_mut(), &plain);
    }));
    assert!(
        acquired.is_err(),
        "the destructor's panic reaches the poller"
    );
    drop(waiter);
    assert!(lock.try_write().is_some(), "nothing still holds the lock");
}

#[test]
fn rwlock_acquire_with_a_panicking_waker_destructor_leaves_the_lock_releasable() {
    let lock = Arc::new(RwLock::new(()));
    acquire_with_a_panicking_waker_destructor(&lock, || Box::pin(lock.read()));
    acquire_with_a_panicking_waker_destructor(&lock, || Box::pin(lock.write()));
    acquire_with_a_panicking_waker_destructor(&lock, || Box::pin(lock.read_owned()));
    acquire_with_a_panicking_waker_destructor(&lock, || Box::pin(lock.write_owned()));
}

/// A cancelled waiter whose waker destructor panics still wakes the
/// waiters its leaving made eligible.
#[test]
fn rwlock_cancel_with_a_panicking_waker_destructor_still_passes_the_wake_on() {
    let lock = Arc::new(RwLock::new(()));
    let held = lock.try_write().expect("free");
    let mut writer = Box::pin(lock.write());
    assert!(poll(writer.as_mut(), &on_drop(|| panic!("waker drop"))).is_pending());
    let (reader_flag, reader_waker) = flag();
    let mut reader = Box::pin(lock.read());
    assert!(poll(reader.as_mut(), &reader_waker).is_pending());
    drop(held);
    let cancelled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(writer)));
    assert!(
        cancelled.is_err(),
        "the destructor's panic reaches the dropper"
    );
    assert!(reader_flag.woken(), "the reader behind the writer is woken");
    assert!(poll(reader.as_mut(), &reader_waker).is_ready());
}
