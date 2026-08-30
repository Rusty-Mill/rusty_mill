#![cfg_attr(not(test), no_std)]
#![deny(missing_docs)]

//! # `rusty_sync`
//!
//! A `#![no_std]` + `alloc` sovereign atomic spinlock, spinlock-protected
//! MPMC channel, and ring buffer implementation for the **Rusty Mill**
//! ecosystem.
//!
//! The channel and ring buffer here are coarse-grained (a single
//! [`SpinLock`] guards the whole queue), not a true lock-free CAS-based
//! design — the same tradeoff `rusty_tokio` documents for its own
//! work-stealing deque: a hand-rolled lock-free MPMC queue is real unsafe
//! concurrent code this crate has no `loom`-based verification set up to
//! trust yet. Correct and simple now; a genuinely lock-free version is a
//! deliberate, separately-verified follow-up, not a silent claim.

extern crate alloc;

use alloc::collections::VecDeque;
use alloc::sync::Arc;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicUsize, Ordering};

/// Atomic spinlock mutex.
pub struct SpinLock<T> {
    lock: core::sync::atomic::AtomicBool,
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Send for SpinLock<T> {}
unsafe impl<T: Send> Sync for SpinLock<T> {}

impl<T> SpinLock<T> {
    /// Creates a new SpinLock wrapping data.
    pub const fn new(data: T) -> Self {
        Self {
            lock: core::sync::atomic::AtomicBool::new(false),
            data: UnsafeCell::new(data),
        }
    }

    /// Acquires spinlock and returns a Guard borrowing inner value.
    pub fn lock(&self) -> SpinLockGuard<'_, T> {
        while self.lock.swap(true, core::sync::atomic::Ordering::Acquire) {
            core::hint::spin_loop();
        }
        SpinLockGuard { lock: self }
    }
}

/// RAII Guard for SpinLock.
pub struct SpinLockGuard<'a, T> {
    lock: &'a SpinLock<T>,
}

impl<'a, T> core::ops::Deref for SpinLockGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<'a, T> core::ops::DerefMut for SpinLockGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<'a, T> Drop for SpinLockGuard<'a, T> {
    fn drop(&mut self) {
        self.lock
            .lock
            .store(false, core::sync::atomic::Ordering::Release);
    }
}

/// A bounded FIFO ring buffer, protected by a [`SpinLock`].
///
/// Not lock-free (see the module-level doc comment) — a straightforward,
/// correct bounded queue that [`channel`] builds on.
pub struct RingBuffer<T> {
    inner: SpinLock<VecDeque<T>>,
    capacity: usize,
}

impl<T> RingBuffer<T> {
    /// Creates an empty ring buffer that holds at most `capacity` elements.
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: SpinLock::new(VecDeque::with_capacity(capacity)),
            capacity,
        }
    }

    /// Pushes `value` onto the back of the buffer. Returns `Err(value)`,
    /// giving the value back, if the buffer is already at [`Self::capacity`].
    pub fn push(&self, value: T) -> Result<(), T> {
        let mut guard = self.inner.lock();
        if guard.len() >= self.capacity {
            return Err(value);
        }
        guard.push_back(value);
        Ok(())
    }

    /// Pops the oldest value off the front of the buffer, or `None` if empty.
    pub fn pop(&self) -> Option<T> {
        self.inner.lock().pop_front()
    }

    /// The number of elements currently buffered.
    pub fn len(&self) -> usize {
        self.inner.lock().len()
    }

    /// Whether the buffer currently holds no elements.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The maximum number of elements this buffer can hold.
    pub const fn capacity(&self) -> usize {
        self.capacity
    }
}

/// Error returned by [`Sender::try_send`].
#[derive(Debug, PartialEq, Eq)]
pub enum SendError<T> {
    /// The channel's ring buffer is at capacity; the value is handed back.
    Full(T),
    /// Every [`Receiver`] for this channel has been dropped; the value is
    /// handed back.
    Disconnected(T),
}

/// Error returned by [`Receiver::try_recv`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecvError {
    /// The channel is currently empty, but at least one [`Sender`] remains.
    Empty,
    /// The channel is empty and every [`Sender`] has been dropped — no
    /// further values can ever arrive.
    Disconnected,
}

struct ChannelInner<T> {
    queue: RingBuffer<T>,
    senders: AtomicUsize,
    receivers: AtomicUsize,
}

/// The sending half of an MPMC channel created by [`channel`]. Cloneable —
/// every clone increments the shared sender count so [`Receiver`]s can
/// detect when the last one is dropped.
pub struct Sender<T> {
    inner: Arc<ChannelInner<T>>,
}

/// The receiving half of an MPMC channel created by [`channel`]. Cloneable —
/// every clone increments the shared receiver count so [`Sender`]s can
/// detect when the last one is dropped.
pub struct Receiver<T> {
    inner: Arc<ChannelInner<T>>,
}

