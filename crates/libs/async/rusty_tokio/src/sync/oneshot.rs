//! A single-value, single-use channel: exactly one [`Sender::send`],
//! exactly one `.await` on the [`Receiver`] to get it back (or find out
//! the sender was dropped without sending).
//!
//! A waker's clone or drop, and a value's drop, may run code that
//! re-enters the channel (a waker that owns the `Sender`, say), so none
//! happens with `state` locked: the receiver clones its waker before
//! locking, and a waker or value taken out of `state` is dropped after
//! unlocking (#578).

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

enum State<T> {
    Empty,
    Waiting(Waker),
    Value(T),
    Closed,
}

struct Shared<T> {
    state: Mutex<State<T>>,
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

pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let shared = Arc::new(Shared {
        state: Mutex::new(State::Empty),
    });
    (
        Sender {
            shared: shared.clone(),
        },
        Receiver { shared },
    )
}

impl<T> Sender<T> {
    /// Sends the value, or hands it back if the receiver was already
    /// dropped (nobody left to receive it).
    pub fn send(self, value: T) -> Result<(), T> {
        let mut guard = self.shared.state.lock().unwrap();
        if matches!(&*guard, State::Closed) {
            return Err(value);
        }
        let old = std::mem::replace(&mut *guard, State::Value(value));
        drop(guard);
        if let State::Waiting(waker) = old {
            waker.wake();
        }
        Ok(())
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        let mut guard = self.shared.state.lock().unwrap();
        if matches!(&*guard, State::Empty | State::Waiting(_)) {
            let old = std::mem::replace(&mut *guard, State::Closed);
            drop(guard);
            if let State::Waiting(waker) = old {
                waker.wake();
            }
        }
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        // The stored waker or unreceived value is dropped once unlocked:
        // the guard is a temporary of this statement.
        let old = std::mem::replace(&mut *self.shared.state.lock().unwrap(), State::Closed);
        drop(old);
    }
}

impl<T> Future for Receiver<T> {
    type Output = Result<T, RecvError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if crate::coop::poll_proceed(cx).is_pending() {
            return Poll::Pending;
        }
        // Cloned before locking: cloning a waker runs arbitrary code.
        let waker = cx.waker().clone();
        let mut guard = self.shared.state.lock().unwrap();
        let (poll, stale) = match std::mem::replace(&mut *guard, State::Closed) {
            State::Value(value) => (Poll::Ready(Ok(value)), Some(waker)),
            State::Closed => (Poll::Ready(Err(RecvError)), Some(waker)),
            State::Empty => {
                *guard = State::Waiting(waker);
                (Poll::Pending, None)
            }
            State::Waiting(stored) => {
                let (keep, stale) = if stored.will_wake(&waker) {
                    (stored, waker)
                } else {
                    (waker, stored)
                };
                *guard = State::Waiting(keep);
                (Poll::Pending, Some(stale))
            }
        };
        drop(guard);
        // Dropped outside the lock: a waker's destructor may re-enter.
        drop(stale);
        poll
    }
}

/// The sender was dropped without ever calling [`Sender::send`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecvError;

impl fmt::Display for RecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the sender was dropped without sending a value")
    }
}

impl std::error::Error for RecvError {}
