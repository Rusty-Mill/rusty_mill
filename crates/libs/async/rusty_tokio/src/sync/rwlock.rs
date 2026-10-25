//! An async-aware reader/writer lock: many concurrent `read().await`
//! guards, or one exclusive `write().await` guard, never blocking the
//! worker thread the way `std::sync::RwLock` would.
//!
//! Write-preferring, matching tokio's own `RwLock`: once a writer is
//! waiting, later readers queue behind it too rather than jumping ahead
//! just because the write lock itself isn't held yet -- otherwise
//! constant read traffic could starve a waiting writer indefinitely.
//! A new reader checks "is a writer queued at all" (not just "is one
//! currently holding"), which is what makes that guarantee hold; a reader
//! already queued checks only for a writer queued ahead of it.
//!
//! Each waiting `read()`/`write()` holds a place in one FIFO queue until
//! it acquires or its future is dropped; a dropped waiter leaves the
//! queue and wakes whoever that makes eligible, so cancellation never
//! strands the waiters behind it. A queued reader is held back only by a
//! writer *ahead of it* in the queue, so one woken behind a released
//! writer is not sent back behind a writer that queued after it.
//!
//! Release (`RwLockReadGuard`/`RwLockWriteGuard`'s `Drop`) only ever
//! *clears* this lock's own contribution and wakes whichever waiters
//! are now eligible -- it never mutates `readers`/`writer_locked` on
//! their behalf. Each woken waiter's own next poll re-checks and
//! self-grants independently, the same principle [`super::Mutex`]'s
//! `Drop` already relies on (see its own doc comment) for why: pre-
//! granting state in the release path, ahead of the waiter's own poll
//! actually running, risks two different tasks both believing they
//! hold the lock if their re-polls happen to interleave unexpectedly.
//! The trade-off is the same one `Mutex` accepts too -- a brand-new
//! `read()`/`write()` call racing in right after a release could
//! acquire before an already-queued waiter gets re-polled; correctness
//! (never two writers, or a writer and a reader, active together) is
//! still fully guaranteed, just not strict FIFO fairness against a
//! fresh (non-queued) caller.

use std::cell::UnsafeCell;
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::mem::ManuallyDrop;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex as StdMutex};
use std::task::{Poll, Waker};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Read,
    Write,
}

struct Waiter {
    id: u64,
    mode: Mode,
    waker: Waker,
}

struct State {
    writer_locked: bool,
    readers: usize,
    waiters: VecDeque<Waiter>,
    next_id: u64,
}

impl State {
    fn writer_pending(&self) -> bool {
        self.waiters.iter().any(|w| w.mode == Mode::Write)
    }

    /// Whether a writer is queued ahead of waiter `id`.
    fn writer_ahead_of(&self, id: u64) -> bool {
        self.waiters
            .iter()
            .take_while(|w| w.id != id)
            .any(|w| w.mode == Mode::Write)
    }

    fn is_front(&self, id: u64) -> bool {
        self.waiters.front().is_some_and(|w| w.id == id)
    }

    /// Queues a new waiter, returning its id.
    fn enqueue(&mut self, mode: Mode, waker: &Waker) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.waiters.push_back(Waiter {
            id,
            mode,
            waker: waker.clone(),
        });
        id
    }

    /// Refreshes waiter `id`'s waker for its next wake.
    fn rewait(&mut self, id: u64, waker: &Waker) {
        if let Some(w) = self.waiters.iter_mut().find(|w| w.id == id) {
            if !w.waker.will_wake(waker) {
                w.waker = waker.clone();
            }
        }
    }

    fn dequeue(&mut self, id: u64) -> bool {
        match self.waiters.iter().position(|w| w.id == id) {
            Some(pos) => {
                self.waiters.remove(pos);
                true
            }
            None => false,
        }
    }

    /// Returns the wakers of every waiter now eligible to run,
    /// given the current (already-updated) state -- called from both
    /// guards' `Drop` after clearing their own contribution, at which
    /// point `writer_locked` is always already `false` (either just
    /// cleared by the write guard calling this, or -- for the read
    /// guard's case -- it could only still be `true` if a writer were
    /// somehow active *and* readers had just reached zero, which can't
    /// happen: a writer never holds while any reader is active). Never
    /// mutates `readers`/`writer_locked` itself; see this module's docs
    /// for why that matters.
    ///
    /// Waiters stay queued when woken: each leaves the queue itself once
    /// its re-poll acquires (or its future is dropped).
    fn wake_eligible(&mut self) -> Vec<Waker> {
        if self.writer_locked {
            return Vec::new();
        }
        let mut woken: Vec<Waker> = self
            .waiters
            .iter()
            .take_while(|w| w.mode == Mode::Read)
            .map(|w| w.waker.clone())
            .collect();
        // Only ever check for a queued writer in the *same* release
        // event when no readers were just woken above -- waking a
        // writer alongside a just-woken reader batch would mean
        // deciding "readers == 0" before any of those readers have
        // actually re-polled and incremented it themselves, which could
        // let the writer and that reader batch both believe they hold
        // the lock if their re-polls happen to interleave unexpectedly.
        // The writer gets its turn one release later instead, once the
        // last of that reader batch's own guards actually drops.
        if woken.is_empty() && self.readers == 0 {
            if let Some(w) = self.waiters.front().filter(|w| w.mode == Mode::Write) {
                woken.push(w.waker.clone());
            }
        }
        woken
    }
}

