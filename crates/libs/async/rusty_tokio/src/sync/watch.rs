//! A single-latest-value broadcast: the sender always holds exactly one
//! current value, and `Receiver::changed().await` resolves once it's
//! been updated since this receiver last observed it. Unlike
//! [`super::broadcast`] (a multi-consumer channel that queues every
//! message, reporting `Lagged` if a receiver falls behind), there's no
//! queue and no lagging here -- a receiver that misses several updates
//! in a row just sees the latest value whenever it does check, not
//! every intermediate one. Useful for something like
//! "the current configuration" or "has shutdown been requested" that
//! many tasks want to observe, exactly the shape
//! [`crate::Handle::shutdown_notified`]/`is_shutting_down` hand-rolled
//! as a one-off special case before this existed.
//!
//! The version counter, the wakers waiting on `changed()`, and the
//! value itself all live under a *single* lock -- the same reasoning
//! [`super::mpsc`]'s module docs give for why its queue and wakers
//! share one lock too: checking "has the version changed" and
//! registering a waker if not need to happen atomically, or a `send`
//! landing in the gap between those two steps would update the version
//! and drain the (not-yet-registered) waker list, and this receiver
//! would then wait for a second, distinct change instead of noticing
//! the one it just missed -- a real lost wakeup, not a hypothetical one.

use std::fmt;
use std::future::Future;
use std::ops::Deref;
use std::pin::Pin;
use std::sync::{Arc, Mutex as StdMutex, MutexGuard};
use std::task::{Context, Poll, Waker};

struct Inner<T> {
    value: T,
    version: u64,
    /// Waiting `changed()` futures, keyed so a dropped wait can remove its
    /// own entry instead of leaving it until the next send.
    wakers: Vec<(u64, Waker)>,
    next_key: u64,
    sender_alive: bool,
    receivers_alive: usize,
}

struct Shared<T> {
    inner: StdMutex<Inner<T>>,
}

pub fn channel<T>(initial: T) -> (Sender<T>, Receiver<T>) {
    let shared = Arc::new(Shared {
        inner: StdMutex::new(Inner {
            value: initial,
            version: 0,
            wakers: Vec::new(),
            next_key: 0,
            sender_alive: true,
            receivers_alive: 1,
        }),
    });
    (
        Sender {
            shared: shared.clone(),
        },
        Receiver {
            shared,
            seen_version: 0,
        },
    )
}

pub struct Sender<T> {
    shared: Arc<Shared<T>>,
}

impl<T> Sender<T> {
    /// Replaces the current value and wakes every receiver waiting on
    /// [`Receiver::changed`]. Fails (handing `value` back) if every
    /// receiver has already dropped -- there would be nobody left to
    /// ever observe it.
    pub fn send(&self, value: T) -> Result<(), SendError<T>> {
        let mut guard = self.shared.inner.lock().unwrap();
        if guard.receivers_alive == 0 {
            drop(guard);
            return Err(SendError(value));
        }
        guard.value = value;
        guard.version += 1;
        let wakers = std::mem::take(&mut guard.wakers);
        drop(guard);
        for (_, waker) in wakers {
            waker.wake();
        }
        Ok(())
    }

    /// Modifies the current value in place via `modify`, without
    /// needing to hand over a whole new value up front. Always
    /// succeeds and always counts as a change, even if `modify` happens
    /// to leave the value equal to what it was.
    pub fn send_modify<F: FnOnce(&mut T)>(&self, modify: F) {
        let mut guard = self.shared.inner.lock().unwrap();
        modify(&mut guard.value);
        guard.version += 1;
        let wakers = std::mem::take(&mut guard.wakers);
        drop(guard);
        for (_, waker) in wakers {
            waker.wake();
        }
    }

    /// Reads the current value without waiting -- the sender's own side
    /// of [`Receiver::borrow`].
    pub fn borrow(&self) -> Ref<'_, T> {
        Ref {
            guard: self.shared.inner.lock().unwrap(),
        }
    }

    pub fn receiver_count(&self) -> usize {
        self.shared.inner.lock().unwrap().receivers_alive
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        let mut guard = self.shared.inner.lock().unwrap();
        guard.sender_alive = false;
        let wakers = std::mem::take(&mut guard.wakers);
        drop(guard);
        // Wake everyone still waiting so they observe the channel is
        // now closed instead of waiting forever.
        for (_, waker) in wakers {
            waker.wake();
        }
    }
}

