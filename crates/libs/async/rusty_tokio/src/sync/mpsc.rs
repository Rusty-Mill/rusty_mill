//! A bounded, multi-producer single-consumer queue. `send().await`
//! suspends the sending task while the buffer is full instead of
//! blocking the worker thread; `recv().await` suspends while it's
//! empty. [`unbounded_channel`] below is the same idea with the
//! capacity check (and therefore any need for the sender to ever wait)
//! removed entirely -- `UnboundedSender::send` is a plain, synchronous
//! method, not `async fn`, since there's genuinely nothing to await.
//!
//! All of the queue, the wakers waiting to send, and the one waker
//! waiting to receive live under a *single* lock. That's deliberate:
//! an earlier version of this split them across separate mutexes (one
//! for the queue, one for the waiter lists), and checking "is there
//! room" under one lock and then registering a waker under a different
//! lock leaves a window where a slot can free up in between and the
//! wakeup is sent to an empty waiter list -- a real lost wakeup, not a
//! hypothetical one (it deadlocked the first version of this file's
//! test suite). One lock covering the check-and-register makes that
//! race impossible by construction instead of requiring a careful
//! recheck-after-register dance. `unbounded_channel`'s own `Inner`
//! reuses this same one-lock shape for the queue/`recv_waker` pair, just
//! without a `send_waiters` list at all -- nothing ever waits to send.
//!
//! Each waiting `send`/`reserve` holds a [`SendWait`]: one keyed entry in
//! `send_waiters`, refreshed rather than duplicated on re-poll, and removed
//! when the future is dropped. A future dropped after its wake was already
//! delivered hands that wake to the next waiter, since the capacity it was
//! woken for is still free. Without this, a cancelled sender's stale entry
//! could absorb the one wake a freed slot sends, leaving a live sender
//! parked while the channel has room.

use std::cell::Cell;
use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::task::{Context, Poll, Waker};

struct Inner<T> {
    queue: VecDeque<T>,
    capacity: usize,
    /// Slots claimed by an outstanding [`Permit`]/[`OwnedPermit`] (or
    /// one not-yet-doled-out by a live [`PermitIterator`]) but not yet
    /// filled with an actual value -- counted separately from
    /// `queue.len()` so `reserve`'s "is there room" check sees them the
    /// same way an already-queued item would (`queue.len() + reserved
    /// < capacity`), without needing a placeholder value in `queue`
    /// itself.
    reserved: usize,
    /// Waiting senders, oldest first, keyed by their [`SendWait`].
    send_waiters: VecDeque<(u64, Waker)>,
    next_waiter: u64,
    recv_waker: Option<Waker>,
    senders_alive: usize,
    receiver_alive: bool,
}

impl<T> Inner<T> {
    /// Wake the oldest waiting sender, if any. Returns the waker so the
    /// caller can wake it after unlocking.
    fn pop_send_waiter(&mut self) -> Option<Waker> {
        self.send_waiters.pop_front().map(|(_, waker)| waker)
    }

    /// Queue `wait` with `waker`, or refresh its waker if still queued.
    /// Returns the waker it replaced, for the caller to drop after
    /// unlocking (see [`Self::stop_waiting`]).
    #[must_use = "drop the replaced waker after unlocking"]
    fn wait_to_send(&mut self, wait: &mut Option<u64>, waker: &Waker) -> Option<Waker> {
        if let Some(key) = *wait {
            if let Some((_, queued)) = self.send_waiters.iter_mut().find(|(k, _)| *k == key) {
                return Some(std::mem::replace(queued, waker.clone()));
            }
        }
        let key = self.next_waiter;
        self.next_waiter = self.next_waiter.wrapping_add(1);
        self.send_waiters.push_back((key, waker.clone()));
        *wait = Some(key);
        None
    }

    /// `wait` finished (capacity taken, or the channel closed): leave the
    /// queue without consuming anyone else's wake. Returns the removed
    /// waker for the caller to drop after unlocking: a waker may own a
    /// `Sender` of this very channel, whose `Drop` takes this lock.
    #[must_use = "drop the removed waker after unlocking"]
    fn stop_waiting(&mut self, wait: &mut Option<u64>) -> Option<Waker> {
        let key = wait.take()?;
        let i = self.send_waiters.iter().position(|(k, _)| *k == key)?;
        self.send_waiters.remove(i).map(|(_, waker)| waker)
    }
}

