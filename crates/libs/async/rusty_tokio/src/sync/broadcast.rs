//! A multi-producer, multi-consumer channel where every receiver gets
//! its own copy of every message -- a real, distinct set of semantics
//! from [`super::mpsc`]'s (where a sent message goes to exactly one
//! receiver), not just "mpsc with multiple receivers."
//!
//! A fixed-capacity ring buffer backs the channel; each [`Receiver`]
//! tracks its own read position (a sequence number) into it rather than
//! actually removing entries, so `send` never waits for room -- once the
//! buffer is full, the oldest message is simply overwritten. A receiver
//! whose read position has fallen behind the oldest message still in
//! the buffer has been lagged: its next `recv()` reports exactly how
//! many messages it missed (`RecvError::Lagged(n)`) and jumps its
//! position forward to the oldest still-available message, rather than
//! reporting `Lagged` again on every subsequent call.
//!
//! The buffer, every receiver's parked waker, and the alive-count
//! bookkeeping all live under a single lock -- the same shape
//! `sync::mpsc`'s own module docs explain the reasoning for: checking
//! "is there something new for me" and registering a waker under
//! separate locks leaves a window where a `send` in between finds no
//! one (yet) registered to wake, a real lost wakeup rather than a
//! hypothetical one.
//!
//! Each pending `recv()` keeps one waker slot under a key: a re-poll
//! replaces its waker in place, and dropping the future (as a `select!`
//! loop does on every turn it loses) removes the slot, so `recv` is
//! cancel safe and leaves nothing queued, as tokio's is. Displaced and
//! removed wakers are dropped after the lock is released, since a
//! waker's destructor may re-enter the channel.

use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex, Weak};
use std::task::{Context, Poll, Waker};

struct Inner<T> {
    buffer: VecDeque<T>,
    capacity: usize,
    /// The sequence number the *next* sent message will get. The oldest
    /// message still in `buffer` (if any) has sequence number
    /// `next_seq - buffer.len()`.
    next_seq: u64,
    /// Every receiver currently parked in `recv`, waiting for a message
    /// past whatever it's already seen -- woken in full on every `send`
    /// (and once every sender drops), since a new message means every
    /// currently-parked receiver, by definition already caught up to the
    /// old `next_seq`, now has something new to read.
    ///
    /// Each keyed by the [`Waiter`] that owns it, which removes it when
    /// dropped (see the module docs).
    waiters: Vec<(u64, Waker)>,
    /// The key the next [`Waiter`] to park gets.
    next_key: u64,
    senders_alive: usize,
    receivers_alive: usize,
}

struct Shared<T> {
    inner: Mutex<Inner<T>>,
}

pub struct Sender<T> {
    shared: Arc<Shared<T>>,
}

pub struct Receiver<T> {
    shared: Arc<Shared<T>>,
    next_to_read: u64,
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

/// Creates a broadcast channel with room for `capacity` unread messages
/// before a lagging receiver starts missing them.
///
/// # Panics
/// Panics if `capacity` is zero.
pub fn channel<T: Clone>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    assert!(capacity > 0, "broadcast channel capacity must be positive");
    let shared = Arc::new(Shared {
        inner: Mutex::new(Inner {
            buffer: VecDeque::with_capacity(capacity),
            capacity,
            next_seq: 0,
            waiters: Vec::new(),
            next_key: 0,
            senders_alive: 1,
            receivers_alive: 1,
        }),
    });
    let sender = Sender {
        shared: shared.clone(),
    };
    let receiver = Receiver {
        shared,
        next_to_read: 0,
    };
    (sender, receiver)
}

impl<T: Clone> Sender<T> {
    /// Sends `value` to every current receiver, returning how many
    /// receivers were subscribed at the time -- not a guarantee any of
    /// them will actually observe it before falling too far behind and
    /// getting `RecvError::Lagged` instead. Never waits: once the ring
    /// buffer is full, the oldest message is simply overwritten.
    ///
    /// # Errors
    /// Fails if every receiver has already been dropped.
    pub fn send(&self, value: T) -> Result<usize, SendError<T>> {
        let mut guard = self.shared.inner.lock().unwrap();
        if guard.receivers_alive == 0 {
            drop(guard);
            return Err(SendError(value));
        }
        if guard.buffer.len() == guard.capacity {
            guard.buffer.pop_front();
        }
        guard.buffer.push_back(value);
        guard.next_seq += 1;
        let receivers = guard.receivers_alive;
        let waiters = std::mem::take(&mut guard.waiters);
        drop(guard);
        for (_, waker) in waiters {
            waker.wake();
        }
        Ok(receivers)
    }