pub struct Receiver<T> {
    shared: Arc<Shared<T>>,
    seen_version: u64,
}

impl<T> fmt::Debug for Sender<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sender").finish_non_exhaustive()
    }
}

impl<T> fmt::Debug for Receiver<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Receiver").finish_non_exhaustive()
    }
}

impl<T> Receiver<T> {
    /// Reads the current value without waiting or marking it seen --
    /// a later [`changed`](Self::changed) call still reports a change
    /// if the version this receiver last *marked* as seen is still
    /// behind, regardless of how many times `borrow` has been called
    /// since.
    pub fn borrow(&self) -> Ref<'_, T> {
        Ref {
            guard: self.shared.inner.lock().unwrap(),
        }
    }

    /// Like [`borrow`](Self::borrow), but also marks the current
    /// version as seen -- equivalent to (but without the intermediate
    /// wait) calling [`changed`](Self::changed) and then `borrow`.
    pub fn borrow_and_update(&mut self) -> Ref<'_, T> {
        let guard = self.shared.inner.lock().unwrap();
        self.seen_version = guard.version;
        Ref { guard }
    }

    /// Resolves once the value has changed since this receiver last
    /// marked a version as seen (via this call or
    /// [`borrow_and_update`](Self::borrow_and_update)) -- immediately,
    /// if it already has by the time this is first polled. Fails once
    /// the sender has dropped and no further changes are possible.
    pub async fn changed(&mut self) -> Result<(), RecvError> {
        changed(&self.shared, &mut self.seen_version).await
    }

    /// Waits until `f` accepts the current value, marking it seen, and
    /// returns it. Checks the current value first, so it resolves
    /// immediately if that already matches. Fails once the sender has
    /// dropped without a matching value.
    pub async fn wait_for(
        &mut self,
        mut f: impl FnMut(&T) -> bool,
    ) -> Result<Ref<'_, T>, RecvError> {
        // Split borrows: the returned guard borrows `shared` while the
        // loop keeps updating `seen_version`.
        let (shared, seen) = (&self.shared, &mut self.seen_version);
        loop {
            // Scoped, not `drop`ped, so the guard is not held across the
            // await below and the future stays `Send`.
            {
                let guard = shared.inner.lock().unwrap();
                // Mark this version seen before inspecting it, so `changed`
                // below waits for a newer one rather than handing the
                // predicate the same value twice.
                *seen = guard.version;
                // As in tokio, a panicking predicate must not poison the
                // channel: release the lock before resuming the unwind.
                let matched =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&guard.value)));
                let matched = match matched {
                    Ok(matched) => matched,
                    Err(panic) => {
                        drop(guard);
                        std::panic::resume_unwind(panic);
                    }
                };
                if matched {
                    return Ok(Ref { guard });
                }
            }
            changed(shared, seen).await?;
        }
    }
}

/// [`Receiver::changed`] over split borrows, so [`Receiver::wait_for`] can
/// hold a borrow of `shared` across it.
fn changed<'a, T>(shared: &'a Shared<T>, seen_version: &'a mut u64) -> Changed<'a, T> {
    Changed {
        shared,
        seen_version,
        key: None,
    }
}

/// The future behind `changed()`. It keeps at most one waker registered,
/// refreshes it on each poll, and removes it when dropped, so a wait that
/// keeps losing a `select!` does not leave registrations behind.
struct Changed<'a, T> {
    shared: &'a Shared<T>,
    seen_version: &'a mut u64,
    key: Option<u64>,
}

impl<T> Future for Changed<'_, T> {
    type Output = Result<(), RecvError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let mut guard = this.shared.inner.lock().unwrap();
        if guard.version != *this.seen_version {
            *this.seen_version = guard.version;
            let stale = this.unregister(&mut guard);
            drop(guard);
            drop(stale);
            return Poll::Ready(Ok(()));
        }
        if !guard.sender_alive {
            let stale = this.unregister(&mut guard);
            drop(guard);
            drop(stale);
            return Poll::Ready(Err(RecvError(())));
        }
        let slot = this
            .key
            .and_then(|key| guard.wakers.iter_mut().find(|(k, _)| *k == key));
        let stale = match slot {
            Some((_, waker)) if waker.will_wake(cx.waker()) => None,
            Some((_, waker)) => Some(std::mem::replace(waker, cx.waker().clone())),
            None => {
                let key = guard.next_key;
                guard.next_key += 1;
                guard.wakers.push((key, cx.waker().clone()));
                this.key = Some(key);
                None
            }
        };
        drop(guard);
        // Dropped outside the lock: a waker's destructor may re-enter.
        drop(stale);
        Poll::Pending
    }
}

