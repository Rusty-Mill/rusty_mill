//! The gateway's response body: whatever a backend produced, boxed.

use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt as _, Full};
use hyper::body::{Body as HttpBody, Frame, SizeHint};

/// The error a boxed body can end with.
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// A response body. Every backend's body type is boxed into this one, so the
/// data plane has one response type to hand to the connection.
pub struct Body(UnsyncBoxBody<Bytes, BoxError>);

impl Body {
    /// Boxes `body`.
    pub fn new<B>(body: B) -> Self
    where
        B: HttpBody<Data = Bytes> + Send + 'static,
        B::Error: Into<BoxError>,
    {
        Body(body.map_err(Into::into).boxed_unsync())
    }
}

impl From<Bytes> for Body {
    fn from(bytes: Bytes) -> Self {
        Body::new(Full::new(bytes))
    }
}

impl From<String> for Body {
    fn from(text: String) -> Self {
        Body::from(Bytes::from(text))
    }
}

impl HttpBody for Body {
    type Data = Bytes;
    type Error = BoxError;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, BoxError>>> {
        Pin::new(&mut self.0).poll_frame(cx)
    }

    fn is_end_stream(&self) -> bool {
        self.0.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.0.size_hint()
    }
}

/// A request body on its way to a backend.
///
/// `Stream` is as it arrived, read at most once. `Buffered` has been read
/// already, because a policy had to see it (an `extAuthz` body, an A2A or
/// AG-UI call) or because it is going to a proxied backend, which takes it
/// whole.
pub enum RequestBody {
    /// As it arrived.
    Stream(hyper::body::Incoming),
    /// Read in full.
    Buffered(Bytes),
}

impl HttpBody for RequestBody {
    type Data = Bytes;
    type Error = hyper::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, hyper::Error>>> {
        // Neither variant is structurally pinned: `Incoming` is `Unpin` and
        // `Bytes` holds no self-references.
        match self.get_mut() {
            RequestBody::Stream(body) => Pin::new(body).poll_frame(cx),
            RequestBody::Buffered(bytes) if bytes.is_empty() => Poll::Ready(None),
            RequestBody::Buffered(bytes) => {
                Poll::Ready(Some(Ok(Frame::data(std::mem::take(bytes)))))
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        match self {
            RequestBody::Stream(body) => body.is_end_stream(),
            RequestBody::Buffered(bytes) => bytes.is_empty(),
        }
    }

    fn size_hint(&self) -> SizeHint {
        match self {
            RequestBody::Stream(body) => body.size_hint(),
            RequestBody::Buffered(bytes) => SizeHint::with_exact(bytes.len() as u64),
        }
    }
}
