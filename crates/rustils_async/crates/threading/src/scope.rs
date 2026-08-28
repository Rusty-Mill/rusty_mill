//! Thread lifecycle: scoped spawn with a decoded join outcome — the
//! same "decode the raw status, never let a raw variant cross the
//! boundary" discipline `platform::process::ExitStatus` applies to
//! child processes (rustils RFC v2 §5.4, pinning bug B-5), applied here
//! to threads instead of processes.

use std::any::Any;
use std::thread::Scope as StdScope;

/// How a spawned thread's closure terminated.
#[derive(Debug)]
pub enum JoinOutcome<T> {
    /// The closure returned normally.
    Completed(T),
    /// The closure panicked; the payload is whatever `panic!` (or a
    /// custom panic hook) produced — the same opaque `Any` payload
    /// `std::thread::Result` already carries, not reinterpreted here.
    Panicked(Box<dyn Any + Send + 'static>),
}

impl<T> JoinOutcome<T> {
    pub fn completed(self) -> Option<T> {
        match self {
            JoinOutcome::Completed(v) => Some(v),
            JoinOutcome::Panicked(_) => None,
        }
    }

    pub fn is_completed(&self) -> bool {
        matches!(self, JoinOutcome::Completed(_))
    }
}

/// A spawned scoped thread's handle. Wraps
/// [`std::thread::ScopedJoinHandle`] so `join` returns a decoded
/// [`JoinOutcome`] instead of `std::thread::Result`.
pub struct ScopedJoinHandle<'scope, T>(std::thread::ScopedJoinHandle<'scope, T>);

impl<'scope, T> ScopedJoinHandle<'scope, T> {
    pub fn join(self) -> JoinOutcome<T> {
        match self.0.join() {
            Ok(v) => JoinOutcome::Completed(v),
            Err(payload) => JoinOutcome::Panicked(payload),
        }
    }

    pub fn thread(&self) -> &std::thread::Thread {
        self.0.thread()
    }
}

/// A scope in which threads may be spawned, all guaranteed joined
/// before [`scope`] returns — same guarantee as `std::thread::scope`,
/// wrapped only to hand back [`ScopedJoinHandle`] instead of the std
/// type.
pub struct Scope<'scope, 'env: 'scope>(&'scope StdScope<'scope, 'env>);

impl<'scope, 'env> Scope<'scope, 'env> {
    pub fn spawn<F, T>(&self, f: F) -> ScopedJoinHandle<'scope, T>
    where
        F: FnOnce() -> T + Send + 'scope,
        T: Send + 'scope,
    {
        ScopedJoinHandle(self.0.spawn(f))
    }
}

/// Run `f` with a fresh scope; every thread spawned via the scope is
/// joined before this function returns, whether `f` returns normally or
/// unwinds — same guarantee `std::thread::scope` provides.
pub fn scope<'env, F, T>(f: F) -> T
where
    F: for<'scope> FnOnce(&Scope<'scope, 'env>) -> T,
{
    std::thread::scope(|s| f(&Scope(s)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawned_thread_completes_normally() {
        let result = scope(|s| {
            let handle = s.spawn(|| 2 + 2);
            handle.join()
        });
        assert_eq!(result.completed(), Some(4));
    }

    #[test]
    fn spawned_thread_panic_is_decoded_not_propagated() {
        // The default panic hook still prints to stderr here — that is
        // expected and independent of whether the panic is propagated
        // to this test's own thread (it is not).
        let outcome = scope(|s| {
            let handle = s.spawn(|| -> i32 { panic!("boom") });
            handle.join()
        });
        assert!(!outcome.is_completed());
        assert!(matches!(outcome, JoinOutcome::Panicked(_)));
    }

    #[test]
    fn multiple_threads_all_join_before_scope_returns() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let counter = AtomicUsize::new(0);
        scope(|s| {
            let handles: Vec<_> = (0..8)
                .map(|_| s.spawn(|| counter.fetch_add(1, Ordering::SeqCst)))
                .collect();
            for h in handles {
                h.join();
            }
        });
        assert_eq!(counter.load(Ordering::SeqCst), 8);
    }
}