struct Shared<T> {
    inner: Mutex<Inner<T>>,
}

/// One `send`/`reserve` call's place in `send_waiters` (see the module
/// docs). `key` is `None` until it first waits and again once it finishes.
struct SendWait<'a, T> {
    shared: &'a Shared<T>,
    key: Option<u64>,
}

impl<'a, T> SendWait<'a, T> {
    fn new(shared: &'a Shared<T>) -> Self {
        SendWait { shared, key: None }
    }
}

impl<T> Drop for SendWait<'_, T> {
    fn drop(&mut self) {
        let Some(key) = self.key.take() else {
            return;
        };
        // Never panic in drop: a poisoned lock still guards a consistent
        // waiter list (every critical section here leaves it whole).
        let mut guard = self
            .shared
            .inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(i) = guard.send_waiters.iter().position(|(k, _)| *k == key) {
            let removed = guard.send_waiters.remove(i);
            drop(guard); // before `removed`: its waker may own a `Sender`
            drop(removed);
            return;
        }
        // Woken but cancelled before using the wake: pass it on.
        let next = guard.pop_send_waiter();
        drop(guard);
        if let Some(waker) = next {
            waker.wake();
        }
    }
}

impl<T> Shared<T> {
    /// Gives back one previously-reserved slot that's never going to be
    /// filled after all (a [`Permit`]/[`OwnedPermit`] dropped without
    /// sending, or a [`PermitIterator`] dropped with some of its permits
    /// never handed out) -- wakes up to `n` queued senders, since that
    /// many could now have room where they didn't before.
    fn release_reserved(&self, n: usize) {
        let mut guard = self.inner.lock().unwrap();
        guard.reserved -= n;
        let mut woken = Vec::new();
        for _ in 0..n {
            match guard.pop_send_waiter() {
                Some(waker) => woken.push(waker),
                None => break,
            }
        }
        drop(guard);
        for waker in woken {
            waker.wake();
        }
    }

