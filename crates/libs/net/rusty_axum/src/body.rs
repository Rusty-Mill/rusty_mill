//! Response bodies: whole, or a stream of chunks.

use std::fmt;
use std::future::poll_fn;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_core::Stream;

/// The error type a body stream may end with.
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

type ChunkStream = Pin<Box<dyn Stream<Item = Result<Vec<u8>, BoxError>> + Send>>;

/// A response body. A whole body is sent with `Content-Length`; a stream is
/// sent chunked, each item as it is produced (SSE, streamed replies).
pub struct Body(pub(crate) Inner);

pub(crate) enum Inner {
    Full(Vec<u8>),
    Stream(ChunkStream),
}

impl Body {
    /// An empty body.
    pub fn empty() -> Self {
        Body(Inner::Full(Vec::new()))
    }

    /// A body written chunk by chunk as `stream` yields. An `Err` item ends
    /// the response early; the client sees a truncated body.
    pub fn from_stream<S, E>(stream: S) -> Self
    where
        S: Stream<Item = Result<Vec<u8>, E>> + Send + 'static,
        E: Into<BoxError>,
    {
        Body(Inner::Stream(Box::pin(ErrInto(Box::pin(stream)))))
    }

    /// The whole body when it is not a stream.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match &self.0 {
            Inner::Full(bytes) => Some(bytes),
            Inner::Stream(_) => None,
        }
    }

    /// Reads the body to its end, a stream included: for tests and for
    /// middleware that needs the whole body.
    pub async fn collect(self) -> Result<Vec<u8>, BoxError> {
        let mut stream = match self.0 {
            Inner::Full(bytes) => return Ok(bytes),
            Inner::Stream(stream) => stream,
        };
        let mut out = Vec::new();
        while let Some(chunk) = poll_fn(|cx| stream.as_mut().poll_next(cx)).await {
            out.extend_from_slice(&chunk?);
        }
        Ok(out)
    }
}

impl Default for Body {
    fn default() -> Self {
        Body::empty()
    }
}

impl From<Vec<u8>> for Body {
    fn from(bytes: Vec<u8>) -> Self {
        Body(Inner::Full(bytes))
    }
}

impl From<String> for Body {
    fn from(text: String) -> Self {
        Body(Inner::Full(text.into_bytes()))
    }
}

impl From<&'static str> for Body {
    fn from(text: &'static str) -> Self {
        Body(Inner::Full(text.as_bytes().to_vec()))
    }
}

impl fmt::Debug for Body {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Inner::Full(bytes) => write!(f, "Body::Full({} bytes)", bytes.len()),
            Inner::Stream(_) => f.write_str("Body::Stream"),
        }
    }
}

/// Converts a stream's error type into [`BoxError`].
struct ErrInto<S>(Pin<Box<S>>);

impl<S, E> Stream for ErrInto<S>
where
    S: Stream<Item = Result<Vec<u8>, E>>,
    E: Into<BoxError>,
{
    type Item = Result<Vec<u8>, BoxError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.0
            .as_mut()
            .poll_next(cx)
            .map(|item| item.map(|r| r.map_err(Into::into)))
    }
}
