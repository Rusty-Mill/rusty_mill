//! A poison-free mutex over [`std::sync::Mutex`] that lets bulk writers step
//! aside for waiters.
//!
//! The grid, parser and termios state are shared between the render loop,
//! reader tasks and signal paths. If one of them panics while holding a lock,
//! the others must still be able to take it, restore the host terminal and
//! exit cleanly. So [`Mutex::lock`] recovers a poisoned lock instead of
//! returning an error, which is the semantics this crate had with
//! `parking_lot` before.
//!
//! `std`'s mutex is not fair. A PTY reader that unlocks the grid and re-locks
//! it for the next chunk wins nearly every time: the waiter it woke on unlock
//! runs tens of microseconds later and finds the lock taken again. The render
//! thread then waits hundreds of milliseconds for a 0.1 ms snapshot. So
//! [`Mutex::lock`] counts its waiters, and loops that re-lock back to back
//! take [`Mutex::lock_bulk`] instead, which yields while anyone is waiting.
//! This bounded, best-effort deferral does not guarantee starvation freedom.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{MutexGuard, PoisonError, TryLockError};
use std::thread;
use std::time::{Duration, Instant};

/// Longest [`Mutex::lock_bulk`] defers to waiters before contending anyway,
/// to bound voluntary deferral even when a caller locks in a loop.
const BULK_DEFER_MAX: Duration = Duration::from_millis(1);

/// [`std::sync::Mutex`] whose [`lock`](Self::lock) never fails: a lock
/// poisoned by a panicking holder is recovered as-is.
#[derive(Debug, Default)]
pub struct Mutex<T: ?Sized> {
    /// [`lock`](Self::lock) callers blocked on the lock right now.
    waiting: AtomicUsize,
    inner: std::sync::Mutex<T>,
}

impl<T> Mutex<T> {
    /// Creates an unlocked mutex holding `value`.
    pub const fn new(value: T) -> Self {
        Self {
            waiting: AtomicUsize::new(0),
            inner: std::sync::Mutex::new(value),
        }
    }
}

impl<T: ?Sized> Mutex<T> {
    /// Blocks until the lock is held, recovering it if a previous holder
    /// panicked. While blocked, [`lock_bulk`](Self::lock_bulk) callers yield
    /// to this one.
    pub fn lock(&self) -> MutexGuard<'_, T> {
        match self.inner.try_lock() {
            Ok(g) => return g,
            Err(TryLockError::Poisoned(e)) => return e.into_inner(),
            Err(TryLockError::WouldBlock) => {}
        }
        self.waiting.fetch_add(1, Ordering::Relaxed);
        let g = self.raw_lock();
        self.waiting.fetch_sub(1, Ordering::Relaxed);
        g
    }

    /// [`lock`](Self::lock) for loops that re-lock back to back (a PTY
    /// reader taking the grid per chunk): first yields while a `lock` caller
    /// is waiting, for at most [`BULK_DEFER_MAX`], so that caller usually
    /// gets in. Best effort, bounded: a waiter the scheduler has not run
    /// within that bound still loses, so this is not a fairness guarantee.
    pub fn lock_bulk(&self) -> MutexGuard<'_, T> {
        let start = Instant::now();
        while self.waiting.load(Ordering::Relaxed) > 0 && start.elapsed() < BULK_DEFER_MAX {
            thread::yield_now();
        }
        self.raw_lock()
    }

    fn raw_lock(&self) -> MutexGuard<'_, T> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::Mutex;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    #[test]
    fn lock_reads_and_writes() {
        let m = Mutex::new(1);
        *m.lock() += 1;
        *m.lock_bulk() += 1;
        assert_eq!(*m.lock(), 3);
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
        assert_eq!(*m.lock_bulk(), 7);
    }

    #[test]
    fn waiting_count_returns_to_zero() {
        let m = Arc::new(Mutex::new(()));
        let g = m.lock();
        let waiter = {
            let m = Arc::clone(&m);
            thread::spawn(move || drop(m.lock()))
        };
        while m.waiting.load(Ordering::Relaxed) == 0 {
            thread::yield_now();
        }
        drop(g);
        waiter.join().expect("waiter");
        assert_eq!(m.waiting.load(Ordering::Relaxed), 0);
    }

    /// A bulk writer that re-locks back to back, holding ~0.5 ms each time,
    /// must let a `lock` caller in promptly, and must keep making progress
    /// itself while that caller locks in a loop.
    #[test]
    fn bulk_writer_yields_to_waiters_and_still_progresses() {
        let m = Arc::new(Mutex::new(()));
        let done = Arc::new(AtomicBool::new(false));
        let chunks = Arc::new(AtomicUsize::new(0));
        let writer = {
            let (m, done, chunks) = (Arc::clone(&m), Arc::clone(&done), Arc::clone(&chunks));
            thread::spawn(move || {
                while !done.load(Ordering::Relaxed) {
                    let _g = m.lock_bulk();
                    let t = Instant::now();
                    while t.elapsed() < Duration::from_micros(500) {
                        std::hint::spin_loop();
                    }
                    chunks.fetch_add(1, Ordering::Relaxed);
                }
            })
        };
        let mut worst = Duration::ZERO;
        for _ in 0..50 {
            let t = Instant::now();
            drop(m.lock());
            worst = worst.max(t.elapsed());
            thread::sleep(Duration::from_millis(2));
        }
        let before = chunks.load(Ordering::Relaxed);
        thread::sleep(Duration::from_millis(20));
        let progressed = chunks.load(Ordering::Relaxed) > before;
        done.store(true, Ordering::Relaxed);
        writer.join().expect("writer");
        // Without the deferral the waiter loses to the writer for hundreds
        // of milliseconds; with it, one hold plus scheduling. Generous for CI.
        assert!(worst < Duration::from_millis(50), "worst wait {worst:?}");
        assert!(progressed, "bulk writer stalled");
    }
}
