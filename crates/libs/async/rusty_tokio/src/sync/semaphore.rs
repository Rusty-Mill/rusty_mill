//! A counting semaphore: `acquire().await` suspends the task (instead
//! of blocking the worker thread) until a permit is available, capping
//! how many callers can hold one at a time -- e.g. "at most 10
//! concurrent outbound requests."
//!
//! Fair (FIFO) like tokio's own `Semaphore`: an `acquire`/`acquire_many`
//! call only takes the fast (immediate) path when *no one* is already
//! queued, so a caller that arrives while others are waiting always
//! queues behind them rather than possibly jumping ahead just because
//! enough permits happen to be free at that instant.
//!
//! Unlike [`super::Mutex`]/[`super::RwLock`]'s release logic (see their
//! own doc comments for why they specifically *avoid* this), releasing
//! permits back here genuinely does decide, and commit to, exactly
//! which queued waiters get how many permits -- directly in the release
//! path, before waking any of them. That's safe here in a way it isn't
//! for a binary locked/unlocked flag: each waiter gets its own
//! independent `granted` flag, set at most once, by whichever release
//! event decides to grant it; nothing else can ever un-decide or
//! re-decide that later. A waiter's own poll only ever checks its own
//! flag, never re-derives eligibility from the shared permit count the
//! way a `Mutex` waiter re-checks the shared `locked` bit -- so there's
//! no path by which two different decisions could end up made for the
//! same waiter.
//!
//! A dropped `acquire` future (a `timeout`, a losing `select!` branch)
//! settles its claim: still queued, it leaves the queue and lets the
//! waiters behind it be granted; already granted, it hands the permits
//! back. Neither strands the queue nor leaks permits.
//!
//! A waker's clone or drop may run executor code that re-enters the
//! semaphore, so neither ever happens with `state` held: a waiter's
//! clone is made before it locks, a grant moves the queued waker out,
//! and wakers removed from or replaced in the queue are dropped after
//! unlocking.
//!
//! Every path that returns permits ([`Semaphore::add_permits`], a
//! dropped permit, a dropped `acquire` future that was already granted)
//! goes through [`State::release`], which bounds the count left after
//! granting at [`Semaphore::MAX_PERMITS`] as tokio does. The bound is
//! checked under `state`, in the same critical section as the update,
//! so two releases cannot both pass it; a release that would exceed it
//! changes nothing, and the caller panics only once `state` is unlocked.

use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex as StdMutex, MutexGuard as StdGuard, PoisonError};
use std::task::{Poll, Waker};

/// A queued waiter's outcome, decided at most once by whichever event
/// (a release granting it, or [`Semaphore::close`]) gets there first --
/// see this module's docs on why that single decision is safe to trust
/// without the waiter ever re-deriving it itself.
const WAITER_PENDING: u8 = 0;
const WAITER_GRANTED: u8 = 1;
const WAITER_CLOSED: u8 = 2;

struct Waiter {
    needed: usize,
    outcome: Arc<AtomicU8>,
    waker: Waker,
}

struct State {
    permits: usize,
    closed: bool,
    waiters: VecDeque<Waiter>,
}

impl State {
    /// Grants queued waiters, in FIFO order, as many as the permit count
    /// allows -- stopping at the first one that doesn't fit, since
    /// granting out of order would break the fairness this module's
    /// docs describe. Returns the wakers to call once unlocked.
    fn grant(&mut self) -> Vec<Waker> {
        let mut woken = Vec::new();
        while self
            .waiters
            .front()
            .is_some_and(|w| w.needed <= self.permits)
        {
            let Some(waiter) = self.waiters.pop_front() else {
                break;
            };
            self.permits -= waiter.needed;
            waiter.outcome.store(WAITER_GRANTED, Ordering::Release);
            woken.push(waiter.waker);
        }
        woken
    }