    /// Converts one previously-reserved slot into an actual queued
    /// value ([`Permit::send`]/[`OwnedPermit::send`]) -- total capacity
    /// used doesn't change, so (unlike [`release_reserved`](Self::release_reserved))
    /// this wakes the *receiver*, not another sender.
    fn fill_reserved(&self, value: T) {
        let mut guard = self.inner.lock().unwrap();
        guard.reserved -= 1;
        guard.queue.push_back(value);
        let waker = guard.recv_waker.take();
        drop(guard);
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

pub struct Sender<T> {
    shared: Arc<Shared<T>>,
}

pub struct Receiver<T> {
    shared: Arc<Shared<T>>,
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

pub fn channel<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    assert!(capacity > 0, "mpsc channel capacity must be positive");
    let shared = Arc::new(Shared {
        inner: Mutex::new(Inner {
            queue: VecDeque::new(),
            capacity,
            reserved: 0,
            send_waiters: VecDeque::new(),
            next_waiter: 0,
            recv_waker: None,
            senders_alive: 1,
            receiver_alive: true,
        }),
    });
    (
        Sender {
            shared: shared.clone(),
        },
        Receiver { shared },
    )
}

impl<T> Sender<T> {
    /// Blocking form of [`send`](Self::send), for synchronous code -- a
    /// plain OS thread bridging blocking I/O into async tasks. Parks the
    /// calling thread until it completes; no runtime is needed.
    ///
    /// Never call this from an async task: it parks the worker thread
    /// running it, stalling every other task queued there.
    pub fn blocking_send(&self, value: T) -> Result<(), SendError<T>> {
        crate::runtime::block_on_inner(self.send(value))
    }

    pub async fn send(&self, value: T) -> Result<(), SendError<T>> {
        let mut value = Some(value);
        let mut wait = SendWait::new(&self.shared);
        let sent = std::future::poll_fn(|cx| {
            let mut guard = self.shared.inner.lock().unwrap();
            if !guard.receiver_alive {
                wait.key = None;
                return Poll::Ready(false);
            }
            if guard.queue.len() + guard.reserved < guard.capacity {
                let stale = guard.stop_waiting(&mut wait.key);
                guard
                    .queue
                    .push_back(value.take().expect("polled after completion"));
                let waker = guard.recv_waker.take();
                drop(guard);
                drop(stale);
                if let Some(waker) = waker {
                    waker.wake();
                }
                return Poll::Ready(true);
            }
            let stale = guard.wait_to_send(&mut wait.key, cx.waker());
            drop(guard);
            drop(stale);
            Poll::Pending
        })
        .await;

        if sent {
            Ok(())
        } else {
            Err(SendError(
                value.expect("value not consumed on failure path"),
            ))
        }
    }

    /// Sends without waiting, failing immediately if the channel is
    /// either full or closed -- the value is handed back in either
    /// case via [`TrySendError`], not lost.
    pub fn try_send(&self, value: T) -> Result<(), TrySendError<T>> {
        let mut guard = self.shared.inner.lock().unwrap();
        if !guard.receiver_alive {
            drop(guard);
            return Err(TrySendError::Closed(value));
        }
        if guard.queue.len() + guard.reserved < guard.capacity {
            guard.queue.push_back(value);
            let waker = guard.recv_waker.take();
            drop(guard);
            if let Some(waker) = waker {
                waker.wake();
            }
            Ok(())
        } else {
            drop(guard);
            Err(TrySendError::Full(value))
        }
    }

    /// Like [`send`](Self::send), but fails instead of waiting past
    /// `timeout` for capacity -- built on [`reserve`](Self::reserve)
    /// rather than timing out `send` directly, so `value` is never
    /// moved anywhere until capacity is actually secured, and can
    /// always be handed back via [`SendTimeoutError`] if it isn't in
    /// time.
    pub async fn send_timeout(
        &self,
        value: T,
        timeout: std::time::Duration,
    ) -> Result<(), SendTimeoutError<T>> {
        match crate::time::timeout(timeout, self.reserve()).await {
            Ok(Ok(permit)) => {
                permit.send(value);
                Ok(())
            }
            Ok(Err(_closed)) => Err(SendTimeoutError::Closed(value)),
            Err(_elapsed) => Err(SendTimeoutError::Timeout(value)),
        }
    }

    /// Waits for capacity, then reserves one slot without filling it
    /// yet -- unlike [`send`](Self::send), the value to put there
    /// doesn't need to exist yet at the time the wait for room happens.
    /// Fill it (synchronously, since the slot's already reserved) via
    /// [`Permit::send`].
    pub async fn reserve(&self) -> Result<Permit<'_, T>, SendError<()>> {
        let mut wait = SendWait::new(&self.shared);
        let reserved = std::future::poll_fn(|cx| self.poll_reserve(cx, 1, &mut wait)).await;
        if reserved {
            Ok(Permit {
                sender: self,
                used: Cell::new(false),
            })
        } else {
            Err(SendError(()))
        }
    }

    /// Like [`reserve`](Self::reserve), but reserves `n` slots at once,
    /// handed back one at a time via the returned [`PermitIterator`].
    pub async fn reserve_many(&self, n: usize) -> Result<PermitIterator<'_, T>, SendError<()>> {
        let mut wait = SendWait::new(&self.shared);
        let reserved = std::future::poll_fn(|cx| self.poll_reserve(cx, n, &mut wait)).await;
        if reserved {
            Ok(PermitIterator {
                sender: self,
                remaining: n,
            })
        } else {
            Err(SendError(()))
        }
    }

    /// Like [`reserve`](Self::reserve), but the returned permit owns a
    /// clone of this `Sender` instead of borrowing it -- usable past
    /// this particular `Sender`'s own lifetime (e.g. moved into a
    /// spawned task), at the cost of consuming this one to make it.
    pub async fn reserve_owned(self) -> Result<OwnedPermit<T>, SendError<()>> {
        let mut wait = SendWait::new(&self.shared);
        let reserved = std::future::poll_fn(|cx| self.poll_reserve(cx, 1, &mut wait)).await;
        drop(wait);
        if reserved {
            Ok(OwnedPermit { sender: Some(self) })
        } else {
            Err(SendError(()))
        }
    }

