//! A poison-free mutex over [`std::sync::Mutex`].
//!
//! The grid, parser and termios state are shared between the render loop,
//! reader tasks and signal paths. If one of them panics while holding a lock,
//! the others must still be able to take it, restore the host terminal and
//! exit cleanly. So [`Mutex::lock`] recovers a poisoned lock instead of
//! returning an error, which is the semantics this crate had with
//! `parking_lot` before.

use std::sync::{MutexGuard, PoisonError};

/// [`std::sync::Mutex`] whose [`lock`](Self::lock) never fails: a lock
/// poisoned by a panicking holder is recovered as-is.
#[derive(Debug, Default)]
pub struct Mutex<T: ?Sized>(std::sync::Mutex<T>);

impl<T> Mutex<T> {
    /// Creates an unlocked mutex holding `value`.
    pub const fn new(value: T) -> Self {
        Self(std::sync::Mutex::new(value))
    }
}

impl<T: ?Sized> Mutex<T> {
    /// Blocks until the lock is held, recovering it if a previous holder
    /// panicked.
    pub fn lock(&self) -> MutexGuard<'_, T> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::Mutex;
    use std::sync::Arc;

    #[test]
    fn lock_reads_and_writes() {
        let m = Mutex::new(1);
        *m.lock() += 1;
        assert_eq!(*m.lock(), 2);
    }

    #[test]
    fn lock_recovers_after_a_panicking_holder() {
        let m = Arc::new(Mutex::new(0));
        let held = Arc::clone(&m);
        let panicked = std::thread::spawn(move || {
            let mut g = held.lock();
            *g = 7;
            panic!("holder dies with the lock");
        })
        .join();
        assert!(panicked.is_err());
        assert_eq!(*m.lock(), 7);
    }
}