    /// Adds `added` permits and grants the queued waiters they let
    /// proceed, as `permits += added` followed by [`grant`] would without
    /// overflow, and returns their wakers. If the count left after granting would
    /// exceed [`Semaphore::MAX_PERMITS`], changes nothing and returns the
    /// part of `added` the waiters did not take, for [`overflow`]: as in
    /// tokio's `add_permits_locked` (batch_semaphore.rs), permits handed
    /// to waiters count against nothing. Plans first and commits after,
    /// so no arithmetic here can overflow and nothing panics with `state`
    /// held.
    ///
    /// [`grant`]: Self::grant
    fn release(&mut self, added: usize) -> Result<Vec<Waker>, usize> {
        // `free + extra` may not fit in a usize, so they stay apart until
        // the waiters have taken their share.
        let mut free = self.permits;
        let mut extra = added;
        let mut granted = 0;
        for waiter in &self.waiters {
            if waiter.needed <= free {
                free -= waiter.needed;
            } else if waiter.needed - free <= extra {
                extra -= waiter.needed - free;
                free = 0;
            } else {
                break;
            }
            granted += 1;
        }
        let left = free
            .checked_add(extra)
            .filter(|&left| left <= Semaphore::MAX_PERMITS)
            .ok_or(extra)?;
        self.permits = left;
        let woken = self
            .waiters
            .drain(..granted)
            .map(|waiter| {
                waiter.outcome.store(WAITER_GRANTED, Ordering::Release);
                waiter.waker
            })
            .collect();
        Ok(woken)
    }

    /// Whether the waiter owning `outcome` already wakes `waker`'s task.
    fn wakes(&self, outcome: &Arc<AtomicU8>, waker: &Waker) -> bool {
        self.waiters
            .iter()
            .any(|w| Arc::ptr_eq(&w.outcome, outcome) && w.waker.will_wake(waker))
    }

    /// Stores `waker` for the waiter owning `outcome`. Returns the waker
    /// it displaces (or `waker`, if that waiter is gone), for the caller
    /// to drop once unlocked.
    fn rewait(&mut self, outcome: &Arc<AtomicU8>, waker: Waker) -> Waker {
        match self
            .waiters
            .iter_mut()
            .find(|w| Arc::ptr_eq(&w.outcome, outcome))
        {
            Some(w) => std::mem::replace(&mut w.waker, waker),
            None => waker,
        }
    }

    /// Removes the waiter owning `outcome`, for the caller to drop once
    /// unlocked.
    fn dequeue(&mut self, outcome: &Arc<AtomicU8>) -> Option<Waiter> {
        let pos = self
            .waiters
            .iter()
            .position(|w| Arc::ptr_eq(&w.outcome, outcome))?;
        self.waiters.remove(pos)
    }
}

/// No code that can panic runs while `state` is held, so a poisoned
/// lock still guards consistent state.
fn lock(state: &StdMutex<State>) -> StdGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Fails a release that [`State::release`] rejected, given the permits
/// left over once waiters were granted. Called with `state` unlocked;
/// the messages are tokio's `add_permits_locked`. `#[track_caller]` here,
/// on `Semaphore::release` and on `add_permits` puts the panic at the
/// caller of `add_permits`; a permit's drop reports its `Drop` impl.
#[cold]
#[track_caller]
fn overflow(rest: usize) -> ! {
    let max = Semaphore::MAX_PERMITS;
    if rest > max {
        panic!("cannot add more than MAX_PERMITS permits ({max})");
    }
    panic!("number of added permits ({rest}) would overflow MAX_PERMITS ({max})")
}

fn wake_all(wakers: Vec<Waker>) {
    for waker in wakers {
        waker.wake();
    }
}