    /// Creates a new receiver that will see every message sent *after*
    /// this call -- not anything already in the buffer, matching what a
    /// receiver returned from [`channel`] itself sees.
    pub fn subscribe(&self) -> Receiver<T> {
        let mut guard = self.shared.inner.lock().unwrap();
        guard.receivers_alive += 1;
        let next_to_read = guard.next_seq;
        drop(guard);
        Receiver {
            shared: self.shared.clone(),
            next_to_read,
        }
    }

    pub fn receiver_count(&self) -> usize {
        self.shared.inner.lock().unwrap().receivers_alive
    }

    /// A weak handle that doesn't keep the channel's sender count alive
    /// by itself -- see `sync::mpsc`'s own `WeakSender` docs for the
    /// full reasoning, identical here.
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
            // That was the last sender: wake every parked receiver so
            // they can see the channel is now closed instead of waiting
            // forever for a message that will never come.
            let waiters = std::mem::take(&mut guard.waiters);
            drop(guard);
            for (_, waker) in waiters {
                waker.wake();
            }
        }
    }
}

/// A handle to a broadcast channel's sender that doesn't keep it (or its
/// sender count) alive by itself -- obtained via [`Sender::downgrade`].
/// See `sync::mpsc::WeakSender`'s own docs for the full reasoning,
/// identical here.
pub struct WeakSender<T> {
    shared: Weak<Shared<T>>,
}

impl<T> WeakSender<T> {
    /// Hands back a real, usable `Sender` -- but only if at least one
    /// other `Sender` already exists. `None` once every real `Sender`
    /// has been dropped, even if receivers are still alive.
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

impl<T: Clone> Receiver<T> {
    /// Waits for the next message. Reports `RecvError::Lagged(n)` (and
    /// jumps this receiver's position forward to the oldest message
    /// still buffered) if `n` messages were missed since the last call,
    /// or `RecvError::Closed` once every sender has dropped and there's
    /// nothing left unread.
    ///
    /// Cancel safe, as tokio's is: dropping the future before it resolves
    /// loses no message and leaves no waker behind.
    pub async fn recv(&mut self) -> Result<T, RecvError> {
        let Receiver {
            shared,
            next_to_read,
        } = self;
        let mut waiter = Waiter { shared, key: None };
        std::future::poll_fn(|cx| waiter.poll_recv(next_to_read, cx)).await
    }

    /// Like [`recv`](Self::recv), but returns immediately instead of
    /// waiting if there's nothing new yet.
    pub fn try_recv(&mut self) -> Result<T, TryRecvError> {
        let guard = self.shared.inner.lock().unwrap();
        let base_seq = guard.next_seq - guard.buffer.len() as u64;
        if self.next_to_read < base_seq {
            let lagged = base_seq - self.next_to_read;
            self.next_to_read = base_seq;
            return Err(TryRecvError::Lagged(lagged));
        }
        if self.next_to_read < guard.next_seq {
            let idx = (self.next_to_read - base_seq) as usize;
            let value = guard.buffer[idx].clone();
            self.next_to_read += 1;
            return Ok(value);
        }
        if guard.senders_alive == 0 {
            return Err(TryRecvError::Closed);
        }
        Err(TryRecvError::Empty)
    }
}

/// The waker slot of one pending [`Receiver::recv`], removed when the
/// `recv` future resolves or is dropped.
struct Waiter<'a, T> {
    shared: &'a Shared<T>,
    /// This waiter's key in [`Inner::waiters`], once it has parked.
    key: Option<u64>,
}

impl<T: Clone> Waiter<'_, T> {
    fn poll_recv(
        &mut self,
        next_to_read: &mut u64,
        cx: &mut Context<'_>,
    ) -> Poll<Result<T, RecvError>> {
        // Cloned before locking: cloning a waker runs arbitrary code.
        let waker = cx.waker().clone();
        let mut guard = self.shared.inner.lock().unwrap();
        let (poll, stale) = match next_message(&mut guard, next_to_read) {
            // Whatever made a parked waiter ready (a send, the last sender
            // dropping) already drained its slot; this only guards that.
            Some(result) => (Poll::Ready(result), self.unregister(&mut guard)),
            None => (Poll::Pending, self.park(&mut guard, waker)),
        };
        drop(guard);
        // Dropped outside the lock: a waker's destructor may re-enter.
        drop(stale);
        poll
    }

    /// Stores `waker` in this waiter's slot, taking a slot on first park.
    /// Returns the waker to drop once the lock is released: the one it
    /// displaced, or `waker` itself when the stored one already wakes
    /// the same task.
    fn park(&mut self, inner: &mut Inner<T>, waker: Waker) -> Option<Waker> {
        let slot = self
            .key
            .and_then(|key| inner.waiters.iter_mut().find(|(k, _)| *k == key));
        match slot {
            Some((_, stored)) if stored.will_wake(&waker) => Some(waker),
            Some((_, stored)) => Some(std::mem::replace(stored, waker)),
            None => {
                let key = inner.next_key;
                inner.next_key = key.wrapping_add(1);
                inner.waiters.push((key, waker));
                self.key = Some(key);
                None
            }
        }
    }
}