/// A waiter's place in the queue, left on drop (a cancelled `read()` or
/// `write()`), waking whoever that makes eligible.
struct Place<'a> {
    state: &'a StdMutex<State>,
    id: Option<u64>,
}

impl Drop for Place<'_> {
    fn drop(&mut self) {
        let Some(id) = self.id else { return };
        let mut state = self.state.lock().unwrap();
        let woken = if state.dequeue(id) {
            state.wake_eligible()
        } else {
            Vec::new()
        };
        drop(state);
        for waker in woken {
            waker.wake();
        }
    }
}

pub struct RwLock<T> {
    state: StdMutex<State>,
    data: UnsafeCell<T>,
}

// SAFETY: `data` is only ever accessed through a guard, and a write
// guard only exists while `writer_locked` is exclusively held -- the
// same exclusion contract `std::sync::RwLock` relies on for its
// identical bounds (`Send` needs only `T: Send`; `Sync` -- letting
// `&RwLock<T>` itself cross threads, which is what lets multiple
// readers share `&T` concurrently -- additionally needs `T: Sync`).
unsafe impl<T: Send> Send for RwLock<T> {}
unsafe impl<T: Send + Sync> Sync for RwLock<T> {}

impl<T> RwLock<T> {
    pub fn new(value: T) -> Self {
        RwLock {
            state: StdMutex::new(State {
                writer_locked: false,
                readers: 0,
                waiters: VecDeque::new(),
                next_id: 0,
            }),
            data: UnsafeCell::new(value),
        }
    }

    /// A new reader waits for any queued writer; a queued one only for a
    /// writer ahead of it.
    async fn acquire_read(&self) {
        let mut place = Place {
            state: &self.state,
            id: None,
        };
        std::future::poll_fn(|cx| {
            let mut guard = self.state.lock().unwrap();
            let blocked = match place.id {
                None => guard.writer_pending(),
                Some(id) => guard.writer_ahead_of(id),
            };
            if !guard.writer_locked && !blocked {
                guard.readers += 1;
                if let Some(id) = place.id.take() {
                    guard.dequeue(id);
                }
                return Poll::Ready(());
            }
            match place.id {
                Some(id) => guard.rewait(id, cx.waker()),
                None => place.id = Some(guard.enqueue(Mode::Read, cx.waker())),
            }
            Poll::Pending
        })
        .await;
    }

    /// A writer acquires a free lock when no one is queued, or when it is
    /// at the front of the queue.
    async fn acquire_write(&self) {
        let mut place = Place {
            state: &self.state,
            id: None,
        };
        std::future::poll_fn(|cx| {
            let mut guard = self.state.lock().unwrap();
            let turn = match place.id {
                None => guard.waiters.is_empty(),
                Some(id) => guard.is_front(id),
            };
            if !guard.writer_locked && guard.readers == 0 && turn {
                guard.writer_locked = true;
                if let Some(id) = place.id.take() {
                    guard.dequeue(id);
                }
                return Poll::Ready(());
            }
            match place.id {
                Some(id) => guard.rewait(id, cx.waker()),
                None => place.id = Some(guard.enqueue(Mode::Write, cx.waker())),
            }
            Poll::Pending
        })
        .await;
    }

    fn try_acquire_read(&self) -> bool {
        let mut guard = self.state.lock().unwrap();
        if !guard.writer_locked && !guard.writer_pending() {
            guard.readers += 1;
            true
        } else {
            false
        }
    }