impl<T> Changed<'_, T> {
    /// Removes this future's waker, if a send has not already drained it.
    fn unregister(&mut self, inner: &mut Inner<T>) -> Option<Waker> {
        let key = self.key.take()?;
        let index = inner.wakers.iter().position(|(k, _)| *k == key)?;
        Some(inner.wakers.swap_remove(index).1)
    }
}

impl<T> Drop for Changed<'_, T> {
    fn drop(&mut self) {
        if self.key.is_none() {
            return;
        }
        let shared = self.shared;
        // Never panic in drop: a poisoned lock has nothing left to clean.
        let Ok(mut guard) = shared.inner.lock() else {
            return;
        };
        let stale = self.unregister(&mut guard);
        drop(guard);
        drop(stale);
    }
}

impl<T> Clone for Receiver<T> {
    fn clone(&self) -> Self {
        self.shared.inner.lock().unwrap().receivers_alive += 1;
        Receiver {
            shared: self.shared.clone(),
            // Starts wherever *this* receiver's own last-seen marker
            // is, not the channel's current version -- if the original
            // hasn't yet observed the latest value, neither has the
            // clone.
            seen_version: self.seen_version,
        }
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        self.shared.inner.lock().unwrap().receivers_alive -= 1;
    }
}

/// A read guard over a [`watch`](self) channel's current value, held
/// via [`Sender::borrow`]/[`Receiver::borrow`]/
/// [`Receiver::borrow_and_update`].
pub struct Ref<'a, T> {
    guard: MutexGuard<'a, Inner<T>>,
}

impl<T> Deref for Ref<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.guard.value
    }
}

/// Every receiver has dropped, so `send`/`send_modify` would have
/// nobody left to ever observe it.
pub struct SendError<T>(pub T);

impl<T> std::fmt::Debug for SendError<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SendError(..)")
    }
}

impl<T> std::fmt::Display for SendError<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sending on a watch channel with no receivers left")
    }
}

impl<T> std::error::Error for SendError<T> {}

/// The sender was dropped, so no further changes are possible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecvError(());

impl std::fmt::Display for RecvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "watch channel sender dropped, no further changes possible"
        )
    }
}

impl std::error::Error for RecvError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn registrations<T>(shared: &Shared<T>) -> usize {
        shared.inner.lock().unwrap().wakers.len()
    }

    fn poll_once<F: Future>(fut: F) -> Poll<F::Output> {
        let mut fut = std::pin::pin!(fut);
        fut.as_mut().poll(&mut Context::from_waker(Waker::noop()))
    }

    #[test]
    fn dropped_waits_do_not_accumulate_registrations() {
        let (_tx, mut rx) = channel(0);
        let shared = rx.shared.clone();
        for _ in 0..1000 {
            assert!(poll_once(rx.changed()).is_pending());
        }
        assert_eq!(registrations(&shared), 0);
    }

    #[test]
    fn repolled_wait_keeps_one_registration() {
        let (tx, mut rx) = channel(0);
        let shared = rx.shared.clone();
        {
            let mut fut = std::pin::pin!(rx.changed());
            let mut cx = Context::from_waker(Waker::noop());
            for _ in 0..10 {
                assert!(fut.as_mut().poll(&mut cx).is_pending());
            }
            assert_eq!(registrations(&shared), 1);
            tx.send(1).unwrap();
            assert!(matches!(fut.as_mut().poll(&mut cx), Poll::Ready(Ok(()))));
        }
        assert_eq!(registrations(&shared), 0);
    }

    #[test]
    fn dropping_one_wait_keeps_the_others_registered() {
        let (_tx, mut rx) = channel(0);
        let mut other = rx.clone();
        let shared = rx.shared.clone();
        let mut cx = Context::from_waker(Waker::noop());
        let mut kept = std::pin::pin!(other.changed());
        assert!(kept.as_mut().poll(&mut cx).is_pending());
        assert!(poll_once(rx.changed()).is_pending());
        assert_eq!(registrations(&shared), 1);
    }
}
