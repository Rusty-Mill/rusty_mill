//! `rusty_stream` — the workspace's async [`Stream`] trait, its combinators,
//! and the [`stream!`]/[`try_stream!`] macros. The in-house replacement for
//! `futures`, `futures-util`, `futures-core` and `async-stream` (#896;
//! `docs/research/ASYNC-RUNTIME-INVENTORY.md`, gap G7).
//!
//! Runtime-agnostic: nothing here needs a particular executor.
//!
//! **Unpin, not unsafe.** Adapters require `Unpin` streams and box the futures
//! they hold, so the crate has no `unsafe` code. Pin a `!Unpin` stream first
//! with [`StreamExt::boxed`] or `Box::pin`. Streams from [`stream!`],
//! [`try_stream!`] and this crate's constructors are already `Unpin`.
//!
//! **Interop.** With the `futures-core` feature, [`Compat`] adapts this
//! crate's [`Stream`] to `futures_core::Stream` and back, for the third-party
//! crates (axum SSE, reqwest, tonic) that speak the latter.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod async_stream;
pub mod channel;
#[cfg(feature = "futures-core")]
mod compat;
mod ext;
pub mod future;
pub mod stream;
mod try_ext;

use std::ops::DerefMut;
use std::pin::Pin;
use std::task::{Context, Poll};

#[cfg(feature = "futures-core")]
pub use compat::Compat;
pub use ext::{Buffered, Collect, Filter, FilterMap, Map, Next, StreamExt, Take, Then};
pub use future::BoxFuture;
pub use rusty_stream_macros::{stream, try_stream};
pub use try_ext::{MapErr, MapOk, TryCollect, TryNext, TryStream, TryStreamExt};

/// Support code for the macros. Not public API.
#[doc(hidden)]
pub mod __private {
    pub use crate::async_stream::{stream, try_stream, Yielder};
}

/// An asynchronous sequence of values: the async counterpart of [`Iterator`].
pub trait Stream {
    /// The values the stream yields.
    type Item;

    /// Poll for the next value. `Ready(None)` means the stream has ended.
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>>;

    /// Bounds on the remaining length, as [`Iterator::size_hint`].
    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, None)
    }
}

impl<S: Stream + Unpin + ?Sized> Stream for &mut S {
    type Item = S::Item;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<S::Item>> {
        Pin::new(&mut **self).poll_next(cx)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (**self).size_hint()
    }
}

impl<S: Stream + Unpin + ?Sized> Stream for Box<S> {
    type Item = S::Item;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<S::Item>> {
        Pin::new(&mut **self).poll_next(cx)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (**self).size_hint()
    }
}

impl<P> Stream for Pin<P>
where
    P: DerefMut + Unpin,
    P::Target: Stream,
{
    type Item = <P::Target as Stream>::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().as_mut().poll_next(cx)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (**self).size_hint()
    }
}

/// A boxed, type-erased, `Send` stream.
pub type BoxStream<'a, T> = Pin<Box<dyn Stream<Item = T> + Send + 'a>>;
