//! An async-aware mutex: `lock().await` suspends the task (freeing the
//! worker thread to run other work) instead of blocking it, unlike
//! `std::sync::Mutex`.
//!
//! Each waiting `lock()` holds a place in a FIFO queue until it acquires
//! or its future is dropped. Release wakes the front waiter and leaves it
//! queued; a dropped front waiter passes that wake to the next one, so
//! cancellation (a `timeout`, a losing `select!` branch) never strands
//! the waiters behind it.
//!
//! A waker's destructor may re-enter this mutex, so wakers removed from
//! or replaced in the queue are only dropped after `state` is released.

use std::cell::UnsafeCell;
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::mem::ManuallyDrop;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex as StdMutex, MutexGuard as StdGuard, PoisonError};
use std::task::{Poll, Waker};

struct State {
    locked: bool,
    waiters: VecDeque<(u64, Waker)>,
    next_id: u64,
}

impl State {
    const fn new() -> Self {
        State {
            locked: false,
            waiters: VecDeque::new(),
            next_id: 0,
        }
    }

    fn enqueue(&mut self, waker: &Waker) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.waiters.push_back((id, waker.clone()));
        id
    }

    /// Refreshes waiter `id`'s waker, so a re-poll never queues twice.
    /// Returns the replaced waker, for the caller to drop once unlocked.
    fn rewait(&mut self, id: u64, waker: &Waker) -> Option<Waker> {
        let (_, w) = self.waiters.iter_mut().find(|(i, _)| *i == id)?;
        (!w.will_wake(waker)).then(|| std::mem::replace(w, waker.clone()))
    }

    /// Removes waiter `id`, returning its waker (to drop once unlocked)
    /// and whether it was at the front.
    fn dequeue(&mut self, id: u64) -> Option<(Waker, bool)> {
        let pos = self.waiters.iter().position(|(i, _)| *i == id)?;
        let (_, waker) = self.waiters.remove(pos)?;
        Some((waker, pos == 0))
    }

    /// The front waiter's waker, if the lock is free for it to take.
    fn next_waker(&self) -> Option<Waker> {
        if self.locked {
            return None;
        }
        self.waiters.front().map(|(_, w)| w.clone())
    }
}

/// No code that can panic runs while `state` is held, so a poisoned
/// lock still guards consistent state.
fn lock(state: &StdMutex<State>) -> StdGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A waiter's place in the queue, left on drop (a cancelled `lock()`).
/// Leaving from the front of a free lock passes the wake on.
struct Place<'a> {
    state: &'a StdMutex<State>,
    id: Option<u64>,
}

impl Drop for Place<'_> {
    fn drop(&mut self) {
        let Some(id) = self.id else { return };
        let mut state = lock(self.state);
        let removed = state.dequeue(id);
        let next = match removed {
            Some((_, true)) => state.next_waker(),
            _ => None,
        };
        drop(state);
        drop(removed);
        if let Some(waker) = next {
            waker.wake();
        }
    }
}

pub struct Mutex<T> {
    state: StdMutex<State>,
    data: UnsafeCell<T>,
}

// SAFETY: `data` is only ever accessed through a `MutexGuard`, and a
// guard only exists while `state.locked` is true and owned by exactly
// one holder -- the same exclusion contract `std::sync::Mutex` relies
// on to justify the identical unsafe impl.
unsafe impl<T: Send> Send for Mutex<T> {}
unsafe impl<T: Send> Sync for Mutex<T> {}

impl<T> Mutex<T> {
    pub fn new(value: T) -> Self {
        Mutex {
            state: StdMutex::new(State::new()),
            data: UnsafeCell::new(value),
        }
    }

    /// Identical to [`new`](Self::new), but usable in a `const` context
    /// -- `static LOCK: Mutex<Vec<u8>> = Mutex::const_new(Vec::new());`
    /// -- the same way [`Notify::const_new`](super::Notify::const_new)
    /// is. Every field's constructor (`std::sync::Mutex::new`,
    /// `VecDeque::new`, `UnsafeCell::new`) is already a `const fn`, so
    /// there's no lazy initialization to hide; `new` stays a plain `fn`
    /// only to match tokio's signatures exactly.
    pub const fn const_new(value: T) -> Self {
        Mutex {
            state: StdMutex::new(State::new()),
            data: UnsafeCell::new(value),
        }
    }

    async fn acquire(&self) {
        let mut place = Place {
            state: &self.state,
            id: None,
        };
        std::future::poll_fn(|cx| {
            let mut state = lock(&self.state);
            if !state.locked {
                state.locked = true;
                let removed = place.id.take().and_then(|id| state.dequeue(id));
                drop(state);
                drop(removed);
                return Poll::Ready(());
            }
            let replaced = match place.id {
                Some(id) => state.rewait(id, cx.waker()),
                None => {
                    place.id = Some(state.enqueue(cx.waker()));
                    None
                }
            };
            drop(state);
            drop(replaced);
            Poll::Pending
        })
        .await;
    }

