//! Compiled retry policies.
//!
//! `retry` is the operator saying "this failure is worth another go". It was
//! consumed only by the HTTP proxy, so an `ai` route with `attempts: 3` got
//! exactly one — the failure this project treats as worse than not supporting
//! the field, because the configuration reads as though it took effect.
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
//!
//! Both rules live here rather than in each caller, so a `503` retried on one
//! backend kind cannot come to mean something else on another.

use std::collections::BTreeSet;
use std::time::Duration;

use agentgateway_config::RetryPolicy;

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

#[cfg(test)]
mod tests {
    use agentgateway_config::DurationString;

    use super::*;

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
}
