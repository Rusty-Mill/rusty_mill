//! [`WaitList`]: the wakers of the futures waiting for one condition,
//! kept under the same lock as that condition, one keyed slot per
//! waiting future. Shared by [`super::Barrier`], [`super::OnceCell`]
//! and [`super::SetOnce`].
//!
//! A [`Waiting`] owns a future's slot: a re-poll replaces the slot's
//! waker in place, and resolving or dropping the future removes it, so a
//! waiter re-polled many times, or cancelled (a `select!` branch that
//! lost, a `timeout`), leaves nothing behind (#579). The pattern
//! `watch::Receiver::changed` already uses.
//!
//! A waker's clone or drop may run executor code that re-enters the
//! primitive, so neither happens with the lock held: a poll clones its
//! waker before locking, and replaced or removed wakers are dropped
//! after unlocking. The completing side takes every waker with
//! [`WaitList::take`] and wakes them once it has unlocked.

use std::sync::Mutex;
use std::task::{Context, Poll, Waker};

/// Wakers of every future parked on one condition, keyed by slot.
pub(super) struct WaitList {
    slots: Vec<(u64, Waker)>,
    next_key: u64,
}

impl WaitList {
    pub(super) const fn new() -> Self {
        WaitList {
            slots: Vec::new(),
            next_key: 0,
        }
    }

    /// Removes every slot, returning the wakers for the caller to wake
    /// once it has unlocked. Their [`Waiting`]s find their slots gone.
    pub(super) fn take(&mut self) -> Vec<Waker> {
        std::mem::take(&mut self.slots)
            .into_iter()
            .map(|(_, waker)| waker)
            .collect()
    }

    /// Stores `waker` in slot `key`, taking a new slot if `key` is
    /// `None` or was taken. Returns the waker to drop once unlocked: the
    /// one displaced, or `waker` itself when the stored one already
    /// wakes the same task.
    fn park(&mut self, key: &mut Option<u64>, waker: Waker) -> Option<Waker> {
        let slot = key.and_then(|key| self.slots.iter_mut().find(|(k, _)| *k == key));
        match slot {
            Some((_, stored)) if stored.will_wake(&waker) => Some(waker),
            Some((_, stored)) => Some(std::mem::replace(stored, waker)),
            None => {
                let new = self.next_key;
                self.next_key = new.wrapping_add(1);
                self.slots.push((new, waker));
                *key = Some(new);
                None
            }
        }
    }

    /// Removes slot `key`, if [`take`](Self::take) has not, returning its
    /// waker to drop once unlocked.
    fn remove(&mut self, key: &mut Option<u64>) -> Option<Waker> {
        let key = key.take()?;
        let index = self.slots.iter().position(|(k, _)| *k == key)?;
        Some(self.slots.swap_remove(index).1)
    }
}

/// State behind a primitive's lock that holds its [`WaitList`].
pub(super) trait HasWaitList {
    fn wait_list(&mut self) -> &mut WaitList;
}

/// One waiting future's slot in the [`WaitList`] behind `state`,
/// removed when the future resolves or is dropped.
pub(super) struct Waiting<'a, S: HasWaitList> {
    state: &'a Mutex<S>,
    key: Option<u64>,
}

impl<'a, S: HasWaitList> Waiting<'a, S> {
    pub(super) fn new(state: &'a Mutex<S>) -> Self {
        Waiting { state, key: None }
    }

    /// Resolves once `done` holds, checked under the lock; until then
    /// keeps this future's slot holding `cx`'s waker. Checking and
    /// parking under one lock is what lets the completing side, which
    /// changes the condition and takes the wakers under that lock, never
    /// miss a waiter.
    pub(super) fn poll_until(
        &mut self,
        cx: &mut Context<'_>,
        done: impl FnOnce(&S) -> bool,
    ) -> Poll<()> {
        // Cloned before locking: cloning a waker runs arbitrary code.
        let waker = cx.waker().clone();
        let mut guard = self.state.lock().unwrap();
        let (poll, stale) = if done(&guard) {
            (Poll::Ready(()), guard.wait_list().remove(&mut self.key))
        } else {
            (Poll::Pending, guard.wait_list().park(&mut self.key, waker))
        };
        drop(guard);
        // Dropped outside the lock: a waker's destructor may re-enter.
        drop(stale);
        poll
    }
}

impl<S: HasWaitList> Drop for Waiting<'_, S> {
    fn drop(&mut self) {
        if self.key.is_none() {
            return;
        }
        // Never panic in drop: a poisoned lock has nothing left to clean.
        let Ok(mut guard) = self.state.lock() else {
            return;
        };
        let stale = guard.wait_list().remove(&mut self.key);
        drop(guard);
        drop(stale);
    }
}
