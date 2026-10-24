//! Stream constructors: [`iter`], [`once`], [`unfold`], [`select_all`].

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

pub use crate::{BoxStream, Stream, StreamExt};

/// A stream over an iterator's items, each ready at once.
pub fn iter<I: IntoIterator>(items: I) -> Iter<I::IntoIter> {
    Iter {
        items: items.into_iter(),
    }
}

/// The stream returned by [`iter`].
#[derive(Debug, Clone)]
#[must_use = "streams do nothing unless polled"]
pub struct Iter<I> {
    items: I,
}

impl<I> Unpin for Iter<I> {}

impl<I: Iterator> Stream for Iter<I> {
    type Item = I::Item;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<I::Item>> {
        Poll::Ready(self.get_mut().items.next())
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.items.size_hint()
    }
}

/// A stream that yields `fut`'s output once.
pub fn once<Fut: Future>(fut: Fut) -> Once<Fut> {
    Once {
        fut: Some(Box::pin(fut)),
    }
}

/// The stream returned by [`once`].
#[must_use = "streams do nothing unless polled"]
pub struct Once<Fut> {
    fut: Option<Pin<Box<Fut>>>,
}

impl<Fut: Future> Stream for Once<Fut> {
    type Item = Fut::Output;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Fut::Output>> {
        let this = self.get_mut();
        let Some(fut) = this.fut.as_mut() else {
            return Poll::Ready(None);
        };
        let output = match fut.as_mut().poll(cx) {
            Poll::Ready(output) => output,
            Poll::Pending => return Poll::Pending,
        };
        this.fut = None;
        Poll::Ready(Some(output))
    }
}

/// A stream built from a state and an async step: `f(state)` resolves to
/// `Some((item, next_state))`, or `None` to end the stream.
pub fn unfold<T, Fut, F, Item>(init: T, f: F) -> Unfold<T, Fut, F>
where
    F: FnMut(T) -> Fut,
    Fut: Future<Output = Option<(Item, T)>>,
{
    Unfold {
        state: UnfoldState::Value(init),
        f,
    }
}

enum UnfoldState<T, Fut> {
    Value(T),
    Running(Pin<Box<Fut>>),
    Done,
}

/// The stream returned by [`unfold`].
#[must_use = "streams do nothing unless polled"]
pub struct Unfold<T, Fut, F> {
    state: UnfoldState<T, Fut>,
    f: F,
}

impl<T, Fut, F> Unpin for Unfold<T, Fut, F> {}

impl<T, Fut, F, Item> Stream for Unfold<T, Fut, F>
where
    F: FnMut(T) -> Fut,
    Fut: Future<Output = Option<(Item, T)>>,
{
    type Item = Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Item>> {
        let this = self.get_mut();
        let mut fut = match std::mem::replace(&mut this.state, UnfoldState::Done) {
            UnfoldState::Value(state) => Box::pin((this.f)(state)),
            UnfoldState::Running(fut) => fut,
            UnfoldState::Done => return Poll::Ready(None),
        };
        match fut.as_mut().poll(cx) {
            Poll::Ready(Some((item, next))) => {
                this.state = UnfoldState::Value(next);
                Poll::Ready(Some(item))
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => {
                this.state = UnfoldState::Running(fut);
                Poll::Pending
            }
        }
    }
}

/// Merge streams, yielding items from whichever is ready. Each poll starts
/// one stream further along, so a busy stream cannot starve the others.
pub fn select_all<I>(streams: I) -> SelectAll<I::Item>
where
    I: IntoIterator,
    I::Item: Stream + Unpin,
{
    SelectAll {
        streams: streams.into_iter().collect(),
        start: 0,
    }
}

/// The stream returned by [`select_all`].
#[derive(Debug)]
#[must_use = "streams do nothing unless polled"]
pub struct SelectAll<S> {
    streams: Vec<S>,
    start: usize,
}

impl<S> SelectAll<S> {
    /// Add another stream to the set.
    pub fn push(&mut self, stream: S) {
        self.streams.push(stream);
    }

    /// The number of streams still running.
    pub fn len(&self) -> usize {
        self.streams.len()
    }

    /// Whether every stream has ended.
    pub fn is_empty(&self) -> bool {
        self.streams.is_empty()
    }
}

impl<S> Unpin for SelectAll<S> {}

impl<S: Stream + Unpin> Stream for SelectAll<S> {
    type Item = S::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<S::Item>> {
        let this = self.get_mut();
        let len = this.streams.len();
        // Visit each stream once, in rotated order, against a fixed index
        // space: ended streams are removed only after the loop, so a removal
        // cannot shift an unvisited stream out of this poll.
        let mut ended = Vec::new();
        let mut found = None;
        for k in 0..len {
            let i = (this.start + k) % len;
            match Pin::new(&mut this.streams[i]).poll_next(cx) {
                Poll::Ready(Some(item)) => {
                    this.start = i + 1;
                    found = Some(item);
                    break;
                }
                Poll::Ready(None) => ended.push(i),
                Poll::Pending => {}
            }
        }
        // Highest index first, so each `swap_remove` moves in a stream that
        // is staying.
        ended.sort_unstable_by(|a, b| b.cmp(a));
        for i in ended {
            this.streams.swap_remove(i);
        }
        match found {
            Some(item) => Poll::Ready(Some(item)),
            None if this.streams.is_empty() => Poll::Ready(None),
            None => Poll::Pending,
        }
    }
}
