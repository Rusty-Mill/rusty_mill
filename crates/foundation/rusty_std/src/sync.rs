//! Sovereign Synchronization primitives for rusty_std.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

/// A mutual exclusion lock.
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

    /// Acquires the lock, blocking until available.
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