/// A queued `acquire`'s claim, settled on drop if its future never
/// observed the outcome: a pending waiter leaves the queue, and a
/// granted one hands its permits back, which panics, as a permit's drop
/// does, if that would take the count past [`Semaphore::MAX_PERMITS`].
/// Outcomes are only ever set with `state` held, so reading one under it
/// is final.
struct Claim<'a> {
    state: &'a StdMutex<State>,
    outcome: Arc<AtomicU8>,
    needed: usize,
    queued: bool,
}

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        if !self.queued {
            return;
        }
        let mut state = lock(self.state);
        let mut removed = None;
        let woken = match self.outcome.load(Ordering::Acquire) {
            WAITER_GRANTED => state.release(self.needed),
            WAITER_PENDING => {
                removed = state.dequeue(&self.outcome);
                Ok(state.grant())
            }
            _ => Ok(Vec::new()),
        };
        drop(state);
        drop(removed);
        match woken {
            Ok(woken) => wake_all(woken),
            Err(rest) => overflow(rest),
        }
    }
}

/// Error returned by [`Semaphore::acquire`]/[`acquire_many`](Semaphore::acquire_many)
/// (and their `_owned` counterparts): the semaphore was already
/// [closed](Semaphore::close), or was closed while this call was still
/// queued waiting for permits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcquireError(());

impl fmt::Display for AcquireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "semaphore closed")
    }
}

impl std::error::Error for AcquireError {}

/// Error returned by [`Semaphore::try_acquire`]/[`try_acquire_many`](Semaphore::try_acquire_many)
/// (and their `_owned` counterparts).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TryAcquireError {
    /// The semaphore has been [closed](Semaphore::close).
    Closed,
    /// Fewer than the requested number of permits were immediately
    /// available (or another caller was already queued -- see this
    /// module's docs on fairness).
    NoPermits,
}

impl fmt::Display for TryAcquireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TryAcquireError::Closed => write!(f, "semaphore closed"),
            TryAcquireError::NoPermits => write!(f, "no permits available"),
        }
    }
}

impl std::error::Error for TryAcquireError {}

pub struct Semaphore {
    state: StdMutex<State>,
}

impl Semaphore {
    /// The largest number of permits a single `Semaphore` can hold.
    ///
    /// Matches tokio's own constant (`usize::MAX >> 3`) for parity, even
    /// though this crate's `Semaphore` doesn't itself pack extra state
    /// into the same word the way tokio's atomic-counter implementation
    /// does -- there's no technical reason a plain `Mutex<State>`-backed
    /// count couldn't go higher, but there's also no reason for a
    /// well-behaved caller to ever need more permits than this.
    pub const MAX_PERMITS: usize = usize::MAX >> 3;

    pub fn new(permits: usize) -> Self {
        assert!(
            permits <= Self::MAX_PERMITS,
            "a Semaphore cannot be created with more than Semaphore::MAX_PERMITS permits"
        );
        Semaphore {
            state: StdMutex::new(State {
                permits,
                closed: false,
                waiters: VecDeque::new(),
            }),
        }
    }

    /// Like [`new`](Self::new), but usable in a `const` context (e.g. a
    /// `static Semaphore`). It makes the same `MAX_PERMITS` assertion, as
    /// tokio's `const_new` does: in a `static` an out-of-range count is a
    /// compile error, and at run time it panics.
    pub const fn const_new(permits: usize) -> Self {
        assert!(
            permits <= Self::MAX_PERMITS,
            "a Semaphore cannot be created with more than Semaphore::MAX_PERMITS permits"
        );
        Semaphore {
            state: StdMutex::new(State {
                permits,
                closed: false,
                waiters: VecDeque::new(),
            }),
        }
    }

    pub fn available_permits(&self) -> usize {
        lock(&self.state).permits
    }

    /// Permanently removes up to `n` permits, without ever needing to
    /// release them back later -- unlike acquiring and then dropping a
    /// permit without holding it for anything, this never grants (and
    /// thus never wakes) any queued waiter. Returns how many permits
    /// were actually forgotten, which is `n` unless fewer than `n` were
    /// available, in which case it's however many were.
    pub fn forget_permits(&self, n: usize) -> usize {
        let mut guard = lock(&self.state);
        let forgotten = n.min(guard.permits);
        guard.permits -= forgotten;
        forgotten
    }