    /// Reserves one slot without waiting, failing immediately if the
    /// channel is either full or closed.
    pub fn try_reserve(&self) -> Result<Permit<'_, T>, TrySendError<()>> {
        let mut guard = self.shared.inner.lock().unwrap();
        if !guard.receiver_alive {
            return Err(TrySendError::Closed(()));
        }
        if guard.queue.len() + guard.reserved < guard.capacity {
            guard.reserved += 1;
            Ok(Permit {
                sender: self,
                used: Cell::new(false),
            })
        } else {
            Err(TrySendError::Full(()))
        }
    }

    /// Like [`try_reserve`](Self::try_reserve), but owning -- and, since
    /// it consumes `self`, hands `self` back inside the error on
    /// failure rather than losing it.
    pub fn try_reserve_owned(self) -> Result<OwnedPermit<T>, TrySendError<Self>> {
        let mut guard = self.shared.inner.lock().unwrap();
        if !guard.receiver_alive {
            drop(guard);
            return Err(TrySendError::Closed(self));
        }
        if guard.queue.len() + guard.reserved < guard.capacity {
            guard.reserved += 1;
            drop(guard);
            Ok(OwnedPermit { sender: Some(self) })
        } else {
            drop(guard);
            Err(TrySendError::Full(self))
        }
    }

    /// Shared poll body for [`reserve`](Self::reserve)/
    /// [`reserve_many`](Self::reserve_many)/[`reserve_owned`](Self::reserve_owned)
    /// -- `Ready(true)` once `n` slots are reserved, `Ready(false)` if
    /// the receiver's gone (no reservation made), `Pending` (queued
    /// behind any other waiting sender) otherwise.
    fn poll_reserve(
        &self,
        cx: &mut std::task::Context<'_>,
        n: usize,
        wait: &mut SendWait<'_, T>,
    ) -> Poll<bool> {
        let mut guard = self.shared.inner.lock().unwrap();
        if !guard.receiver_alive {
            wait.key = None;
            return Poll::Ready(false);
        }
        let ready = guard.queue.len() + guard.reserved + n <= guard.capacity;
        let stale = if ready {
            guard.reserved += n;
            guard.stop_waiting(&mut wait.key)
        } else {
            guard.wait_to_send(&mut wait.key, cx.waker())
        };
        drop(guard);
        drop(stale);
        if ready {
            Poll::Ready(true)
        } else {
            Poll::Pending
        }
    }
}

impl<T> Sender<T> {
    /// A weak handle that doesn't keep the channel's sender count alive
    /// by itself -- [`WeakSender::upgrade`] later hands back a real
    /// `Sender` only if at least one other still exists.
    pub fn downgrade(&self) -> WeakSender<T> {
        WeakSender {
            shared: Arc::downgrade(&self.shared),
        }
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        self.shared.inner.lock().unwrap().senders_alive += 1;
        Sender {
            shared: self.shared.clone(),
        }
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        let mut guard = self.shared.inner.lock().unwrap();
        guard.senders_alive -= 1;
        if guard.senders_alive == 0 {
            // That was the last sender: wake the receiver so it can see
            // the channel is now closed instead of waiting forever.
            let waker = guard.recv_waker.take();
            drop(guard);
            if let Some(waker) = waker {
                waker.wake();
            }
        }
    }
}

/// A handle to a bounded channel's sender that doesn't keep it (or its
/// sender count) alive by itself -- obtained via [`Sender::downgrade`].
/// Useful for a background task that wants to send *if* the channel is
/// still in use, without that reference alone being why it's still in
/// use (e.g. a task holding one of these instead of a real `Sender`
/// won't stop the receiver from ever seeing the channel close).
pub struct WeakSender<T> {
    shared: Weak<Shared<T>>,
}

