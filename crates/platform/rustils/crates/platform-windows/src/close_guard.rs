//! The pseudo console's close guard, kept free of OS calls so its
//! ordering can be tested on every host with injected, deterministic
//! blocking (`ClosePseudoConsole` blocks on some Windows builds and not on
//! others, so the live tests alone cannot cover both).
//!
//! Three operations share it: the close that must run at most once
//! ([`CloseGuard::close_once`], used by the exit watcher and
//! `WindowsPtyMaster::hang_up`), an operation that must never reach a
//! closed console ([`CloseGuard::while_open`], used by `resize`), and the
//! owner's teardown ([`CloseGuard::close_unless_busy`], used by `Drop`),
//! which must never wait on a close that may itself be blocked.

use std::sync::{Mutex, MutexGuard, PoisonError, TryLockError};

/// Why [`CloseGuard::while_open`] did not run its operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// The close has already run.
    Closed,
    /// Another holder (a close in progress, or another guarded operation)
    /// has the lock.
    Busy,
}

/// `true` once the close has run, behind a lock that every close and every
/// guarded operation holds for its whole duration.
///
/// A poisoned lock is read as its inner value: the flag is only set after
/// a close returns, so a panic never leaves it claiming a close that did
/// not happen.
#[derive(Debug, Default)]
pub struct CloseGuard {
    closed: Mutex<bool>,
}

impl CloseGuard {
    /// An open guard.
    pub fn new() -> Self {
        Self::default()
    }

    /// Run `close` unless it has already run, waiting for any other holder
    /// first. Exactly one `close` runs across all callers.
    pub fn close_once(&self, close: impl FnOnce()) {
        let mut closed = self.closed.lock().unwrap_or_else(PoisonError::into_inner);
        if *closed {
            return;
        }
        close();
        *closed = true;
    }

    /// Run `op` while holding the lock, if the close has not run and no one
    /// else holds it. Never waits.
    pub fn while_open<T>(&self, op: impl FnOnce() -> T) -> Result<T, Refused> {
        let closed = self.try_lock().ok_or(Refused::Busy)?;
        if *closed {
            return Err(Refused::Closed);
        }
        Ok(op())
    }

    /// Run `close` if the close has not run and the lock is free; return at
    /// once without running it if someone else holds the lock. For an
    /// owner's teardown: the holder is then a close in progress (the owner
    /// has exclusive access, so no guarded operation of its own can be
    /// running), which may be blocked until the owner releases its
    /// resources, so waiting for it could deadlock.
    pub fn close_unless_busy(&self, close: impl FnOnce()) {
        let Some(mut closed) = self.try_lock() else {
            return;
        };
        if *closed {
            return;
        }
        close();
        *closed = true;
    }