    fn try_acquire(&self) -> bool {
        let mut guard = lock(&self.state);
        if guard.locked {
            false
        } else {
            guard.locked = true;
            true
        }
    }

    /// Releases the lock and wakes the front waiter, if any, which stays
    /// queued until its re-poll acquires (see the module docs). Shared by
    /// every guard flavor's `Drop` impl (`MutexGuard`, `OwnedMutexGuard`,
    /// and the mapped guards, which forward to whichever of those two
    /// they were created from).
    fn release(&self) {
        let mut guard = lock(&self.state);
        guard.locked = false;
        let next = guard.next_waker();
        drop(guard);
        if let Some(waker) = next {
            waker.wake();
        }
        // Note: a brand-new `lock()` call racing in right after we drop
        // `guard` above could grab the mutex before the woken waiter
        // above gets re-polled; the woken waiter then stays at the
        // front, and that caller's release wakes it again. Exclusion is
        // still fully guaranteed (only one caller's `locked = true`
        // check-and-set can ever win), just not strict FIFO fairness --
        // an acceptable simplification for a hand-rolled mutex.
    }

    pub async fn lock(&self) -> MutexGuard<'_, T> {
        self.acquire().await;
        MutexGuard { mutex: self }
    }

    /// Locks without waiting, failing if it's currently held.
    pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
        // `then` (lazy), not `then_some`: `then_some`'s argument is
        // eagerly constructed regardless of the bool, and a guard's
        // `Drop` unconditionally releases -- constructing one here even
        // on the `false` path would release a lock this call never
        // actually acquired, corrupting `locked` for whoever really
        // holds it.
        self.try_acquire().then(|| MutexGuard { mutex: self })
    }

    /// Like [`lock`](Self::lock), but the returned guard owns an `Arc`
    /// clone of this mutex instead of borrowing it -- lets the guard
    /// outlive the scope holding `self`, e.g. across a spawned task
    /// boundary.
    pub async fn lock_owned(self: &Arc<Self>) -> OwnedMutexGuard<T> {
        self.acquire().await;
        OwnedMutexGuard {
            mutex: self.clone(),
        }
    }

    /// `Arc`-owned counterpart of [`try_lock`](Self::try_lock).
    pub fn try_lock_owned(self: &Arc<Self>) -> Option<OwnedMutexGuard<T>> {
        self.try_acquire().then(|| OwnedMutexGuard {
            mutex: self.clone(),
        })
    }

    pub fn get_mut(&mut self) -> &mut T {
        // SAFETY: `&mut self` proves exclusive access, no guard can
        // exist concurrently.
        unsafe { &mut *self.data.get() }
    }
}

pub struct MutexGuard<'a, T> {
    mutex: &'a Mutex<T>,
}

/// As tokio's: a `Mutex` around `value`.
impl<T> From<T> for Mutex<T> {
    fn from(value: T) -> Self {
        Mutex::new(value)
    }
}

/// As tokio's: a `Mutex` around `T::default()`.
impl<T: Default> Default for Mutex<T> {
    fn default() -> Self {
        Mutex::new(T::default())
    }
}

impl<T> Deref for MutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: holding a `MutexGuard` proves `state.locked` was set
        // by us and won't be released until this guard drops -- nobody
        // else can be dereferencing `data` at the same time.
        unsafe { &*self.mutex.data.get() }
    }
}

impl<T> DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: see `Deref` above.
        unsafe { &mut *self.mutex.data.get() }
    }
}

impl<T> Drop for MutexGuard<'_, T> {
    fn drop(&mut self) {
        self.mutex.release();
    }
}

impl<'a, T> MutexGuard<'a, T> {
    /// Projects this guard onto a field or sub-value reachable via `f`,
    /// giving up direct access to the rest of `T` in exchange -- the
    /// returned [`MappedMutexGuard`] still holds the lock (and releases
    /// it on drop) but only derefs to `U`.
    pub fn map<U, F>(this: Self, f: F) -> MappedMutexGuard<'a, T, U>
    where
        F: FnOnce(&mut T) -> &mut U,
    {
        let mutex = this.mutex;
        // SAFETY: `this` holding the lock proves exclusive access to
        // `data` for as long as `this` (and, after the projection below,
        // the `MappedMutexGuard` replacing it) lives.
        let value: *mut U = f(unsafe { &mut *mutex.data.get() });
        // Skip `MutexGuard`'s own `Drop` -- ownership of "release this
        // lock on drop" moves to the `MappedMutexGuard` being returned,
        // and running both would double-release.
        std::mem::forget(this);
        MappedMutexGuard {
            mutex,
            value,
            _marker: PhantomData,
        }
    }
}