impl<T> WeakSender<T> {
    /// Hands back a real, usable `Sender` -- but only if at least one
    /// other `Sender` (or another still-live `WeakSender::upgrade`
    /// result) already exists. `None` once every real `Sender` has been
    /// dropped, even if the `Receiver` is still alive: a `WeakSender`
    /// tracks "is there still a real sender", not just "is the
    /// underlying channel state still allocated".
    pub fn upgrade(&self) -> Option<Sender<T>> {
        let shared = self.shared.upgrade()?;
        let mut guard = shared.inner.lock().unwrap();
        if guard.senders_alive == 0 {
            return None;
        }
        guard.senders_alive += 1;
        drop(guard);
        Some(Sender { shared })
    }
}

impl<T> Clone for WeakSender<T> {
    fn clone(&self) -> Self {
        WeakSender {
            shared: self.shared.clone(),
        }
    }
}

impl<T> fmt::Debug for WeakSender<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WeakSender").finish()
    }
}

/// A reserved (but not yet filled) slot in a bounded channel's buffer,
/// obtained via [`Sender::reserve`]/[`Sender::reserve_many`]. Sending
/// through it (via [`send`](Self::send)) is then synchronous and
/// infallible -- the capacity wait already happened at reservation
/// time -- and dropping it unused gives the slot back for someone else
/// to reserve or send into.
#[must_use = "a Permit does nothing unless `send` is called on it -- dropping it unused just releases the reservation"]
pub struct Permit<'a, T> {
    sender: &'a Sender<T>,
    used: Cell<bool>,
}

impl<T> Permit<'_, T> {
    /// Fills this reservation with `value`. Always succeeds and never
    /// waits -- the capacity was already secured when this `Permit` was
    /// obtained.
    pub fn send(self, value: T) {
        self.used.set(true);
        self.sender.shared.fill_reserved(value);
    }
}

impl<T> Drop for Permit<'_, T> {
    fn drop(&mut self) {
        if !self.used.get() {
            self.sender.shared.release_reserved(1);
        }
    }
}

/// Like [`Permit`], but owns a clone of the `Sender` it came from
/// instead of borrowing it -- obtained via
/// [`Sender::reserve_owned`]/[`Sender::try_reserve_owned`].
#[must_use = "an OwnedPermit does nothing unless `send` (or `release`) is called on it -- dropping it unused just releases the reservation"]
pub struct OwnedPermit<T> {
    // `None` only in between `send`/`release` taking it and the
    // `OwnedPermit` itself actually finishing being dropped -- lets
    // both of those hand the underlying `Sender` back by value without
    // needing `unsafe`/`ManuallyDrop` to move out of a `Drop` type.
    sender: Option<Sender<T>>,
}

impl<T> OwnedPermit<T> {
    /// Fills this reservation with `value`, handing back the `Sender`
    /// this permit was reserved from (unlike [`Permit::send`], which has
    /// no owned `Sender` to give back).
    pub fn send(mut self, value: T) -> Sender<T> {
        let sender = self.sender.take().expect("OwnedPermit already consumed");
        sender.shared.fill_reserved(value);
        sender
    }

    /// Gives up this reservation without sending anything, handing back
    /// the `Sender` -- equivalent to dropping the permit, except the
    /// `Sender` isn't lost along with it.
    pub fn release(mut self) -> Sender<T> {
        let sender = self.sender.take().expect("OwnedPermit already consumed");
        sender.shared.release_reserved(1);
        sender
    }
}

impl<T> Drop for OwnedPermit<T> {
    fn drop(&mut self) {
        if let Some(sender) = &self.sender {
            sender.shared.release_reserved(1);
        }
    }
}

/// Hands out the `n` slots reserved by [`Sender::reserve_many`], one
/// [`Permit`] at a time. Dropping this iterator before exhausting it
/// releases whatever reservations were never handed out.
pub struct PermitIterator<'a, T> {
    sender: &'a Sender<T>,
    remaining: usize,
}

impl<'a, T> Iterator for PermitIterator<'a, T> {
    type Item = Permit<'a, T>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        Some(Permit {
            sender: self.sender,
            used: Cell::new(false),
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<T> ExactSizeIterator for PermitIterator<'_, T> {}

impl<T> Drop for PermitIterator<'_, T> {
    fn drop(&mut self) {
        if self.remaining > 0 {
            self.sender.shared.release_reserved(self.remaining);
        }
    }
}

