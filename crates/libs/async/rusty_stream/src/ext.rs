//! [`StreamExt`] and its adapters.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::{BoxStream, Stream};

/// Combinators for every [`Stream`].
pub trait StreamExt: Stream {
    /// The next item, or `None` once the stream has ended.
    fn next(&mut self) -> Next<'_, Self>
    where
        Self: Unpin,
    {
        Next { stream: self }
    }

    /// Transform each item with `f`.
    fn map<T, F>(self, f: F) -> Map<Self, F>
    where
        Self: Sized,
        F: FnMut(Self::Item) -> T,
    {
        Map { stream: self, f }
    }

    /// Keep the items for which the async predicate `f` resolves to `true`.
    fn filter<Fut, F>(self, f: F) -> Filter<Self, Fut, F>
    where
        Self: Sized,
        F: FnMut(&Self::Item) -> Fut,
        Fut: Future<Output = bool>,
    {
        Filter {
            stream: self,
            f,
            pending: None,
        }
    }

    /// Transform each item with the async `f`, dropping the `None`s.
    fn filter_map<T, Fut, F>(self, f: F) -> FilterMap<Self, Fut, F>
    where
        Self: Sized,
        F: FnMut(Self::Item) -> Fut,
        Fut: Future<Output = Option<T>>,
    {
        FilterMap {
            stream: self,
            f,
            pending: None,
        }
    }

    /// Transform each item with the async `f`, one at a time.
    fn then<Fut, F>(self, f: F) -> Then<Self, Fut, F>
    where
        Self: Sized,
        F: FnMut(Self::Item) -> Fut,
        Fut: Future,
    {
        Then {
            stream: self,
            f,
            pending: None,
        }
    }

    /// Run up to `limit` of the stream's futures at once, yielding their
    /// outputs in the stream's order.
    fn buffered(self, limit: usize) -> Buffered<Self>
    where
        Self: Sized,
        Self::Item: Future,
    {
        Buffered {
            stream: Some(self),
            queue: VecDeque::new(),
            limit: limit.max(1),
        }
    }

    /// At most the first `n` items.
    fn take(self, n: usize) -> Take<Self>
    where
        Self: Sized,
    {
        Take {
            stream: self,
            remaining: n,
        }
    }

    /// Collect every item into `C`.
    fn collect<C>(self) -> Collect<Self, C>
    where
        Self: Sized,
        C: Default + Extend<Self::Item>,
    {
        Collect {
            stream: self,
            items: C::default(),
        }
    }

    /// Box and pin the stream, erasing its type.
    fn boxed<'a>(self) -> BoxStream<'a, Self::Item>
    where
        Self: Sized + Send + 'a,
    {
        Box::pin(self)
    }
}

impl<S: Stream + ?Sized> StreamExt for S {}

/// The future returned by [`StreamExt::next`].
#[derive(Debug)]
#[must_use = "futures do nothing unless awaited"]
pub struct Next<'a, S: ?Sized> {
    stream: &'a mut S,
}

impl<S: Stream + Unpin + ?Sized> Future for Next<'_, S> {
    type Output = Option<S::Item>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut *self.stream).poll_next(cx)
    }
}

/// The stream returned by [`StreamExt::map`].
#[derive(Debug)]
#[must_use = "streams do nothing unless polled"]
pub struct Map<S, F> {
    stream: S,
    f: F,
}

impl<S: Unpin, F> Unpin for Map<S, F> {}

impl<S: Stream + Unpin, T, F: FnMut(S::Item) -> T> Stream for Map<S, F> {
    type Item = T;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
        let this = self.get_mut();
        Pin::new(&mut this.stream)
            .poll_next(cx)
            .map(|item| item.map(&mut this.f))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.stream.size_hint()
    }
}

/// The stream returned by [`StreamExt::filter`].
#[must_use = "streams do nothing unless polled"]
pub struct Filter<S: Stream, Fut, F> {
    stream: S,
    f: F,
    pending: Option<(S::Item, Pin<Box<Fut>>)>,
}

impl<S: Stream + Unpin, Fut, F> Unpin for Filter<S, Fut, F> {}

impl<S, Fut, F> Stream for Filter<S, Fut, F>
where
    S: Stream + Unpin,
    F: FnMut(&S::Item) -> Fut,
    Fut: Future<Output = bool>,
{
    type Item = S::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<S::Item>> {
        let this = self.get_mut();
        loop {
            if let Some((item, mut verdict)) = this.pending.take() {
                match verdict.as_mut().poll(cx) {
                    Poll::Ready(true) => return Poll::Ready(Some(item)),
                    Poll::Ready(false) => {}
                    Poll::Pending => {
                        this.pending = Some((item, verdict));
                        return Poll::Pending;
                    }
                }
            }
            match Pin::new(&mut this.stream).poll_next(cx) {
                Poll::Ready(Some(item)) => {
                    let verdict = Box::pin((this.f)(&item));
                    this.pending = Some((item, verdict));
                }
                other => return other,
            }
        }
    }
}

/// The stream returned by [`StreamExt::filter_map`].
#[must_use = "streams do nothing unless polled"]
pub struct FilterMap<S, Fut, F> {
    stream: S,
    f: F,
    pending: Option<Pin<Box<Fut>>>,
}

