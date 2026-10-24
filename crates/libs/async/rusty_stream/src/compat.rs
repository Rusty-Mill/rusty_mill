//! [`Compat`]: interop with `futures_core::Stream` (feature `futures-core`).

use std::pin::Pin;
use std::task::{Context, Poll};

use crate::Stream;

/// Adapts between this crate's [`Stream`] and `futures_core::Stream`.
///
/// `Compat<S>` implements `futures_core::Stream` when `S` implements this
/// crate's `Stream`, and this crate's `Stream` when `S` implements
/// `futures_core::Stream`. Both directions need `S: Unpin`; pin a stream
/// with `Box::pin` first if it is not.
#[derive(Debug, Clone)]
#[must_use = "streams do nothing unless polled"]
pub struct Compat<S> {
    inner: S,
}

impl<S> Compat<S> {
    /// Wrap `inner`.
    pub fn new(inner: S) -> Self {
        Self { inner }
    }

    /// The wrapped stream.
    pub fn into_inner(self) -> S {
        self.inner
    }
}

impl<S: Stream + Unpin> futures_core::Stream for Compat<S> {
    type Item = S::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<S::Item>> {
        Stream::poll_next(Pin::new(&mut self.get_mut().inner), cx)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        Stream::size_hint(&self.inner)
    }
}

impl<S: futures_core::Stream + Unpin> Stream for Compat<S> {
    type Item = S::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<S::Item>> {
        futures_core::Stream::poll_next(Pin::new(&mut self.get_mut().inner), cx)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        futures_core::Stream::size_hint(&self.inner)
    }
}
