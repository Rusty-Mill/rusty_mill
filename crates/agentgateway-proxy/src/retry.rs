//! Retrying a failed upstream attempt.
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
//! # What is safe to retry
//!
//! A **connect** failure never reached the upstream, so replaying it cannot
//! duplicate work. Any other transport error is ambiguous: the request may
//! have arrived and been processed, and the response lost on the way back.
//! Replaying that would silently double a payment or a write, so it is not
//! retried.
//!
//! A **configured status code** is a different matter — the upstream answered,
//! so it certainly saw the request. Retrying is the operator's explicit choice
//! by listing the code, which is why nothing is retried on status unless
//! `codes` names it.

use std::collections::BTreeSet;
use std::time::Duration;

use agentgateway_config::RetryPolicy;
use bytes::Bytes;
use hyper::body::{Body as _, Incoming};

/// Largest request body this proxy will hold in memory to make it replayable.
///
/// Anything larger is streamed and not retried. The number is a judgement:
/// big enough for the API calls people actually retry, small enough that a
/// burst of them cannot exhaust the process.
pub const MAX_REPLAY_BYTES: u64 = 64 * 1024;

/// A compiled [`RetryPolicy`].
#[derive(Debug, Clone)]
pub struct Retry {
    attempts: u32,
    backoff: Option<Duration>,
    codes: BTreeSet<u16>,
}

impl Retry {
    /// Compile a retry policy, or `None` when it would never retry.
    pub fn new(policy: &RetryPolicy) -> Option<Self> {
        if policy.attempts == 0 {
            return None;
        }
        Some(Retry {
            attempts: policy.attempts,
            backoff: policy.backoff.map(Duration::from),
            codes: policy.codes.iter().copied().collect(),
        })
    }

    /// Total attempts, counting the first.
    pub fn max_attempts(&self) -> u32 {
        self.attempts.saturating_add(1)
    }

    /// Whether a response with this status should be retried.
    pub fn retries_status(&self, status: u16) -> bool {
        self.codes.contains(&status)
    }

    /// How long to wait before attempt `attempt` (1-based for the first retry).
    ///
    /// Doubles each time, matching the config docs, and is capped so a large
    /// `attempts` cannot produce a wait nobody will sit through.
    pub fn backoff(&self, attempt: u32) -> Option<Duration> {
        let base = self.backoff?;
        let shift = attempt.saturating_sub(1).min(16);
        Some(base.saturating_mul(1u32 << shift).min(MAX_BACKOFF))
    }
}

/// Ceiling on a single backoff wait.
const MAX_BACKOFF: Duration = Duration::from_secs(30);

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
    use agentgateway_config::DurationString;

    fn policy(attempts: u32, backoff: Option<Duration>, codes: &[u16]) -> RetryPolicy {
        RetryPolicy {
            attempts,
            backoff: backoff.map(DurationString),
            codes: codes.to_vec(),
        }
    }

    #[test]
    fn zero_attempts_means_no_retry_policy_at_all() {
        assert!(Retry::new(&policy(0, None, &[503])).is_none());
    }

    #[test]
    fn attempts_counts_retries_not_total_tries() {
        let retry = Retry::new(&policy(2, None, &[])).expect("should compile");
        assert_eq!(
            retry.max_attempts(),
            3,
            "two retries after the first try is three attempts"
        );
    }

    #[test]
    fn only_listed_codes_are_retried() {
        let retry = Retry::new(&policy(1, None, &[502, 503])).expect("should compile");
        assert!(retry.retries_status(503));
        assert!(!retry.retries_status(500), "500 was not opted into");
        assert!(!retry.retries_status(200));
    }

    #[test]
    fn an_empty_code_list_retries_no_status() {
        // Transport failures still retry; a response that arrived does not,
        // because the upstream definitely saw the request.
        let retry = Retry::new(&policy(3, None, &[])).expect("should compile");
        for status in [500, 502, 503, 504] {
            assert!(!retry.retries_status(status));
        }
    }

    #[test]
    fn backoff_doubles_each_attempt() {
        let retry =
            Retry::new(&policy(5, Some(Duration::from_millis(100)), &[])).expect("should compile");
        assert_eq!(retry.backoff(1), Some(Duration::from_millis(100)));
        assert_eq!(retry.backoff(2), Some(Duration::from_millis(200)));
        assert_eq!(retry.backoff(3), Some(Duration::from_millis(400)));
    }

    #[test]
    fn backoff_is_capped() {
        // A large `attempts` with a doubling backoff would otherwise produce a
        // wait nobody will sit through.
        let retry =
            Retry::new(&policy(40, Some(Duration::from_secs(1)), &[])).expect("should compile");
        assert_eq!(retry.backoff(30), Some(MAX_BACKOFF));
    }

    #[test]
    fn no_backoff_configured_means_retry_immediately() {
        let retry = Retry::new(&policy(2, None, &[503])).expect("should compile");
        assert_eq!(retry.backoff(1), None);
    }

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