    fn try_acquire_write(&self) -> bool {
        let mut guard = self.state.lock().unwrap();
        if !guard.writer_locked && guard.readers == 0 {
            guard.writer_locked = true;
            true
        } else {
            false
        }
    }

    /// Releases one reader's share of the lock, waking whichever waiters
    /// are now eligible once the last reader clears. Shared by
    /// `RwLockReadGuard`'s and `OwnedRwLockReadGuard`'s `Drop` impls.
    fn release_read(&self) {
        let mut guard = self.state.lock().unwrap();
        guard.readers -= 1;
        let woken = if guard.readers == 0 {
            guard.wake_eligible()
        } else {
            Vec::new()
        };
        drop(guard);
        for waker in woken {
            waker.wake();
        }
    }

    /// Releases the write lock, waking whichever waiters are now
    /// eligible. Shared by every write-side guard flavor's `Drop` impl
    /// (`RwLockWriteGuard`, `OwnedRwLockWriteGuard`, and the mapped write
    /// guards).
    fn release_write(&self) {
        let mut guard = self.state.lock().unwrap();
        guard.writer_locked = false;
        let woken = guard.wake_eligible();
        drop(guard);
        for waker in woken {
            waker.wake();
        }
    }

    pub async fn read(&self) -> RwLockReadGuard<'_, T> {
        self.acquire_read().await;
        RwLockReadGuard { lock: self }
    }

    pub async fn write(&self) -> RwLockWriteGuard<'_, T> {
        self.acquire_write().await;
        RwLockWriteGuard { lock: self }
    }

    /// Acquires a read lock without waiting, failing if a writer
    /// currently holds it or is queued (see this module's docs for why
    /// a *queued* writer also fails this, not just a holding one).
    pub fn try_read(&self) -> Option<RwLockReadGuard<'_, T>> {
        // `then` (lazy), not `then_some`: `then_some`'s argument is
        // eagerly constructed regardless of the bool, and a guard's
        // `Drop` unconditionally releases -- constructing one here even
        // on the `false` path would release a lock this call never
        // actually acquired.
        self.try_acquire_read()
            .then(|| RwLockReadGuard { lock: self })
    }

    /// Acquires the write lock without waiting, failing if it's
    /// currently held or any readers are active.
    pub fn try_write(&self) -> Option<RwLockWriteGuard<'_, T>> {
        self.try_acquire_write()
            .then(|| RwLockWriteGuard { lock: self })
    }

    /// `Arc`-owned counterpart of [`read`](Self::read) -- the returned
    /// guard holds an `Arc` clone instead of borrowing, so it can
    /// outlive the scope holding `self`, e.g. across a spawned task
    /// boundary.
    pub async fn read_owned(self: &Arc<Self>) -> OwnedRwLockReadGuard<T> {
        self.acquire_read().await;
        OwnedRwLockReadGuard { lock: self.clone() }
    }

    /// `Arc`-owned counterpart of [`write`](Self::write).
    pub async fn write_owned(self: &Arc<Self>) -> OwnedRwLockWriteGuard<T> {
        self.acquire_write().await;
        OwnedRwLockWriteGuard { lock: self.clone() }
    }

    /// `Arc`-owned counterpart of [`try_read`](Self::try_read).
    pub fn try_read_owned(self: &Arc<Self>) -> Option<OwnedRwLockReadGuard<T>> {
        self.try_acquire_read()
            .then(|| OwnedRwLockReadGuard { lock: self.clone() })
    }

    /// `Arc`-owned counterpart of [`try_write`](Self::try_write).
    pub fn try_write_owned(self: &Arc<Self>) -> Option<OwnedRwLockWriteGuard<T>> {
        self.try_acquire_write()
            .then(|| OwnedRwLockWriteGuard { lock: self.clone() })
    }

    pub fn get_mut(&mut self) -> &mut T {
        // SAFETY: `&mut self` proves exclusive access, no guard can
        // exist concurrently.
        unsafe { &mut *self.data.get() }
    }
}

pub struct RwLockReadGuard<'a, T> {
    lock: &'a RwLock<T>,
}

/// As tokio's: a `RwLock` around `value`.
impl<T> From<T> for RwLock<T> {
    fn from(value: T) -> Self {
        RwLock::new(value)
    }
}

/// As tokio's: the value when a read lock is free right now, else `<locked>`.
impl<T: std::fmt::Debug> std::fmt::Debug for RwLock<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut d = f.debug_struct("RwLock");
        match self.try_read() {
            Some(inner) => d.field("data", &&*inner),
            None => d.field("data", &format_args!("<locked>")),
        };
        d.finish()
    }
}