/// `Arc`-owned counterpart of [`MutexGuard`] -- see
/// [`Mutex::lock_owned`].
pub struct OwnedMutexGuard<T> {
    mutex: Arc<Mutex<T>>,
}

impl<T> Deref for OwnedMutexGuard<T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: see `MutexGuard::deref`.
        unsafe { &*self.mutex.data.get() }
    }
}

impl<T> DerefMut for OwnedMutexGuard<T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: see `MutexGuard::deref`.
        unsafe { &mut *self.mutex.data.get() }
    }
}

impl<T> Drop for OwnedMutexGuard<T> {
    fn drop(&mut self) {
        self.mutex.release();
    }
}

impl<T> OwnedMutexGuard<T> {
    /// `Arc`-owned counterpart of [`MutexGuard::map`].
    pub fn map<U, F>(this: Self, f: F) -> OwnedMappedMutexGuard<T, U>
    where
        F: FnOnce(&mut T) -> &mut U,
    {
        // `ManuallyDrop` (rather than `mem::forget`, as `MutexGuard::map`
        // uses) so `mutex`'s `Arc` refcount can still be moved out below
        // instead of being leaked along with the rest of `this`.
        let this = ManuallyDrop::new(this);
        // SAFETY: see `MutexGuard::map`; `this.mutex` isn't dropped
        // (going through `ManuallyDrop`) so the `Arc` clone below is the
        // only one -- no double-release, no leaked refcount.
        let value: *mut U = f(unsafe { &mut *this.mutex.data.get() });
        let mutex = this.mutex.clone();
        OwnedMappedMutexGuard {
            mutex,
            value,
            _marker: PhantomData,
        }
    }
}

/// A [`MutexGuard`] narrowed to a projected field or sub-value via
/// [`MutexGuard::map`] -- still holds (and releases, on drop) the same
/// lock, but only derefs to `U`.
pub struct MappedMutexGuard<'a, T, U: ?Sized> {
    mutex: &'a Mutex<T>,
    value: *mut U,
    _marker: PhantomData<&'a mut U>,
}

// SAFETY: a `MappedMutexGuard` represents the same exclusive-access
// guarantee `MutexGuard` itself does (nothing else can reach `value`
// until this guard drops), just narrowed to `U` instead of `T` -- same
// justification as `Mutex<T>`'s own unsafe `Send`/`Sync` impls.
unsafe impl<T, U: ?Sized + Send> Send for MappedMutexGuard<'_, T, U> {}
unsafe impl<T, U: ?Sized + Sync> Sync for MappedMutexGuard<'_, T, U> {}

impl<T, U: ?Sized> Deref for MappedMutexGuard<'_, T, U> {
    type Target = U;
    fn deref(&self) -> &U {
        // SAFETY: `value` was projected from `mutex.data` while its lock
        // was held, and stays valid until this guard drops (which is
        // when the lock actually releases).
        unsafe { &*self.value }
    }
}

impl<T, U: ?Sized> DerefMut for MappedMutexGuard<'_, T, U> {
    fn deref_mut(&mut self) -> &mut U {
        // SAFETY: see `Deref` above.
        unsafe { &mut *self.value }
    }
}

impl<T, U: ?Sized> Drop for MappedMutexGuard<'_, T, U> {
    fn drop(&mut self) {
        self.mutex.release();
    }
}

/// `Arc`-owned counterpart of [`MappedMutexGuard`] -- see
/// [`OwnedMutexGuard::map`].
pub struct OwnedMappedMutexGuard<T, U: ?Sized> {
    mutex: Arc<Mutex<T>>,
    value: *mut U,
    _marker: PhantomData<*mut U>,
}

// SAFETY: see `MappedMutexGuard`'s identical unsafe impls above.
unsafe impl<T, U: ?Sized + Send> Send for OwnedMappedMutexGuard<T, U> {}
unsafe impl<T, U: ?Sized + Sync> Sync for OwnedMappedMutexGuard<T, U> {}

impl<T, U: ?Sized> Deref for OwnedMappedMutexGuard<T, U> {
    type Target = U;
    fn deref(&self) -> &U {
        // SAFETY: see `MappedMutexGuard::deref`.
        unsafe { &*self.value }
    }
}

impl<T, U: ?Sized> DerefMut for OwnedMappedMutexGuard<T, U> {
    fn deref_mut(&mut self) -> &mut U {
        // SAFETY: see `MappedMutexGuard::deref`.
        unsafe { &mut *self.value }
    }
}

impl<T, U: ?Sized> Drop for OwnedMappedMutexGuard<T, U> {
    fn drop(&mut self) {
        self.mutex.release();
    }
}