/// Why [`Sender::try_reserve`]/[`Sender::try_reserve_owned`] failed to
/// reserve a slot without waiting.
pub enum TrySendError<T> {
    /// No free capacity right now -- retry later, or use
    /// [`Sender::reserve`]/[`Sender::reserve_many`] to wait for room.
    Full(T),
    /// The receiver has already been dropped, so no capacity will ever
    /// free up again.
    Closed(T),
}

impl<T> TrySendError<T> {
    /// The value (or, for the owning variants, the `Sender`) that
    /// couldn't be used to reserve a slot.
    pub fn into_inner(self) -> T {
        match self {
            TrySendError::Full(t) | TrySendError::Closed(t) => t,
        }
    }

    pub fn is_full(&self) -> bool {
        matches!(self, TrySendError::Full(_))
    }

    pub fn is_closed(&self) -> bool {
        matches!(self, TrySendError::Closed(_))
    }
}

impl<T> fmt::Debug for TrySendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TrySendError::Full(_) => f.write_str("TrySendError::Full(..)"),
            TrySendError::Closed(_) => f.write_str("TrySendError::Closed(..)"),
        }
    }
}

impl<T> fmt::Display for TrySendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TrySendError::Full(_) => write!(f, "no available capacity"),
            TrySendError::Closed(_) => write!(f, "channel closed"),
        }
    }
}

impl<T> std::error::Error for TrySendError<T> {}

/// Why [`Sender::send_timeout`] failed to deliver `value` in time.
pub enum SendTimeoutError<T> {
    /// `timeout` elapsed before capacity freed up.
    Timeout(T),
    /// The receiver has already been dropped.
    Closed(T),
}

impl<T> SendTimeoutError<T> {
    /// The value that couldn't be sent.
    pub fn into_inner(self) -> T {
        match self {
            SendTimeoutError::Timeout(t) | SendTimeoutError::Closed(t) => t,
        }
    }

    pub fn is_timeout(&self) -> bool {
        matches!(self, SendTimeoutError::Timeout(_))
    }

    pub fn is_closed(&self) -> bool {
        matches!(self, SendTimeoutError::Closed(_))
    }
}

impl<T> fmt::Debug for SendTimeoutError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SendTimeoutError::Timeout(_) => f.write_str("SendTimeoutError::Timeout(..)"),
            SendTimeoutError::Closed(_) => f.write_str("SendTimeoutError::Closed(..)"),
        }
    }
}

impl<T> fmt::Display for SendTimeoutError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SendTimeoutError::Timeout(_) => write!(f, "timed out waiting for channel capacity"),
            SendTimeoutError::Closed(_) => write!(f, "channel closed"),
        }
    }
}

impl<T> std::error::Error for SendTimeoutError<T> {}

impl<T> Receiver<T> {
    /// Takes a queued value without waiting, as tokio's: `Empty` while
    /// senders remain, `Disconnected` once they are all gone and the queue
    /// is drained.
    pub fn try_recv(&mut self) -> Result<T, TryRecvError> {
        let mut guard = self.shared.inner.lock().unwrap();
        if let Some(v) = guard.queue.pop_front() {
            let waker = guard.pop_send_waiter();
            drop(guard);
            if let Some(waker) = waker {
                waker.wake();
            }
            return Ok(v);
        }
        if guard.senders_alive == 0 {
            return Err(TryRecvError::Disconnected);
        }
        Err(TryRecvError::Empty)
    }

    /// Blocking form of [`recv`](Self::recv), for synchronous code -- a
    /// plain OS thread bridging blocking I/O into async tasks. Parks the
    /// calling thread until it completes; no runtime is needed.
    ///
    /// Never call this from an async task: it parks the worker thread
    /// running it, stalling every other task queued there.
    pub fn blocking_recv(&mut self) -> Option<T> {
        crate::runtime::block_on_inner(self.recv())
    }

    pub async fn recv(&mut self) -> Option<T> {
        std::future::poll_fn(|cx| self.poll_recv(cx)).await
    }

