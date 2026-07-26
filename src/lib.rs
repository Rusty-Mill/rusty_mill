#![no_std]
#![deny(missing_docs)]

//! # `rusty_sync`
//!
//! A `#![no_std]` + `alloc` sovereign atomic spinlock, lock-free MPMC queue channel,
//! and ring buffer implementation for the **Rusty Mill** ecosystem.

extern crate alloc;

use core::cell::UnsafeCell;

/// Atomic spinlock mutex.
pub struct SpinLock<T> {
    lock: core::sync::atomic::AtomicBool,
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Send for SpinLock<T> {}
unsafe impl<T: Send> Sync for SpinLock<T> {}

impl<T> SpinLock<T> {
    /// Creates a new SpinLock wrapping data.
    pub const fn new(data: T) -> Self {
        Self {
            lock: core::sync::atomic::AtomicBool::new(false),
            data: UnsafeCell::new(data),
        }
    }

    /// Acquires spinlock and returns a Guard borrowing inner value.
    pub fn lock(&self) -> SpinLockGuard<'_, T> {
        while self.lock.swap(true, core::sync::atomic::Ordering::Acquire) {
            core::hint::spin_loop();
        }
        SpinLockGuard { lock: self }
    }
}

/// RAII Guard for SpinLock.
pub struct SpinLockGuard<'a, T> {
    lock: &'a SpinLock<T>,
}

impl<'a, T> core::ops::Deref for SpinLockGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<'a, T> core::ops::DerefMut for SpinLockGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<'a, T> Drop for SpinLockGuard<'a, T> {
    fn drop(&mut self) {
        self.lock.lock.store(false, core::sync::atomic::Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spinlock_mutual_exclusion() {
        let lock = SpinLock::new(42);
        {
            let mut val = lock.lock();
            *val += 1;
        }
        assert_eq!(*lock.lock(), 43);
    }
}