    fn try_lock(&self) -> Option<MutexGuard<'_, bool>> {
        match self.closed.try_lock() {
            Ok(guard) => Some(guard),
            Err(TryLockError::Poisoned(p)) => Some(p.into_inner()),
            Err(TryLockError::WouldBlock) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    const BUDGET: Duration = Duration::from_secs(5);

    /// Starts a `close_once` on another thread whose close blocks until the
    /// returned sender is used (or dropped), the way `ClosePseudoConsole`
    /// blocks on a full pipe on older Windows builds. Returns once the
    /// close is running.
    fn blocked_close(
        guard: &Arc<CloseGuard>,
        runs: &Arc<AtomicUsize>,
    ) -> (mpsc::Sender<()>, thread::JoinHandle<()>) {
        let (release, released) = mpsc::channel::<()>();
        let (entered, is_running) = mpsc::channel();
        let (guard, runs) = (Arc::clone(guard), Arc::clone(runs));
        let closer = thread::spawn(move || {
            guard.close_once(|| {
                runs.fetch_add(1, Ordering::SeqCst);
                entered.send(()).expect("test alive");
                let _ = released.recv();
            });
        });
        is_running.recv_timeout(BUDGET).expect("close started");
        (release, closer)
    }

    /// Runs `f` on its own thread and returns its result, failing if it
    /// takes longer than `BUDGET`: a wait that should not happen shows up
    /// as a named failure, not a hung test.
    fn returns_promptly<T: Send + 'static>(
        what: &str,
        f: impl FnOnce() -> T + Send + 'static,
    ) -> T {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(f());
        });
        rx.recv_timeout(BUDGET)
            .unwrap_or_else(|_| panic!("{what} waited on the blocked close"))
    }

    #[test]
    fn close_once_runs_one_close_across_threads() {
        let guard = Arc::new(CloseGuard::new());
        let runs = Arc::new(AtomicUsize::new(0));
        let closers: Vec<_> = (0..8)
            .map(|_| {
                let (guard, runs) = (Arc::clone(&guard), Arc::clone(&runs));
                thread::spawn(move || {
                    guard.close_once(|| {
                        runs.fetch_add(1, Ordering::SeqCst);
                    })
                })
            })
            .collect();
        for closer in closers {
            closer.join().expect("closer");
        }
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn while_open_runs_until_closed_then_refuses() {
        let guard = CloseGuard::new();
        assert_eq!(guard.while_open(|| 7), Ok(7));
        guard.close_once(|| {});
        assert_eq!(guard.while_open(|| 7), Err(Refused::Closed));
    }

    #[test]
    fn concurrent_resize_close_and_drop_with_a_blocking_close() {
        let guard = Arc::new(CloseGuard::new());
        let runs = Arc::new(AtomicUsize::new(0));
        let (release, closer) = blocked_close(&guard, &runs);

        // A resize during the close is refused without waiting and never
        // runs its operation.
        let resized = Arc::new(AtomicUsize::new(0));
        let (g, r) = (Arc::clone(&guard), Arc::clone(&resized));
        let refused = returns_promptly("resize", move || {
            g.while_open(|| {
                r.fetch_add(1, Ordering::SeqCst);
            })
        });
        assert_eq!(refused, Err(Refused::Busy));

        // The owner's teardown returns without running a second close.
        let (g, n) = (Arc::clone(&guard), Arc::clone(&runs));
        returns_promptly("drop", move || {
            g.close_unless_busy(|| {
                n.fetch_add(1, Ordering::SeqCst);
            })
        });

        // A second hang-up waits for the first, then does nothing.
        let (g, n) = (Arc::clone(&guard), Arc::clone(&runs));
        let second = thread::spawn(move || {
            g.close_once(|| {
                n.fetch_add(1, Ordering::SeqCst);
            })
        });

        release.send(()).expect("closer alive");
        closer.join().expect("closer");
        second.join().expect("second close");
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert_eq!(resized.load(Ordering::SeqCst), 0);
        assert_eq!(guard.while_open(|| ()), Err(Refused::Closed));
    }

    #[test]
    fn close_unless_busy_closes_when_free_and_later_closes_are_no_ops() {
        let guard = CloseGuard::new();
        let mut runs = 0;
        guard.close_unless_busy(|| runs += 1);
        guard.close_once(|| runs += 1);
        guard.close_unless_busy(|| runs += 1);
        assert_eq!(runs, 1);
    }

    #[test]
    fn a_resize_in_progress_makes_a_close_wait_not_skip() {
        let guard = Arc::new(CloseGuard::new());
        let (release, released) = mpsc::channel::<()>();
        let (entered, is_resizing) = mpsc::channel();
        let g = Arc::clone(&guard);
        let resizer = thread::spawn(move || {
            g.while_open(|| {
                entered.send(()).expect("test alive");
                let _ = released.recv();
            })
        });
        is_resizing.recv_timeout(BUDGET).expect("resize started");

        let runs = Arc::new(AtomicUsize::new(0));
        let (g, n) = (Arc::clone(&guard), Arc::clone(&runs));
        let closer = thread::spawn(move || {
            g.close_once(|| {
                n.fetch_add(1, Ordering::SeqCst);
            })
        });
        thread::sleep(Duration::from_millis(50));
        assert_eq!(
            runs.load(Ordering::SeqCst),
            0,
            "the close ran during the resize"
        );

        release.send(()).expect("resizer alive");
        assert_eq!(resizer.join().expect("resizer"), Ok(()));
        closer.join().expect("closer");
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_panic_under_the_lock_does_not_mark_the_console_closed() {
        let guard = Arc::new(CloseGuard::new());
        let g = Arc::clone(&guard);
        let panicked = thread::spawn(move || {
            let _ = g.while_open(|| panic!("injected"));
        })
        .join();
        assert!(panicked.is_err());

        let mut runs = 0;
        guard.close_once(|| runs += 1);
        assert_eq!(runs, 1);
    }
}