    /// Adds `n` permits to the semaphore's capacity, waking any queued
    /// waiters that can now proceed -- useful for a semaphore whose
    /// capacity isn't fixed at creation (e.g. starting at zero and
    /// being fed permits as some external resource becomes available).
    ///
    /// # Panics
    /// Panics, as tokio's does and with its messages, if the available
    /// permits left after granting queued waiters would exceed
    /// [`MAX_PERMITS`](Self::MAX_PERMITS). Unlike tokio, which may already
    /// have granted waiters or added the permits when it panics, the
    /// semaphore is then unchanged: no permit is added and no waiter is
    /// granted.
    #[track_caller]
    pub fn add_permits(&self, n: usize) {
        Self::release(&self.state, n);
    }

    /// Closes the semaphore: every queued waiter is woken immediately
    /// with [`AcquireError`], and every `acquire`/`try_acquire` call
    /// (including ones already in flight, once they next poll) fails
    /// from then on instead of ever granting a new permit. Permits
    /// already held by an outstanding [`SemaphorePermit`]/
    /// [`OwnedSemaphorePermit`] are unaffected -- dropping one still
    /// releases its permits back, they just can no longer be acquired
    /// by anyone else. Idempotent: closing an already-closed semaphore
    /// does nothing.
    pub fn close(&self) {
        let mut guard = lock(&self.state);
        if guard.closed {
            return;
        }
        guard.closed = true;
        let waiters = std::mem::take(&mut guard.waiters);
        for waiter in &waiters {
            waiter.outcome.store(WAITER_CLOSED, Ordering::Release);
        }
        drop(guard);
        wake_all(waiters.into_iter().map(|w| w.waker).collect());
    }

    /// Whether [`close`](Self::close) has been called.
    pub fn is_closed(&self) -> bool {
        lock(&self.state).closed
    }