/// As tokio's: a `RwLock` around `T::default()`.
impl<T: Default> Default for RwLock<T> {
    fn default() -> Self {
        RwLock::new(T::default())
    }
}

impl<T> Deref for RwLockReadGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: holding a read guard proves `readers` was incremented
        // by us and won't be invalidated by a concurrent writer until
        // every read guard (including this one) drops.
        unsafe { &*self.lock.data.get() }
    }
}

impl<T> Drop for RwLockReadGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.release_read();
    }
}

/// `Arc`-owned counterpart of [`RwLockReadGuard`] -- see
/// [`RwLock::read_owned`].
pub struct OwnedRwLockReadGuard<T> {
    lock: Arc<RwLock<T>>,
}

impl<T> Deref for OwnedRwLockReadGuard<T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: see `RwLockReadGuard::deref`.
        unsafe { &*self.lock.data.get() }
    }
}

impl<T> Drop for OwnedRwLockReadGuard<T> {
    fn drop(&mut self) {
        self.lock.release_read();
    }
}

pub struct RwLockWriteGuard<'a, T> {
    lock: &'a RwLock<T>,
}

impl<'a, T> RwLockWriteGuard<'a, T> {
    /// Converts this write guard into a read guard on the same lock,
    /// without ever passing through a moment where nothing holds it --
    /// a fresh `write().await` racing in between a plain `drop` and a
    /// new `read().await` could otherwise grab the lock first. Any
    /// queued readers behind this writer become eligible immediately
    /// (they were only ever blocked by `writer_locked`, not by this
    /// guard's own presence as a reader); a queued writer is not, since
    /// `readers` becomes `1` -- not `0` -- as part of the same update.
    pub fn downgrade(self) -> RwLockReadGuard<'a, T> {
        let lock = self.lock;
        // Skip this guard's own `Drop` -- it would clear `writer_locked`
        // and decide waiter eligibility as if no reader were taking over,
        // which is exactly the momentary "nothing holds the lock" gap
        // this method exists to avoid.
        std::mem::forget(self);
        let mut guard = lock.state.lock().unwrap();
        guard.writer_locked = false;
        guard.readers = 1;
        let woken = guard.wake_eligible();
        drop(guard);
        for waker in woken {
            waker.wake();
        }
        RwLockReadGuard { lock }
    }

    /// Projects this guard onto a field or sub-value reachable via `f`,
    /// giving up direct access to the rest of `T` in exchange -- the
    /// returned [`RwLockMappedWriteGuard`] still holds the write lock
    /// (and releases it on drop) but only derefs to `U`.
    pub fn map<U, F>(this: Self, f: F) -> RwLockMappedWriteGuard<'a, T, U>
    where
        F: FnOnce(&mut T) -> &mut U,
    {
        let lock = this.lock;
        // SAFETY: `this` holding the write lock proves exclusive access
        // to `data` for as long as `this` (and, after the projection
        // below, the `RwLockMappedWriteGuard` replacing it) lives.
        let value: *mut U = f(unsafe { &mut *lock.data.get() });
        std::mem::forget(this);
        RwLockMappedWriteGuard {
            lock,
            value,
            _marker: PhantomData,
        }
    }
}

impl<T> Deref for RwLockWriteGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: see `DerefMut` below.
        unsafe { &*self.lock.data.get() }
    }
}

impl<T> DerefMut for RwLockWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: holding a write guard proves `writer_locked` was set
        // by us and won't be released until this guard drops -- nobody
        // else (reader or writer) can be accessing `data` at the same
        // time.
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T> Drop for RwLockWriteGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.release_write();
    }
}

/// `Arc`-owned counterpart of [`RwLockWriteGuard`] -- see
/// [`RwLock::write_owned`].
pub struct OwnedRwLockWriteGuard<T> {
    lock: Arc<RwLock<T>>,
}

impl<T> OwnedRwLockWriteGuard<T> {
    /// `Arc`-owned counterpart of [`RwLockWriteGuard::downgrade`].
    pub fn downgrade(self) -> OwnedRwLockReadGuard<T> {
        let this = ManuallyDrop::new(self);
        // SAFETY: `this.lock` isn't dropped (going through
        // `ManuallyDrop`), so the clone below is the only `Arc` handle
        // produced here -- no double-release, no leaked refcount.
        let lock = unsafe { std::ptr::read(&this.lock) };
        let mut guard = lock.state.lock().unwrap();
        guard.writer_locked = false;
        guard.readers = 1;
        let woken = guard.wake_eligible();
        drop(guard);
        for waker in woken {
            waker.wake();
        }
        OwnedRwLockReadGuard { lock }
    }

