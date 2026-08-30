//! Poison-tolerant `Mutex` locking for this crate's in-memory state.
//!
//! A lock here only poisons when some other thread already panicked
//! while holding it — almost always an unrelated failure elsewhere in
//! the same test process, not corruption of the guarded value itself
//! (this crate's mocked state has no real OS resource behind it to be
//! left inconsistent). Refusing to proceed would just turn that
//! original panic into a second, more confusing one here, so recover
//! the guard and continue instead of `.expect()`-ing on it.

pub(crate) fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