    pub async fn acquire(&self) -> Result<SemaphorePermit<'_>, AcquireError> {
        self.acquire_many(1).await
    }

    pub async fn acquire_many(&self, n: u32) -> Result<SemaphorePermit<'_>, AcquireError> {
        self.acquire_permits(n as usize).await?;
        Ok(SemaphorePermit {
            semaphore: self,
            permits: n as usize,
        })
    }

    /// Acquires a permit without waiting, failing if the semaphore is
    /// closed, or if fewer than one permit is immediately available (or
    /// anyone else is already queued -- see this module's docs on
    /// fairness).
    pub fn try_acquire(&self) -> Result<SemaphorePermit<'_>, TryAcquireError> {
        self.try_acquire_many(1)
    }

    pub fn try_acquire_many(&self, n: u32) -> Result<SemaphorePermit<'_>, TryAcquireError> {
        let needed = n as usize;
        let mut guard = lock(&self.state);
        if guard.closed {
            return Err(TryAcquireError::Closed);
        }
        if guard.waiters.is_empty() && guard.permits >= needed {
            guard.permits -= needed;
            Ok(SemaphorePermit {
                semaphore: self,
                permits: needed,
            })
        } else {
            Err(TryAcquireError::NoPermits)
        }
    }

    /// Like [`acquire`](Self::acquire), but the returned permit owns an
    /// `Arc` clone of the semaphore instead of borrowing it -- usable
    /// past this semaphore's own lifetime, e.g. held across a spawned
    /// task boundary without the call site needing its own separate
    /// `Arc` juggling.
    pub async fn acquire_owned(self: &Arc<Self>) -> Result<OwnedSemaphorePermit, AcquireError> {
        self.acquire_many_owned(1).await
    }

    pub async fn acquire_many_owned(
        self: &Arc<Self>,
        n: u32,
    ) -> Result<OwnedSemaphorePermit, AcquireError> {
        self.acquire_permits(n as usize).await?;
        Ok(OwnedSemaphorePermit {
            semaphore: self.clone(),
            permits: n as usize,
        })
    }

    pub fn try_acquire_owned(self: &Arc<Self>) -> Result<OwnedSemaphorePermit, TryAcquireError> {
        self.try_acquire_many_owned(1)
    }

    pub fn try_acquire_many_owned(
        self: &Arc<Self>,
        n: u32,
    ) -> Result<OwnedSemaphorePermit, TryAcquireError> {
        let needed = n as usize;
        let mut guard = lock(&self.state);
        if guard.closed {
            return Err(TryAcquireError::Closed);
        }
        if guard.waiters.is_empty() && guard.permits >= needed {
            guard.permits -= needed;
            drop(guard);
            Ok(OwnedSemaphorePermit {
                semaphore: self.clone(),
                permits: needed,
            })
        } else {
            Err(TryAcquireError::NoPermits)
        }
    }

    /// The actual wait: resolves once `needed` permits have been
    /// reserved for this call, either taken immediately (nobody queued,
    /// enough available) or granted later by a release -- see this
    /// module's docs for why that later grant is safe to decide (and
    /// commit to) directly in the release path. Resolves to
    /// [`AcquireError`] instead if the semaphore is (or becomes) closed
    /// before that happens.
    async fn acquire_permits(&self, needed: usize) -> Result<(), AcquireError> {
        assert!(needed > 0, "must acquire at least one permit");
        let mut claim = Claim {
            state: &self.state,
            outcome: Arc::new(AtomicU8::new(WAITER_PENDING)),
            needed,
            queued: false,
        };
        std::future::poll_fn(|cx| {
            // A clone made on a pass through the loop that had to store
            // one; see the module docs. Dropped after `guard` on return.
            let mut fresh: Option<Waker> = None;
            loop {
                let mut guard = lock(&self.state);
                if claim.queued {
                    let result = match claim.outcome.load(Ordering::Acquire) {
                        WAITER_GRANTED => Ok(()),
                        WAITER_CLOSED => Err(AcquireError(())),
                        _ if guard.wakes(&claim.outcome, cx.waker()) => return Poll::Pending,
                        _ => {
                            let Some(waker) = fresh.take() else {
                                drop(guard);
                                fresh = Some(cx.waker().clone());
                                continue;
                            };
                            let displaced = guard.rewait(&claim.outcome, waker);
                            drop(guard);
                            drop(displaced);
                            return Poll::Pending;
                        }
                    };
                    claim.queued = false;
                    return Poll::Ready(result);
                }
                if guard.closed {
                    return Poll::Ready(Err(AcquireError(())));
                }
                if guard.waiters.is_empty() && guard.permits >= needed {
                    guard.permits -= needed;
                    return Poll::Ready(Ok(()));
                }
                let Some(waker) = fresh.take() else {
                    drop(guard);
                    fresh = Some(cx.waker().clone());
                    continue;
                };
                guard.waiters.push_back(Waiter {
                    needed,
                    outcome: claim.outcome.clone(),
                    waker,
                });
                claim.queued = true;
                return Poll::Pending;
            }
        })
        .await
    }

    /// Gives `n` permits back (a permit's `Drop`, or [`add_permits`]) and
    /// grants queued waiters (see [`State::release`]), or, past
    /// [`MAX_PERMITS`](Self::MAX_PERMITS), changes nothing and panics once
    /// `state` is unlocked. Adding none does nothing, as in tokio.
    ///
    /// [`add_permits`]: Self::add_permits
    #[track_caller]
    fn release(state: &StdMutex<State>, n: usize) {
        if n == 0 {
            return;
        }
        let mut guard = lock(state);
        let woken = guard.release(n);
        drop(guard);
        match woken {
            Ok(woken) => wake_all(woken),
            Err(rest) => overflow(rest),
        }
    }
}

/// Dropping it returns its permits. As in tokio, that panics if the
/// semaphore's available count would pass [`Semaphore::MAX_PERMITS`],
/// which takes [`Semaphore::add_permits`] beyond the capacity in use;
/// the permits are then not returned.
pub struct SemaphorePermit<'a> {
    semaphore: &'a Semaphore,
    permits: usize,
}

impl fmt::Debug for SemaphorePermit<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SemaphorePermit")
            .field("permits", &self.permits)
            .finish()
    }
}