impl<S: Unpin, Fut, F> Unpin for FilterMap<S, Fut, F> {}

impl<S, T, Fut, F> Stream for FilterMap<S, Fut, F>
where
    S: Stream + Unpin,
    F: FnMut(S::Item) -> Fut,
    Fut: Future<Output = Option<T>>,
{
    type Item = T;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
        let this = self.get_mut();
        loop {
            if let Some(fut) = this.pending.as_mut() {
                let mapped = match fut.as_mut().poll(cx) {
                    Poll::Ready(mapped) => mapped,
                    Poll::Pending => return Poll::Pending,
                };
                this.pending = None;
                if mapped.is_some() {
                    return Poll::Ready(mapped);
                }
            }
            match Pin::new(&mut this.stream).poll_next(cx) {
                Poll::Ready(Some(item)) => this.pending = Some(Box::pin((this.f)(item))),
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

/// The stream returned by [`StreamExt::then`].
#[must_use = "streams do nothing unless polled"]
pub struct Then<S, Fut, F> {
    stream: S,
    f: F,
    pending: Option<Pin<Box<Fut>>>,
}

impl<S: Unpin, Fut, F> Unpin for Then<S, Fut, F> {}

impl<S, Fut, F> Stream for Then<S, Fut, F>
where
    S: Stream + Unpin,
    F: FnMut(S::Item) -> Fut,
    Fut: Future,
{
    type Item = Fut::Output;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Fut::Output>> {
        let this = self.get_mut();
        let mut fut = match this.pending.take() {
            Some(fut) => fut,
            None => match Pin::new(&mut this.stream).poll_next(cx) {
                Poll::Ready(Some(item)) => Box::pin((this.f)(item)),
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => return Poll::Pending,
            },
        };
        match fut.as_mut().poll(cx) {
            Poll::Ready(output) => Poll::Ready(Some(output)),
            Poll::Pending => {
                this.pending = Some(fut);
                Poll::Pending
            }
        }
    }
}

/// One in-flight future of [`Buffered`].
enum Slot<Fut: Future> {
    Running(Pin<Box<Fut>>),
    Done(Fut::Output),
}

/// The stream returned by [`StreamExt::buffered`].
#[must_use = "streams do nothing unless polled"]
pub struct Buffered<S: Stream>
where
    S::Item: Future,
{
    stream: Option<S>,
    queue: VecDeque<Slot<S::Item>>,
    limit: usize,
}

impl<S: Stream + Unpin> Unpin for Buffered<S> where S::Item: Future {}

impl<S> Stream for Buffered<S>
where
    S: Stream + Unpin,
    S::Item: Future,
{
    type Item = <S::Item as Future>::Output;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        while this.queue.len() < this.limit {
            let Some(stream) = this.stream.as_mut() else {
                break;
            };
            match Pin::new(stream).poll_next(cx) {
                Poll::Ready(Some(fut)) => this.queue.push_back(Slot::Running(Box::pin(fut))),
                Poll::Ready(None) => this.stream = None,
                Poll::Pending => break,
            }
        }
        for slot in this.queue.iter_mut() {
            if let Slot::Running(fut) = slot {
                if let Poll::Ready(output) = fut.as_mut().poll(cx) {
                    *slot = Slot::Done(output);
                }
            }
        }
        if matches!(this.queue.front(), Some(Slot::Done(_))) {
            if let Some(Slot::Done(output)) = this.queue.pop_front() {
                return Poll::Ready(Some(output));
            }
        }
        if this.queue.is_empty() && this.stream.is_none() {
            Poll::Ready(None)
        } else {
            Poll::Pending
        }
    }
}

/// The stream returned by [`StreamExt::take`].
#[derive(Debug)]
#[must_use = "streams do nothing unless polled"]
pub struct Take<S> {
    stream: S,
    remaining: usize,
}

impl<S: Stream + Unpin> Stream for Take<S> {
    type Item = S::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<S::Item>> {
        let this = self.get_mut();
        if this.remaining == 0 {
            return Poll::Ready(None);
        }
        let item = Pin::new(&mut this.stream).poll_next(cx);
        if let Poll::Ready(Some(_)) = item {
            this.remaining -= 1;
        }
        item
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let (low, high) = self.stream.size_hint();
        let high = high.map_or(self.remaining, |h| h.min(self.remaining));
        (low.min(self.remaining), Some(high))
    }
}

/// The future returned by [`StreamExt::collect`].
#[derive(Debug)]
#[must_use = "futures do nothing unless awaited"]
pub struct Collect<S, C> {
    stream: S,
    items: C,
}

impl<S: Unpin, C> Unpin for Collect<S, C> {}

impl<S, C> Future for Collect<S, C>
where
    S: Stream + Unpin,
    C: Default + Extend<S::Item>,
{
    type Output = C;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<C> {
        let this = self.get_mut();
        loop {
            match Pin::new(&mut this.stream).poll_next(cx) {
                Poll::Ready(Some(item)) => this.items.extend(Some(item)),
                Poll::Ready(None) => return Poll::Ready(std::mem::take(&mut this.items)),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}