/// Creates a bounded MPMC channel with room for `capacity` in-flight values.
pub fn channel<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    let inner = Arc::new(ChannelInner {
        queue: RingBuffer::new(capacity),
        senders: AtomicUsize::new(1),
        receivers: AtomicUsize::new(1),
    });
    (
        Sender {
            inner: inner.clone(),
        },
        Receiver { inner },
    )
}

impl<T> Sender<T> {
    /// Attempts to send `value` without blocking. Fails with
    /// [`SendError::Full`] if the channel's buffer is at capacity, or
    /// [`SendError::Disconnected`] if every `Receiver` has been dropped.
    pub fn try_send(&self, value: T) -> Result<(), SendError<T>> {
        if self.inner.receivers.load(Ordering::Acquire) == 0 {
            return Err(SendError::Disconnected(value));
        }
        self.inner.queue.push(value).map_err(SendError::Full)
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        self.inner.senders.fetch_add(1, Ordering::AcqRel);
        Sender {
            inner: self.inner.clone(),
        }
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        self.inner.senders.fetch_sub(1, Ordering::AcqRel);
    }
}

impl<T> Receiver<T> {
    /// Attempts to receive a value without blocking. Fails with
    /// [`RecvError::Empty`] if nothing is queued yet, or
    /// [`RecvError::Disconnected`] if the channel is empty and every
    /// `Sender` has been dropped.
    pub fn try_recv(&self) -> Result<T, RecvError> {
        if let Some(value) = self.inner.queue.pop() {
            return Ok(value);
        }
        if self.inner.senders.load(Ordering::Acquire) == 0 {
            Err(RecvError::Disconnected)
        } else {
            Err(RecvError::Empty)
        }
    }
}

impl<T> Clone for Receiver<T> {
    fn clone(&self) -> Self {
        self.inner.receivers.fetch_add(1, Ordering::AcqRel);
        Receiver {
            inner: self.inner.clone(),
        }
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        self.inner.receivers.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spinlock_mutual_exclusion() {
        let lock = SpinLock::new(42);
        {
            let mut val = lock.lock();
            *val += 1;
        }
        assert_eq!(*lock.lock(), 43);
    }

    #[test]
    fn spinlock_holds_up_under_real_concurrent_threads() {
        let lock = Arc::new(SpinLock::new(0i64));
        let mut handles = alloc::vec::Vec::new();
        for _ in 0..8 {
            let lock = lock.clone();
            handles.push(std::thread::spawn(move || {
                for _ in 0..1000 {
                    *lock.lock() += 1;
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(*lock.lock(), 8000);
    }

    #[test]
    fn ring_buffer_push_pop_respects_fifo_order_and_capacity() {
        let rb = RingBuffer::new(2);
        assert!(rb.is_empty());
        rb.push(1).unwrap();
        rb.push(2).unwrap();
        assert_eq!(rb.push(3), Err(3));
        assert_eq!(rb.len(), 2);
        assert_eq!(rb.pop(), Some(1));
        assert_eq!(rb.pop(), Some(2));
        assert_eq!(rb.pop(), None);
    }

    #[test]
    fn channel_send_recv_round_trips() {
        let (tx, rx) = channel(4);
        tx.try_send(1).unwrap();
        tx.try_send(2).unwrap();
        assert_eq!(rx.try_recv(), Ok(1));
        assert_eq!(rx.try_recv(), Ok(2));
        assert_eq!(rx.try_recv(), Err(RecvError::Empty));
    }

    #[test]
    fn channel_reports_full_at_capacity() {
        let (tx, _rx) = channel(1);
        tx.try_send(1).unwrap();
        assert_eq!(tx.try_send(2), Err(SendError::Full(2)));
    }

    #[test]
    fn channel_send_after_every_receiver_dropped_is_disconnected() {
        let (tx, rx) = channel::<i32>(1);
        drop(rx);
        assert_eq!(tx.try_send(1), Err(SendError::Disconnected(1)));
    }

    #[test]
    fn channel_recv_after_every_sender_dropped_drains_then_disconnects() {
        let (tx, rx) = channel(2);
        tx.try_send(1).unwrap();
        drop(tx);
        assert_eq!(rx.try_recv(), Ok(1));
        assert_eq!(rx.try_recv(), Err(RecvError::Disconnected));
    }

    #[test]
    fn channel_holds_up_across_real_producer_and_consumer_threads() {
        let (tx, rx) = channel::<i32>(16);
        let producer = std::thread::spawn(move || {
            for i in 0..100 {
                loop {
                    if tx.try_send(i).is_ok() {
                        break;
                    }
                    std::thread::yield_now();
                }
            }
        });
        let mut received = alloc::vec::Vec::new();
        while received.len() < 100 {
            match rx.try_recv() {
                Ok(v) => received.push(v),
                Err(_) => std::thread::yield_now(),
            }
        }
        producer.join().unwrap();
        assert_eq!(received, (0..100).collect::<alloc::vec::Vec<_>>());
    }
}
