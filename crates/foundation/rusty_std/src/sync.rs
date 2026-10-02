//! Sovereign Synchronization primitives for rusty_std.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

/// A synchronous, non-poisoning spin-based mutual exclusion lock.
///
/// Acquisition repeatedly performs a test-and-set operation and does not
/// promise fairness, FIFO ordering, starvation freedom, or a bounded wait.
/// Callers must avoid reentrant acquisition and ensure that the current holder
/// can make progress. In particular, this lock does not mask interrupts or
/// preemption and should not be held across suspension points.
///
/// Unwinding releases a held lock when the guard is dropped, without rolling
/// back payload mutations. Aborting or leaking a guard need not release it.
/// The implementation is allocation-free and uses only `core` primitives.
///
/// A lock containing a non-`Send` value is neither `Send` nor `Sync`:
///
/// ```compile_fail
/// fn assert_send<T: Send>() {}
/// assert_send::<rusty_std::sync::Mutex<alloc::rc::Rc<()>>>();
/// ```
///
/// ```compile_fail
/// fn assert_sync<T: Sync>() {}
/// assert_sync::<rusty_std::sync::Mutex<alloc::rc::Rc<()>>>();
/// ```
pub struct Mutex<T> {
    locked: AtomicBool,
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Send for Mutex<T> {}
unsafe impl<T: Send> Sync for Mutex<T> {}

impl<T> Mutex<T> {
    /// Creates a new Mutex wrapping `data`.
    pub const fn new(data: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            data: UnsafeCell::new(data),
        }
    }

    /// Acquires the lock by synchronously spinning until it is available.
    pub fn lock(&self) -> MutexGuard<'_, T> {
        while self.locked.swap(true, Ordering::Acquire) {
            core::hint::spin_loop();
        }
        MutexGuard { lock: self }
    }
}

/// RAII structure used to release the exclusive lock when dropped.
///
/// The guard is `Sync` only when `T: Sync`; a guard over a `!Sync` payload
/// cannot be shared across threads:
///
/// ```compile_fail
/// fn assert_sync<T: Sync>() {}
/// assert_sync::<rusty_std::sync::MutexGuard<'static, core::cell::Cell<u32>>>();
/// ```
///
/// A guard is `Send` only when its payload is `Send`:
///
/// ```compile_fail
/// fn assert_send<T: Send>() {}
/// assert_send::<rusty_std::sync::MutexGuard<'static, alloc::rc::Rc<()>>>();
/// ```
pub struct MutexGuard<'a, T> {
    lock: &'a Mutex<T>,
}

// SAFETY: the only field is `&Mutex<T>`, and `Mutex<T>: Sync` needs just
// `T: Send`. Auto-derivation would therefore make the guard `Sync` for a
// `Send + !Sync` payload such as `Cell`, letting threads share `&guard` and
// obtain concurrent `&T` through `Deref`. This explicit impl replaces that
// auto impl and requires `T: Sync`, the bound for sound concurrent `&T`.
unsafe impl<'a, T: Sync> Sync for MutexGuard<'a, T> {}

impl<'a, T> core::ops::Deref for MutexGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<'a, T> core::ops::DerefMut for MutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<'a, T> Drop for MutexGuard<'a, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::cell::Cell;
    use core::future::Future;
    use core::pin::Pin;
    use core::sync::atomic::AtomicUsize;
    use core::task::{Context, Poll, Waker};

    static STATIC_LOCK: Mutex<u32> = Mutex::new(0);

    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}

    #[allow(drop_bounds)]
    fn assert_drop<T: Drop>() {}

    #[test]
    fn constructor_mutation_and_drop_release() {
        let _const_constructed = const { Mutex::new(0u32) };
        let mut guard = STATIC_LOCK.lock();
        *guard += 1;
        drop(guard);
        assert_eq!(*STATIC_LOCK.lock(), 1);
    }

    #[test]
    fn traits_match_the_payload_contract() {
        assert_send::<Mutex<Cell<u32>>>();
        assert_sync::<Mutex<Cell<u32>>>();
        assert_send::<MutexGuard<'static, Cell<u32>>>();
        assert_sync::<MutexGuard<'static, AtomicUsize>>();
        assert_sync::<MutexGuard<'static, std::sync::MutexGuard<'static, ()>>>();
        assert_drop::<MutexGuard<'static, ()>>();
    }

    #[test]
    fn contended_non_atomic_updates_have_an_exact_total() {
        let lock = Mutex::new(0usize);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    for _ in 0..1_000 {
                        *lock.lock() += 1;
                    }
                });
            }
        });
        assert_eq!(*lock.lock(), 8_000);
    }

    #[cfg(panic = "unwind")]
    #[test]
    fn unwind_releases_without_poisoning_or_rollback() {
        let lock = Mutex::new(1usize);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            *lock.lock() = 2;
            panic!("release the guard while unwinding");
        }));
        assert!(result.is_err());
        assert_eq!(*lock.lock(), 2);
    }

    struct HeldGuardFuture<'a> {
        _guard: MutexGuard<'a, usize>,
    }

    impl Future for HeldGuardFuture<'_> {
        type Output = ();

        fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
            Poll::Pending
        }
    }

    #[test]
    fn dropping_a_polled_future_drops_its_held_guard() {
        let lock = Mutex::new(7usize);
        let mut future = std::boxed::Box::pin(HeldGuardFuture {
            _guard: lock.lock(),
        });
        let mut context = Context::from_waker(Waker::noop());
        assert!(future.as_mut().poll(&mut context).is_pending());
        drop(future);
        assert_eq!(*lock.lock(), 7);
    }

    #[test]
    fn eligible_guard_can_move_and_drop_on_a_scoped_thread() {
        let lock = Mutex::new(1usize);
        let guard = lock.lock();
        std::thread::scope(|scope| {
            scope.spawn(move || drop(guard)).join().unwrap();
        });
        assert_eq!(*lock.lock(), 1);
    }
}
