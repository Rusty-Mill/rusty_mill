//! Future helpers: [`BoxFuture`], [`join_all`], [`ready`].

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

pub use std::future::{ready, Ready};

/// A boxed, type-erased, `Send` future.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Run every future concurrently; resolve to their outputs in input order.
pub fn join_all<I>(futures: I) -> JoinAll<I::Item>
where
    I: IntoIterator,
    I::Item: Future,
{
    JoinAll {
        slots: futures
            .into_iter()
            .map(|fut| JoinSlot::Running(Box::pin(fut)))
            .collect(),
    }
}

enum JoinSlot<Fut: Future> {
    Running(Pin<Box<Fut>>),
    Done(Fut::Output),
    Taken,
}

/// The future returned by [`join_all`].
#[must_use = "futures do nothing unless awaited"]
pub struct JoinAll<Fut: Future> {
    slots: Vec<JoinSlot<Fut>>,
}

impl<Fut: Future> Unpin for JoinAll<Fut> {}

impl<Fut: Future> Future for JoinAll<Fut> {
    type Output = Vec<Fut::Output>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let mut all_done = true;
        for slot in this.slots.iter_mut() {
            if let JoinSlot::Running(fut) = slot {
                match fut.as_mut().poll(cx) {
                    Poll::Ready(output) => *slot = JoinSlot::Done(output),
                    Poll::Pending => all_done = false,
                }
            }
        }
        if !all_done {
            return Poll::Pending;
        }
        let outputs = this
            .slots
            .iter_mut()
            .filter_map(|slot| match std::mem::replace(slot, JoinSlot::Taken) {
                JoinSlot::Done(output) => Some(output),
                JoinSlot::Running(_) | JoinSlot::Taken => None,
            })
            .collect();
        Poll::Ready(outputs)
    }
}