    /// Poll form of [`recv`](Self::recv), for a hand-written `Future` or
    /// `Stream`: `Ready(None)` once every sender is gone and the queue is
    /// drained.
    pub fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Option<T>> {
        if crate::coop::poll_proceed(cx).is_pending() {
            return Poll::Pending;
        }
        let mut guard = self.shared.inner.lock().unwrap();
        if let Some(v) = guard.queue.pop_front() {
            let waker = guard.pop_send_waiter();
            drop(guard);
            if let Some(waker) = waker {
                waker.wake();
            }
            return Poll::Ready(Some(v));
        }
        if guard.senders_alive == 0 {
            return Poll::Ready(None);
        }
        let stale = guard.recv_waker.replace(cx.waker().clone());
        drop(guard);
        drop(stale); // after unlocking: a waker may own a `Sender`
        Poll::Pending
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        let mut guard = self.shared.inner.lock().unwrap();
        guard.receiver_alive = false;
        let waiters = std::mem::take(&mut guard.send_waiters);
        // Undelivered values are dropped with the receiver, as tokio does, so
        // anything they own (a one-shot responder, say) closes too. Dropped
        // after unlocking: a value's own `Drop` may touch this channel.
        let undelivered = std::mem::take(&mut guard.queue);
        drop(guard);
        drop(undelivered);
        for (_, waker) in waiters {
            waker.wake();
        }
    }
}

struct UnboundedInner<T> {
    queue: VecDeque<T>,
    recv_waker: Option<Waker>,
    senders_alive: usize,
    receiver_alive: bool,
}

struct UnboundedShared<T> {
    inner: Mutex<UnboundedInner<T>>,
}

pub struct UnboundedSender<T> {
    shared: Arc<UnboundedShared<T>>,
}

pub struct UnboundedReceiver<T> {
    shared: Arc<UnboundedShared<T>>,
}

impl<T> fmt::Debug for UnboundedSender<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UnboundedSender").finish_non_exhaustive()
    }
}

impl<T> fmt::Debug for UnboundedReceiver<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UnboundedReceiver").finish_non_exhaustive()
    }
}

pub fn unbounded_channel<T>() -> (UnboundedSender<T>, UnboundedReceiver<T>) {
    let shared = Arc::new(UnboundedShared {
        inner: Mutex::new(UnboundedInner {
            queue: VecDeque::new(),
            recv_waker: None,
            senders_alive: 1,
            receiver_alive: true,
        }),
    });
    (
        UnboundedSender {
            shared: shared.clone(),
        },
        UnboundedReceiver { shared },
    )
}

impl<T> UnboundedSender<T> {
    /// Pushes `value` onto the queue and returns immediately -- there's
    /// no capacity to wait on, so unlike the bounded [`Sender::send`]
    /// this is a plain synchronous method, not `async fn`.
    pub fn send(&self, value: T) -> Result<(), SendError<T>> {
        let mut guard = self.shared.inner.lock().unwrap();
        if !guard.receiver_alive {
            drop(guard);
            return Err(SendError(value));
        }
        guard.queue.push_back(value);
        let waker = guard.recv_waker.take();
        drop(guard);
        if let Some(waker) = waker {
            waker.wake();
        }
        Ok(())
    }

    /// A weak handle that doesn't keep the channel's sender count alive
    /// by itself -- see [`Sender::downgrade`]/[`WeakSender`] for the
    /// full reasoning, identical here.
    pub fn downgrade(&self) -> WeakUnboundedSender<T> {
        WeakUnboundedSender {
            shared: Arc::downgrade(&self.shared),
        }
    }
}

impl<T> Clone for UnboundedSender<T> {
    fn clone(&self) -> Self {
        self.shared.inner.lock().unwrap().senders_alive += 1;
        UnboundedSender {
            shared: self.shared.clone(),
        }
    }
}

impl<T> Drop for UnboundedSender<T> {
    fn drop(&mut self) {
        let mut guard = self.shared.inner.lock().unwrap();
        guard.senders_alive -= 1;
        if guard.senders_alive == 0 {
            // That was the last sender: wake the receiver so it can see
            // the channel is now closed instead of waiting forever.
            let waker = guard.recv_waker.take();
            drop(guard);
            if let Some(waker) = waker {
                waker.wake();
            }
        }
    }
}