    /// `Arc`-owned counterpart of [`RwLockWriteGuard::map`].
    pub fn map<U, F>(this: Self, f: F) -> OwnedRwLockMappedWriteGuard<T, U>
    where
        F: FnOnce(&mut T) -> &mut U,
    {
        let this = ManuallyDrop::new(this);
        // SAFETY: see `RwLockWriteGuard::map`; `this.lock` isn't dropped
        // (going through `ManuallyDrop`) so the clone below is the only
        // `Arc` handle produced here.
        let value: *mut U = f(unsafe { &mut *this.lock.data.get() });
        let lock = this.lock.clone();
        OwnedRwLockMappedWriteGuard {
            lock,
            value,
            _marker: PhantomData,
        }
    }
}

impl<T> Deref for OwnedRwLockWriteGuard<T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: see `RwLockWriteGuard::deref`.
        unsafe { &*self.lock.data.get() }
    }
}

impl<T> DerefMut for OwnedRwLockWriteGuard<T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: see `RwLockWriteGuard::deref_mut`.
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T> Drop for OwnedRwLockWriteGuard<T> {
    fn drop(&mut self) {
        self.lock.release_write();
    }
}

/// A [`RwLockWriteGuard`] narrowed to a projected field or sub-value via
/// [`RwLockWriteGuard::map`] -- still holds (and releases, on drop) the
/// write lock, but only derefs to `U`.
pub struct RwLockMappedWriteGuard<'a, T, U: ?Sized> {
    lock: &'a RwLock<T>,
    value: *mut U,
    _marker: PhantomData<&'a mut U>,
}

// SAFETY: a `RwLockMappedWriteGuard` represents the same exclusive-access
// guarantee `RwLockWriteGuard` itself does, just narrowed to `U` instead
// of `T` -- same justification as `RwLock<T>`'s own unsafe `Send`/`Sync`
// impls.
unsafe impl<T, U: ?Sized + Send> Send for RwLockMappedWriteGuard<'_, T, U> {}
unsafe impl<T, U: ?Sized + Sync> Sync for RwLockMappedWriteGuard<'_, T, U> {}

impl<T, U: ?Sized> Deref for RwLockMappedWriteGuard<'_, T, U> {
    type Target = U;
    fn deref(&self) -> &U {
        // SAFETY: `value` was projected from `lock.data` while the write
        // lock was held, and stays valid until this guard drops (which
        // is when the write lock actually releases).
        unsafe { &*self.value }
    }
}

impl<T, U: ?Sized> DerefMut for RwLockMappedWriteGuard<'_, T, U> {
    fn deref_mut(&mut self) -> &mut U {
        // SAFETY: see `Deref` above.
        unsafe { &mut *self.value }
    }
}

impl<T, U: ?Sized> Drop for RwLockMappedWriteGuard<'_, T, U> {
    fn drop(&mut self) {
        self.lock.release_write();
    }
}

/// `Arc`-owned counterpart of [`RwLockMappedWriteGuard`] -- see
/// [`OwnedRwLockWriteGuard::map`].
pub struct OwnedRwLockMappedWriteGuard<T, U: ?Sized> {
    lock: Arc<RwLock<T>>,
    value: *mut U,
    _marker: PhantomData<*mut U>,
}

// SAFETY: see `RwLockMappedWriteGuard`'s identical unsafe impls above.
unsafe impl<T, U: ?Sized + Send> Send for OwnedRwLockMappedWriteGuard<T, U> {}
unsafe impl<T, U: ?Sized + Sync> Sync for OwnedRwLockMappedWriteGuard<T, U> {}

impl<T, U: ?Sized> Deref for OwnedRwLockMappedWriteGuard<T, U> {
    type Target = U;
    fn deref(&self) -> &U {
        // SAFETY: see `RwLockMappedWriteGuard::deref`.
        unsafe { &*self.value }
    }
}

impl<T, U: ?Sized> DerefMut for OwnedRwLockMappedWriteGuard<T, U> {
    fn deref_mut(&mut self) -> &mut U {
        // SAFETY: see `RwLockMappedWriteGuard::deref`.
        unsafe { &mut *self.value }
    }
}

impl<T, U: ?Sized> Drop for OwnedRwLockMappedWriteGuard<T, U> {
    fn drop(&mut self) {
        self.lock.release_write();
    }
}
