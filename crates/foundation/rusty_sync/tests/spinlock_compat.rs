use core::cell::Cell;
use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::task::{Context, Poll, Waker};
use rusty_std::sync::{Mutex, MutexGuard};
use rusty_sync::{SpinLock, SpinLockGuard};
use std::sync::Arc;

static STD_STATIC: Mutex<usize> = Mutex::new(0);
static SYNC_STATIC: SpinLock<usize> = SpinLock::new(0);

trait Identity {
    fn identity(&self) -> &'static str;
}

impl<T> Identity for Mutex<T> {
    fn identity(&self) -> &'static str {
        "rusty_std lock"
    }
}

impl<T> Identity for SpinLock<T> {
    fn identity(&self) -> &'static str {
        "rusty_sync lock"
    }
}

impl<T> Identity for MutexGuard<'_, T> {
    fn identity(&self) -> &'static str {
        "rusty_std guard"
    }
}

impl<T> Identity for SpinLockGuard<'_, T> {
    fn identity(&self) -> &'static str {
        "rusty_sync guard"
    }
}

fn assert_send<T: Send>() {}
fn assert_sync<T: Sync>() {}

#[allow(drop_bounds)]
fn assert_drop<T: Drop>() {}

#[test]
fn public_paths_remain_nominally_distinct_for_local_trait_coherence() {
    let std_guard: MutexGuard<'_, usize> = STD_STATIC.lock();
    let sync_guard: SpinLockGuard<'_, usize> = SYNC_STATIC.lock();

    assert_eq!(STD_STATIC.identity(), "rusty_std lock");
    assert_eq!(SYNC_STATIC.identity(), "rusty_sync lock");
    assert_eq!(std_guard.identity(), "rusty_std guard");
    assert_eq!(sync_guard.identity(), "rusty_sync guard");
}

#[test]
fn const_constructors_mutation_and_reacquisition_work_through_both_paths() {
    let _std_const = const { Mutex::new(0usize) };
    let _sync_const = const { SpinLock::new(0usize) };
    let std_lock = Mutex::new(1usize);
    let sync_lock = SpinLock::new(1usize);

    *std_lock.lock() += 1;
    *sync_lock.lock() += 1;
    assert_eq!(*std_lock.lock(), 2);
    assert_eq!(*sync_lock.lock(), 2);
}

struct DropCounted(Arc<AtomicUsize>);

impl Drop for DropCounted {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn guard_release_does_not_drop_payload_and_lock_drops_it_once() {
    for compatibility_path in [false, true] {
        let drops = Arc::new(AtomicUsize::new(0));
        if compatibility_path {
            let lock = SpinLock::new(DropCounted(drops.clone()));
            drop(lock.lock());
            assert_eq!(drops.load(Ordering::SeqCst), 0);
            drop(lock);
        } else {
            let lock = Mutex::new(DropCounted(drops.clone()));
            drop(lock.lock());
            assert_eq!(drops.load(Ordering::SeqCst), 0);
            drop(lock);
        }
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn contended_non_atomic_payloads_reach_exact_totals() {
    let std_lock = Mutex::new(0usize);
    let sync_lock = SpinLock::new(0usize);
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| {
                for _ in 0..500 {
                    *std_lock.lock() += 1;
                    *sync_lock.lock() += 1;
                }
            });
        }
    });
    assert_eq!(*std_lock.lock(), 2_000);
    assert_eq!(*sync_lock.lock(), 2_000);
}

#[cfg(panic = "unwind")]
#[test]
fn unwinding_releases_both_locks_without_rolling_back_mutation() {
    let std_lock = Mutex::new(0usize);
    let sync_lock = SpinLock::new(0usize);
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut guard = std_lock.lock();
            *guard = 1;
            panic!("canonical guard unwinds");
        }))
        .is_err()
    );
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut guard = sync_lock.lock();
            *guard = 1;
            panic!("facade guard unwinds");
        }))
        .is_err()
    );
    assert_eq!(*std_lock.lock(), 1);
    assert_eq!(*sync_lock.lock(), 1);
}

struct PendingWithGuard<G> {
    _guard: G,
}

impl<G> Future for PendingWithGuard<G> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
        Poll::Pending
    }
}

#[test]
fn cancelling_polled_futures_by_drop_releases_both_guards() {
    let std_lock = Mutex::new(0usize);
    let sync_lock = SpinLock::new(0usize);
    let mut context = Context::from_waker(Waker::noop());

    let mut std_future = Box::pin(PendingWithGuard {
        _guard: std_lock.lock(),
    });
    assert!(std_future.as_mut().poll(&mut context).is_pending());
    drop(std_future);
    drop(std_lock.lock());

    let mut sync_future = Box::pin(PendingWithGuard {
        _guard: sync_lock.lock(),
    });
    assert!(sync_future.as_mut().poll(&mut context).is_pending());
    drop(sync_future);
    drop(sync_lock.lock());
}

#[test]
fn auto_traits_and_explicit_drop_contracts_are_preserved() {
    assert_send::<Mutex<Cell<u32>>>();
    assert_sync::<Mutex<Cell<u32>>>();
    assert_send::<MutexGuard<'static, Cell<u32>>>();
    assert_sync::<MutexGuard<'static, std::sync::MutexGuard<'static, ()>>>();
    assert_send::<SpinLock<Cell<u32>>>();
    assert_sync::<SpinLock<Cell<u32>>>();
    assert_send::<SpinLockGuard<'static, Cell<u32>>>();
    assert_sync::<SpinLockGuard<'static, std::sync::MutexGuard<'static, ()>>>();
    assert_drop::<MutexGuard<'static, ()>>();
    assert_drop::<SpinLockGuard<'static, ()>>();
}

#[test]
fn shared_atomic_guards_and_moved_guards_work_for_both_paths() {
    let std_atomic = Mutex::new(AtomicUsize::new(0));
    let sync_atomic = SpinLock::new(AtomicUsize::new(0));
    let std_guard = std_atomic.lock();
    let sync_guard = sync_atomic.lock();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            std_guard.fetch_add(1, Ordering::SeqCst);
            sync_guard.fetch_add(1, Ordering::SeqCst);
        });
    });
    assert_eq!(std_guard.load(Ordering::SeqCst), 1);
    assert_eq!(sync_guard.load(Ordering::SeqCst), 1);
    drop(std_guard);
    drop(sync_guard);

    let std_guard = std_atomic.lock();
    let sync_guard = sync_atomic.lock();
    std::thread::scope(|scope| {
        scope.spawn(move || drop(std_guard));
        scope.spawn(move || drop(sync_guard));
    });
    drop(std_atomic.lock());
    drop(sync_atomic.lock());
}