/// The [`UnboundedSender`] counterpart of [`WeakSender`] -- see that
/// type's own docs for the full reasoning, identical here.
pub struct WeakUnboundedSender<T> {
    shared: Weak<UnboundedShared<T>>,
}

impl<T> WeakUnboundedSender<T> {
    /// See [`WeakSender::upgrade`] -- identical semantics here.
    pub fn upgrade(&self) -> Option<UnboundedSender<T>> {
        let shared = self.shared.upgrade()?;
        let mut guard = shared.inner.lock().unwrap();
        if guard.senders_alive == 0 {
            return None;
        }
        guard.senders_alive += 1;
        drop(guard);
        Some(UnboundedSender { shared })
    }
}

impl<T> Clone for WeakUnboundedSender<T> {
    fn clone(&self) -> Self {
        WeakUnboundedSender {
            shared: self.shared.clone(),
        }
    }
}

impl<T> fmt::Debug for WeakUnboundedSender<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WeakUnboundedSender").finish()
    }
}

impl<T> UnboundedReceiver<T> {
    /// As [`Receiver::try_recv`].
    pub fn try_recv(&mut self) -> Result<T, TryRecvError> {
        let mut guard = self.shared.inner.lock().unwrap();
        if let Some(v) = guard.queue.pop_front() {
            return Ok(v);
        }
        if guard.senders_alive == 0 {
            return Err(TryRecvError::Disconnected);
        }
        Err(TryRecvError::Empty)
    }

    /// Blocking form of [`recv`](Self::recv), for synchronous code -- a
    /// plain OS thread bridging blocking I/O into async tasks. Parks the
    /// calling thread until it completes; no runtime is needed.
    ///
    /// Never call this from an async task: it parks the worker thread
    /// running it, stalling every other task queued there.
    pub fn blocking_recv(&mut self) -> Option<T> {
        crate::runtime::block_on_inner(self.recv())
    }

    pub async fn recv(&mut self) -> Option<T> {
        std::future::poll_fn(|cx| self.poll_recv(cx)).await
    }

    /// As [`Receiver::poll_recv`].
    pub fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Option<T>> {
        if crate::coop::poll_proceed(cx).is_pending() {
            return Poll::Pending;
        }
        let mut guard = self.shared.inner.lock().unwrap();
        if let Some(v) = guard.queue.pop_front() {
            return Poll::Ready(Some(v));
        }
        if guard.senders_alive == 0 {
            return Poll::Ready(None);
        }
        let stale = guard.recv_waker.replace(cx.waker().clone());
        drop(guard);
        drop(stale); // after unlocking: a waker may own a `Sender`
        Poll::Pending
    }
}

impl<T> Drop for UnboundedReceiver<T> {
    fn drop(&mut self) {
        // No send_waiters to wake here, unlike the bounded Receiver's
        // Drop -- an unbounded Sender never waits to send, so there's
        // never anyone parked on this channel besides (at most) the
        // receiver itself, which is what's dropping right now.
        let mut guard = self.shared.inner.lock().unwrap();
        guard.receiver_alive = false;
        // As the bounded Receiver's Drop: undelivered values go with it,
        // dropped after unlocking.
        let undelivered = std::mem::take(&mut guard.queue);
        drop(guard);
        drop(undelivered);
    }
}

/// tokio's `mpsc::error` path for this module's error types.
pub mod error {
    pub use super::{SendError, SendTimeoutError, TryRecvError, TrySendError};
}

/// Why [`Receiver::try_recv`] returned no value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TryRecvError {
    /// Nothing is queued, but a sender remains.
    Empty,
    /// Nothing is queued, and every sender is gone.
    Disconnected,
}

impl fmt::Display for TryRecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TryRecvError::Empty => write!(f, "receiving on an empty channel"),
            TryRecvError::Disconnected => write!(f, "receiving on a closed channel"),
        }
    }
}

impl std::error::Error for TryRecvError {}

/// The receiver was dropped, so the value in this error could not be
/// delivered.
pub struct SendError<T>(pub T);

impl<T> fmt::Debug for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SendError(..)")
    }
}

impl<T> fmt::Display for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sending on a closed channel")
    }
}

impl<T> std::error::Error for SendError<T> {}