impl<T> Waiter<'_, T> {
    /// Removes this waiter's slot, if a send has not already drained it,
    /// returning its waker to drop once the lock is released.
    fn unregister(&mut self, inner: &mut Inner<T>) -> Option<Waker> {
        let key = self.key.take()?;
        let index = inner.waiters.iter().position(|(k, _)| *k == key)?;
        Some(inner.waiters.swap_remove(index).1)
    }
}

impl<T> Drop for Waiter<'_, T> {
    fn drop(&mut self) {
        if self.key.is_none() {
            return;
        }
        // Never panic in drop: a poisoned lock has nothing left to clean.
        let Ok(mut guard) = self.shared.inner.lock() else {
            return;
        };
        let stale = self.unregister(&mut guard);
        drop(guard);
        drop(stale);
    }
}

/// The message, lag or close a receiver at `next_to_read` sees now, or
/// `None` if it must wait. Advances `next_to_read` past what it returns.
fn next_message<T: Clone>(
    inner: &mut Inner<T>,
    next_to_read: &mut u64,
) -> Option<Result<T, RecvError>> {
    let base_seq = inner.next_seq - inner.buffer.len() as u64;
    if *next_to_read < base_seq {
        let lagged = base_seq - *next_to_read;
        *next_to_read = base_seq;
        return Some(Err(RecvError::Lagged(lagged)));
    }
    if *next_to_read < inner.next_seq {
        let idx = (*next_to_read - base_seq) as usize;
        let value = inner.buffer[idx].clone();
        *next_to_read += 1;
        return Some(Ok(value));
    }
    if inner.senders_alive == 0 {
        return Some(Err(RecvError::Closed));
    }
    None
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        // Dropping a receiver doesn't change whether any *other*
        // receiver has progress to make (there's no shared capacity
        // gated on receiver count the way mpsc's bounded `Sender::send`
        // is), so unlike mpsc's `Receiver::drop`, nothing needs waking
        // here.
        self.shared.inner.lock().unwrap().receivers_alive -= 1;
    }
}

/// Every receiver had already been dropped, so `value` could not be
/// delivered to anyone.
pub struct SendError<T>(pub T);

impl<T> fmt::Debug for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SendError(..)")
    }
}

impl<T> fmt::Display for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sending on a broadcast channel with no receivers left")
    }
}

impl<T> std::error::Error for SendError<T> {}

/// Why [`Receiver::recv`] didn't return a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecvError {
    /// Every sender has dropped and there's nothing left unread.
    Closed,
    /// This many messages were missed -- the receiver had fallen behind
    /// the oldest message still in the ring buffer. Its read position
    /// has already been advanced to that oldest message, so the next
    /// call resumes from there rather than reporting this again.
    Lagged(u64),
}

impl fmt::Display for RecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecvError::Closed => write!(f, "broadcast channel closed"),
            RecvError::Lagged(n) => write!(f, "receiver lagged behind by {n} messages"),
        }
    }
}

impl std::error::Error for RecvError {}

/// Why [`Receiver::try_recv`] didn't return a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TryRecvError {
    /// No message available right now, but the channel isn't closed.
    Empty,
    /// Every sender has dropped and there's nothing left unread.
    Closed,
    /// See [`RecvError::Lagged`].
    Lagged(u64),
}

impl fmt::Display for TryRecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TryRecvError::Empty => write!(f, "no message currently available"),
            TryRecvError::Closed => write!(f, "broadcast channel closed"),
            TryRecvError::Lagged(n) => write!(f, "receiver lagged behind by {n} messages"),
        }
    }
}

