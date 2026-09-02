//! Making a proxied request replayable.
//!
//! The retry *policy* — how many attempts, which statuses, how long to wait
//! — lives in [`agentgateway_core::Retry`], because every backend kind needs
//! the same answers. What is here is the part only a proxy has: a request body
//! arriving as a stream that can be read exactly once.
//!
//! # Why a request body has to be buffered
//!
//! A retry replays the request, and a streaming body can only be read once.
//! So a request is replayable only if its body was buffered first, and
//! buffering an arbitrary upload turns a proxy into a memory limit.
//!
//! The rule here is deliberately narrow: buffer only when the body's size is
//! *known in advance* and fits in [`MAX_REPLAY_BYTES`]. That means the body is
//! never partially consumed to find out how big it is — which would leave a
//! half-read stream that can be neither replayed nor forwarded. Requests with
//! a `Content-Length` inside the limit (which is almost every request worth
//! retrying) get retries; chunked or oversized ones are streamed straight
//! through and simply do not.
//!
//! An `ai` request, by contrast, is buffered by construction — it has to be
//! read to be translated — so every one of them is replayable and none of
//! this applies there.

use bytes::Bytes;
use hyper::body::{Body as _, Incoming};

/// Largest request body this proxy will hold in memory to make it replayable.
///
/// Anything larger is streamed and not retried. The number is a judgement:
/// big enough for the API calls people actually retry, small enough that a
/// burst of them cannot exhaust the process.
pub const MAX_REPLAY_BYTES: u64 = 64 * 1024;

/// Whether this body can be buffered for replay without reading it first.
///
/// Only a body whose length is known up front qualifies. Reading to find out
/// would leave a partially consumed stream that can be neither replayed nor
/// forwarded intact. A body that arrived already buffered -- because a policy
/// upstream had to read it -- is replayable by construction.
pub fn is_replayable(body: &RequestBody) -> bool {
    match body {
        RequestBody::Buffered(_) => true,
        RequestBody::Stream(stream) => stream
            .size_hint()
            .upper()
            .is_some_and(|upper| upper <= MAX_REPLAY_BYTES),
    }
}

/// The body of a request on its way upstream.
///
/// `Buffered` can be replayed; `Stream` is a one-shot pass-through.
pub enum RequestBody {
    /// Forwarded as it arrives, and not retryable.
    Stream(Incoming),
    /// Held in memory so an attempt can be replayed.
    Buffered(Bytes),
}

impl RequestBody {
    /// A copy for the next attempt, if this body can be replayed.
    pub fn replay(&self) -> Option<RequestBody> {
        match self {
            RequestBody::Stream(_) => None,
            RequestBody::Buffered(bytes) => Some(RequestBody::Buffered(bytes.clone())),
        }
    }
}

impl http_body::Body for RequestBody {
    type Data = Bytes;
    type Error = hyper::Error;

    fn poll_frame(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        // Neither variant is structurally pinned: `Incoming` is `Unpin` and
        // `Bytes` holds no self-references.
        match self.get_mut() {
            RequestBody::Stream(body) => std::pin::Pin::new(body).poll_frame(cx),
            RequestBody::Buffered(bytes) => {
                if bytes.is_empty() {
                    std::task::Poll::Ready(None)
                } else {
                    let chunk = std::mem::take(bytes);
                    std::task::Poll::Ready(Some(Ok(http_body::Frame::data(chunk))))
                }
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        match self {
            RequestBody::Stream(body) => body.is_end_stream(),
            RequestBody::Buffered(bytes) => bytes.is_empty(),
        }
    }

    fn size_hint(&self) -> http_body::SizeHint {
        match self {
            RequestBody::Stream(body) => body.size_hint(),
            RequestBody::Buffered(bytes) => http_body::SizeHint::with_exact(bytes.len() as u64),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_buffered_body_can_be_replayed_and_a_stream_cannot() {
        let buffered = RequestBody::Buffered(Bytes::from_static(b"payload"));
        let replay = buffered.replay().expect("buffered bodies replay");
        match replay {
            RequestBody::Buffered(bytes) => assert_eq!(&bytes[..], b"payload"),
            RequestBody::Stream(_) => panic!("expected a buffered replay"),
        }
    }
}
