//! [`TryStream`], [`TryStreamExt`] and their adapters: combinators for
//! streams of `Result`s.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::Stream;

/// A stream of `Result<Self::Ok, Self::Error>`. Implemented for every such
/// [`Stream`]; it names the two halves so adapters can bound on them.
pub trait TryStream: Stream<Item = Result<Self::Ok, Self::Error>> {
    /// The success type.
    type Ok;
    /// The error type.
    type Error;
}

impl<S, T, E> TryStream for S
where
    S: Stream<Item = Result<T, E>> + ?Sized,
{
    type Ok = T;
    type Error = E;
}

/// Combinators for every [`TryStream`].
pub trait TryStreamExt: TryStream {
    /// The next item: `Ok(Some(v))`, `Ok(None)` at the end, or the error.
    fn try_next(&mut self) -> TryNext<'_, Self>
    where
        Self: Unpin,
    {
        TryNext { stream: self }
    }

    /// Transform each `Ok` value with `f`.
    fn map_ok<T, F>(self, f: F) -> MapOk<Self, F>
    where
        Self: Sized,
        F: FnMut(Self::Ok) -> T,
    {
        MapOk { stream: self, f }
    }

    /// Transform each error with `f`.
    fn map_err<E, F>(self, f: F) -> MapErr<Self, F>
    where
        Self: Sized,
        F: FnMut(Self::Error) -> E,
    {
        MapErr { stream: self, f }
    }

    /// Collect the `Ok` values into `C`, stopping at the first error.
    fn try_collect<C>(self) -> TryCollect<Self, C>
    where
        Self: Sized,
        C: Default + Extend<Self::Ok>,
    {
        TryCollect {
            stream: self,
            items: C::default(),
        }
    }
}

impl<S: TryStream + ?Sized> TryStreamExt for S {}

/// The future returned by [`TryStreamExt::try_next`].
#[derive(Debug)]
#[must_use = "futures do nothing unless awaited"]
pub struct TryNext<'a, S: ?Sized> {
    stream: &'a mut S,
}

impl<S: TryStream + Unpin + ?Sized> Future for TryNext<'_, S> {
    type Output = Result<Option<S::Ok>, S::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut *self.stream)
            .poll_next(cx)
            .map(Option::transpose)
    }
}

/// The stream returned by [`TryStreamExt::map_ok`].
#[derive(Debug)]
#[must_use = "streams do nothing unless polled"]
pub struct MapOk<S, F> {
    stream: S,
    f: F,
}

impl<S: Unpin, F> Unpin for MapOk<S, F> {}

impl<S, T, F> Stream for MapOk<S, F>
where
    S: TryStream + Unpin,
    F: FnMut(S::Ok) -> T,
{
    type Item = Result<T, S::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        Pin::new(&mut this.stream)
            .poll_next(cx)
            .map(|item| item.map(|result| result.map(&mut this.f)))
    }
}

/// The stream returned by [`TryStreamExt::map_err`].
#[derive(Debug)]
#[must_use = "streams do nothing unless polled"]
pub struct MapErr<S, F> {
    stream: S,
    f: F,
}

impl<S: Unpin, F> Unpin for MapErr<S, F> {}

impl<S, E, F> Stream for MapErr<S, F>
where
    S: TryStream + Unpin,
    F: FnMut(S::Error) -> E,
{
    type Item = Result<S::Ok, E>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        Pin::new(&mut this.stream)
            .poll_next(cx)
            .map(|item| item.map(|result| result.map_err(&mut this.f)))
    }
}

/// The future returned by [`TryStreamExt::try_collect`].
#[derive(Debug)]
#[must_use = "futures do nothing unless awaited"]
pub struct TryCollect<S, C> {
    stream: S,
    items: C,
}

impl<S: Unpin, C> Unpin for TryCollect<S, C> {}

impl<S, C> Future for TryCollect<S, C>
where
    S: TryStream + Unpin,
    C: Default + Extend<S::Ok>,
{
    type Output = Result<C, S::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        loop {
            match Pin::new(&mut this.stream).poll_next(cx) {
                Poll::Ready(Some(Ok(item))) => this.items.extend(Some(item)),
                Poll::Ready(Some(Err(error))) => return Poll::Ready(Err(error)),
                Poll::Ready(None) => return Poll::Ready(Ok(std::mem::take(&mut this.items))),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}