impl std::error::Error for TryRecvError {}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};
    use std::time::Duration;

    use super::*;

    /// Counts its wakes; its `Arc` strong count tells how many clones
    /// the channel stores.
    #[derive(Default)]
    struct Count(AtomicUsize);

    impl Wake for Count {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn counting() -> (Arc<Count>, Waker) {
        let count = Arc::new(Count::default());
        let waker = Waker::from(count.clone());
        (count, waker)
    }

    /// Clones of `count`'s waker held by anyone but the test, which
    /// holds the `Arc` and one `Waker`.
    fn stored(count: &Arc<Count>) -> usize {
        Arc::strong_count(count) - 2
    }

    fn poll<F: Future>(future: std::pin::Pin<&mut F>, waker: &Waker) -> Poll<F::Output> {
        future.poll(&mut Context::from_waker(waker))
    }

    /// The usual `select!` loop over an idle channel drops a pending
    /// `recv()` on every turn; tokio documents `recv` as cancel safe
    /// (#577).
    #[test]
    fn a_cancelled_recv_leaves_no_waker() {
        let (_tx, mut rx) = channel::<u8>(4);
        let (count, waker) = counting();
        for _ in 0..1000 {
            let mut recv = Box::pin(rx.recv());
            assert!(poll(recv.as_mut(), &waker).is_pending());
        }
        assert_eq!(stored(&count), 0);
    }

    #[test]
    fn a_repolled_recv_stores_one_waker() {
        let (_tx, mut rx) = channel::<u8>(4);
        let (count, waker) = counting();
        let mut recv = pin!(rx.recv());
        for _ in 0..1000 {
            assert!(poll(recv.as_mut(), &waker).is_pending());
        }
        assert_eq!(stored(&count), 1);
    }

    /// A re-poll with another waker replaces the old one: only the
    /// newest is woken, as tokio's `Recv` keeps a single waker.
    #[test]
    fn a_repolled_recv_wakes_only_its_newest_waker() {
        let (tx, mut rx) = channel::<u8>(4);
        let (old, old_waker) = counting();
        let (new, new_waker) = counting();
        let mut recv = pin!(rx.recv());
        assert!(poll(recv.as_mut(), &old_waker).is_pending());
        assert!(poll(recv.as_mut(), &new_waker).is_pending());
        assert_eq!(stored(&old), 0);
        tx.send(7).unwrap();
        assert_eq!(old.0.load(Ordering::Relaxed), 0);
        assert_eq!(new.0.load(Ordering::Relaxed), 1);
        assert_eq!(poll(recv.as_mut(), &new_waker), Poll::Ready(Ok(7)));
    }

    #[test]
    fn a_cancelled_recv_does_not_stop_another_receiver_waking() {
        let (tx, mut cancelled) = channel::<u8>(4);
        let mut live = tx.subscribe();
        let (count, waker) = counting();
        let mut waiting = pin!(live.recv());
        assert!(poll(waiting.as_mut(), &waker).is_pending());
        {
            let mut dropped = pin!(cancelled.recv());
            assert!(poll(dropped.as_mut(), Waker::noop()).is_pending());
        }
        tx.send(1).unwrap();
        assert_eq!(count.0.load(Ordering::Relaxed), 1);
        assert_eq!(poll(waiting.as_mut(), &waker), Poll::Ready(Ok(1)));
    }

    /// A waker that owns a `Sender`: dropping it runs `Sender::drop`, and
    /// waking it calls `receiver_count`, both of which take the channel
    /// lock, so it must be woken and dropped unlocked.
    struct OwnsSender(Sender<u8>);

    impl Wake for OwnsSender {
        fn wake(self: Arc<Self>) {
            let _ = self.0.receiver_count();
        }
    }

    fn bounded(test: impl FnOnce() + Send + 'static) {
        let (done, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            test();
            let _ = done.send(());
        });
        finished
            .recv_timeout(Duration::from_secs(5))
            .expect("deadlocked: a waker was dropped under the channel lock");
    }

    #[test]
    fn a_waker_owning_the_sender_is_dropped_unlocked_on_cancel() {
        bounded(|| {
            let (tx, mut rx) = channel::<u8>(4);
            let waker = Waker::from(Arc::new(OwnsSender(tx)));
            let mut recv = Box::pin(rx.recv());
            assert!(poll(recv.as_mut(), &waker).is_pending());
            drop(waker);
            drop(recv);
        });
    }

    #[test]
    fn a_waker_owning_the_sender_is_dropped_unlocked_on_repoll() {
        bounded(|| {
            let (tx, mut rx) = channel::<u8>(4);
            let waker = Waker::from(Arc::new(OwnsSender(tx)));
            let mut recv = Box::pin(rx.recv());
            assert!(poll(recv.as_mut(), &waker).is_pending());
            drop(waker);
            // The stored waker, and with it the last `Sender`, is displaced.
            assert_eq!(
                poll(recv.as_mut(), Waker::noop()),
                Poll::Pending,
                "the channel closes only once the old waker is dropped",
            );
            assert_eq!(
                poll(recv.as_mut(), Waker::noop()),
                Poll::Ready(Err(RecvError::Closed))
            );
        });
    }
}
