//! `Mutex`/`RwLock` with an explicit poisoning policy (Atlas
//! `ATLAS-STATE-0001`: shared mutable state must have an explicit
//! synchronization strategy). `std::sync::Mutex`/`RwLock` poison on
//! panic-while-held by default and leave every call site to decide
//! `unwrap()` vs. handle it, ad hoc. This module makes that decision
//! once, explicitly, per lock, at construction — not scattered across
//! call sites.

use std::sync::{self, LockResult, PoisonError, TryLockError};

/// What a lock does when it observes poisoning — a prior holder
/// panicked while the lock was held, which may have left the guarded
/// data in a state that violates its invariants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoisonPolicy {
    /// Use the guarded data anyway. Appropriate when every guarded
    /// critical section is small and individually atomic from the
    /// data's own perspective (a panic mid-section can't leave a
    /// half-updated invariant, because there is only one field/step to
    /// update) — `std`'s own poisoning is deliberately conservative
    /// about this, not a proof that recovery is unsafe here.
    Recover,
    /// Panic on poisoning — the same effect as calling `.unwrap()` on
    /// `std`'s `LockResult`, and the correct default whenever a
    /// poisoned lock's data cannot be trusted without re-validation.
    Propagate,
}

fn resolve<G>(result: LockResult<G>, policy: PoisonPolicy) -> G {
    match result {
        Ok(guard) => guard,
        Err(poison) => apply_policy(poison, policy),
    }
}

fn apply_policy<G>(poison: PoisonError<G>, policy: PoisonPolicy) -> G {
    match policy {
        PoisonPolicy::Recover => poison.into_inner(),
        PoisonPolicy::Propagate => {
            panic!("lock poisoned by a prior panic while held (PoisonPolicy::Propagate)")
        }
    }
}

/// A `Mutex` with an explicit, chosen-at-construction [`PoisonPolicy`].
pub struct Mutex<T> {
    inner: sync::Mutex<T>,
    policy: PoisonPolicy,
}

impl<T> Mutex<T> {
    pub fn new(value: T, policy: PoisonPolicy) -> Self {
        Self {
            inner: sync::Mutex::new(value),
            policy,
        }
    }

    pub fn lock(&self) -> sync::MutexGuard<'_, T> {
        resolve(self.inner.lock(), self.policy)
    }

    pub fn try_lock(&self) -> Option<sync::MutexGuard<'_, T>> {
        match self.inner.try_lock() {
            Ok(guard) => Some(guard),
            Err(TryLockError::WouldBlock) => None,
            Err(TryLockError::Poisoned(poison)) => Some(apply_policy(poison, self.policy)),
        }
    }
}

/// An `RwLock` with an explicit, chosen-at-construction [`PoisonPolicy`].
pub struct RwLock<T> {
    inner: sync::RwLock<T>,
    policy: PoisonPolicy,
}

impl<T> RwLock<T> {
    pub fn new(value: T, policy: PoisonPolicy) -> Self {
        Self {
            inner: sync::RwLock::new(value),
            policy,
        }
    }

    pub fn read(&self) -> sync::RwLockReadGuard<'_, T> {
        resolve(self.inner.read(), self.policy)
    }

    pub fn write(&self) -> sync::RwLockWriteGuard<'_, T> {
        resolve(self.inner.write(), self.policy)
    }

    pub fn try_read(&self) -> Option<sync::RwLockReadGuard<'_, T>> {
        match self.inner.try_read() {
            Ok(guard) => Some(guard),
            Err(TryLockError::WouldBlock) => None,
            Err(TryLockError::Poisoned(poison)) => Some(apply_policy(poison, self.policy)),
        }
    }

    pub fn try_write(&self) -> Option<sync::RwLockWriteGuard<'_, T>> {
        match self.inner.try_write() {
            Ok(guard) => Some(guard),
            Err(TryLockError::WouldBlock) => None,
            Err(TryLockError::Poisoned(poison)) => Some(apply_policy(poison, self.policy)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn poison_mutex(mutex: &Mutex<i32>) {
        let _ = std::thread::scope(|s| {
            s.spawn(|| {
                let _guard = mutex.inner.lock().unwrap();
                panic!("deliberately poisoning the lock");
            })
            .join()
        });
    }

    #[test]
    fn recover_policy_returns_data_after_poisoning() {
        let mutex = Mutex::new(1, PoisonPolicy::Recover);
        poison_mutex(&mutex);
        // Must not panic: the whole point of `Recover`.
        let guard = mutex.lock();
        assert_eq!(*guard, 1);
    }

    #[test]
    #[should_panic(expected = "PoisonPolicy::Propagate")]
    fn propagate_policy_panics_after_poisoning() {
        let mutex = Mutex::new(1, PoisonPolicy::Propagate);
        poison_mutex(&mutex);
        drop(mutex.lock());
    }

    #[test]
    fn unpoisoned_lock_works_under_either_policy() {
        let recover = Mutex::new(1, PoisonPolicy::Recover);
        *recover.lock() += 1;
        assert_eq!(*recover.lock(), 2);

        let propagate = Mutex::new(1, PoisonPolicy::Propagate);
        *propagate.lock() += 1;
        assert_eq!(*propagate.lock(), 2);
    }

    #[test]
    fn rwlock_allows_concurrent_reads() {
        let lock = RwLock::new(42, PoisonPolicy::Propagate);
        let a = lock.read();
        let b = lock.read();
        assert_eq!(*a, 42);
        assert_eq!(*b, 42);
    }
}