impl Drop for SemaphorePermit<'_> {
    fn drop(&mut self) {
        Semaphore::release(&self.semaphore.state, self.permits);
    }
}

/// Dropping it returns its permits. As in tokio, that panics if the
/// semaphore's available count would pass [`Semaphore::MAX_PERMITS`],
/// which takes [`Semaphore::add_permits`] beyond the capacity in use;
/// the permits are then not returned.
pub struct OwnedSemaphorePermit {
    semaphore: Arc<Semaphore>,
    permits: usize,
}

impl fmt::Debug for OwnedSemaphorePermit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnedSemaphorePermit")
            .field("permits", &self.permits)
            .finish()
    }
}

impl OwnedSemaphorePermit {
    /// How many permits this one holds (e.g. from
    /// [`acquire_many_owned`](Semaphore::acquire_many_owned)).
    pub fn num_permits(&self) -> usize {
        self.permits
    }

    /// The `Arc`-owned `Semaphore` this permit was acquired from.
    pub fn semaphore(&self) -> &Arc<Semaphore> {
        &self.semaphore
    }

    /// Merges `other`'s permits into `self`, so dropping `self`
    /// afterward releases both at once -- `other` itself is consumed
    /// without releasing anything on its own.
    ///
    /// # Panics
    /// Panics if `other` was acquired from a different `Semaphore`, or if
    /// the merged count would not fit in a `usize`; either way `self` is
    /// unchanged and `other` releases its own permits as it drops.
    pub fn merge(&mut self, other: Self) {
        assert!(
            Arc::ptr_eq(&self.semaphore, &other.semaphore),
            "merge called with permits from different Semaphores"
        );
        let Some(permits) = self.permits.checked_add(other.permits) else {
            panic!("merged permit count would overflow usize");
        };
        self.permits = permits;
        // Skip `other`'s own `Drop` -- it would release its permits
        // back, which its count has instead just been folded into
        // `self` to release later, together.
        std::mem::forget(other);
    }
}

impl Drop for OwnedSemaphorePermit {
    fn drop(&mut self) {
        Semaphore::release(&self.semaphore.state, self.permits);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    /// #1305: the check runs under `state` but panics only once it is
    /// released, so the lock is never poisoned. Base, debug build: the
    /// `+=` overflow panics with `state` held and poisons it.
    #[test]
    fn add_permits_overflow_does_not_poison_the_state_lock() {
        let semaphore = Semaphore::new(1);
        let result = catch_unwind(AssertUnwindSafe(|| semaphore.add_permits(usize::MAX)));
        assert!(result.is_err(), "adding past MAX_PERMITS must panic");
        assert!(!semaphore.state.is_poisoned());
        assert_eq!(semaphore.available_permits(), 1);
    }

    /// #1305: `merge` sums two counts outside any lock; the sum must not
    /// wrap in a release build, and on a panic both permits stay as they
    /// were (`other` is released by unwinding, `self` keeps its count).
    #[test]
    fn merge_past_usize_panics_without_wrapping() {
        let semaphore = Arc::new(Semaphore::new(0));
        // Never acquired, so never released: dropping it would overflow
        // (and, while a failed assertion unwinds, abort the test binary).
        let mut big = std::mem::ManuallyDrop::new(OwnedSemaphorePermit {
            semaphore: semaphore.clone(),
            permits: usize::MAX,
        });
        let one = OwnedSemaphorePermit {
            semaphore: semaphore.clone(),
            permits: 1,
        };
        let err = catch_unwind(AssertUnwindSafe(|| big.merge(one)))
            .expect_err("a merged count past usize::MAX must panic");
        let msg = err.downcast_ref::<String>().map(String::as_str);
        let msg = msg.or_else(|| err.downcast_ref::<&str>().copied());
        assert_eq!(msg, Some("merged permit count would overflow usize"));
        assert_eq!(big.num_permits(), usize::MAX);
        assert_eq!(semaphore.available_permits(), 1);
    }
}
