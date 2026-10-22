//! The runtime behind [`stream!`](crate::stream!) and
//! [`try_stream!`](crate::try_stream!).
//!
//! The macro body is an async block that owns a [`Yielder`]. Each `yield`
//! parks its value in a slot shared with the stream and suspends once;
//! [`AsyncStream::poll_next`] hands the parked value out. No thread-locals and
//! no `unsafe`: the slot is an `Arc<Mutex<Option<T>>>`.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll};

use crate::Stream;

/// The handle a macro body yields through.
#[derive(Debug)]
pub struct Yielder<T> {
    slot: Arc<Mutex<Option<T>>>,
}

impl<T> Clone for Yielder<T> {
    fn clone(&self) -> Self {
        Self {
            slot: Arc::clone(&self.slot),
        }
    }
}

impl<T> Yielder<T> {
    /// Park `value` for the stream and suspend until it has been taken.
    pub fn send(&self, value: T) -> Send<'_, T> {
        Send {
            yielder: self,
            value: Some(value),
        }
    }
}

/// The future returned by [`Yielder::send`].
#[derive(Debug)]
#[must_use = "a yield does nothing unless awaited"]
pub struct Send<'a, T> {
    yielder: &'a Yielder<T>,
    value: Option<T>,
}

impl<T> Unpin for Send<'_, T> {}

impl<T> Future for Send<'_, T> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        match this.value.take() {
            // First poll: park the value and suspend. `AsyncStream::poll_next`
            // returns it at once, so no wake-up is needed.
            Some(value) => {
                *lock(&this.yielder.slot) = Some(value);
                Poll::Pending
            }
            None => Poll::Ready(()),
        }
    }
}

/// A stream driven by an async block that yields through a [`Yielder`].
#[must_use = "streams do nothing unless polled"]
pub struct AsyncStream<T, Fut> {
    slot: Arc<Mutex<Option<T>>>,
    body: Option<Pin<Box<Fut>>>,
}

impl<T, Fut> Unpin for AsyncStream<T, Fut> {}

impl<T, Fut: Future<Output = ()>> Stream for AsyncStream<T, Fut> {
    type Item = T;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
        let this = self.get_mut();
        let Some(body) = this.body.as_mut() else {
            return Poll::Ready(None);
        };
        let finished = body.as_mut().poll(cx).is_ready();
        if finished {
            this.body = None;
        }
        match lock(&this.slot).take() {
            Some(item) => Poll::Ready(Some(item)),
            None if finished => Poll::Ready(None),
            None => Poll::Pending,
        }
    }
}

/// Build the stream for `stream!`.
pub fn stream<T, Fut, F>(body: F) -> AsyncStream<T, Fut>
where
    F: FnOnce(Yielder<T>) -> Fut,
    Fut: Future<Output = ()>,
{
    let slot = Arc::new(Mutex::new(None));
    let yielder = Yielder {
        slot: Arc::clone(&slot),
    };
    AsyncStream {
        slot,
        body: Some(Box::pin(body(yielder))),
    }
}

/// Build the stream for `try_stream!`: an `Err` from the body is yielded as
/// the last item.
pub fn try_stream<T, E, Fut, F>(body: F) -> AsyncStream<Result<T, E>, impl Future<Output = ()>>
where
    F: FnOnce(Yielder<Result<T, E>>) -> Fut,
    Fut: Future<Output = Result<(), E>>,
{
    stream(move |yielder: Yielder<Result<T, E>>| {
        let on_error = yielder.clone();
        let body = body(yielder);
        async move {
            if let Err(error) = body.await {
                on_error.send(Err(error)).await;
            }
        }
    })
}

/// Lock the slot, recovering from poisoning: the slot holds plain data, so a
/// panic elsewhere cannot leave it inconsistent.
fn lock<T>(slot: &Mutex<T>) -> MutexGuard<'_, T> {
    slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
