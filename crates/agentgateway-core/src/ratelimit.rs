//! Token-bucket rate limiting.
//!
//! A route may carry several limits at once — the usual shape is a small
//! bucket for burst and a large one for sustained rate — and a request must
//! satisfy *all* of them. That is why the buckets share one lock: checking
//! them one at a time would consume a token from the first bucket before
//! discovering the second refuses, quietly charging a request that was never
//! served.
//!
//! Refill is lazy. There is no background task ticking every bucket in the
//! process; a bucket works out how many intervals have elapsed the next time
//! someone looks at it. Idle routes cost nothing.
//!
//! Time is a parameter rather than an ambient read, so the tests drive a clock
//! instead of sleeping. A rate limiter tested with `sleep` is a rate limiter
//! tested at one resolution, on one machine, when the CI box was not busy.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use agentgateway_config::{LocalRateLimit, RateLimitKind};

/// A configuration a bucket cannot be built from.
#[derive(Debug, thiserror::Error)]
pub enum RateLimitError {
    /// A bucket that can never hold a token would refuse every request.
    #[error("{at}: maxTokens is 0, so every request would be refused")]
    NoCapacity {
        /// Where in the configuration it came from.
        at: String,
    },

    /// A bucket that never refills drains once and stays empty.
    #[error(
        "{at}: tokensPerFill is 0, so the bucket drains once and never refills; \
         use a non-zero fill or remove the limit"
    )]
    NoRefill {
        /// Where in the configuration it came from.
        at: String,
    },

    /// A zero interval would divide by zero working out the refill.
    #[error("{at}: fillInterval is 0")]
    NoInterval {
        /// Where in the configuration it came from.
        at: String,
    },
}

/// How long until the caller should try again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryAfter(pub Duration);

impl RetryAfter {
    /// The value for a `Retry-After` header, in whole seconds.
    ///
    /// Rounded up, and never zero: `Retry-After: 0` invites an immediate
    /// retry, which is the opposite of what a rate limit is asking for.
    pub fn seconds(self) -> u64 {
        let secs = self.0.as_secs();
        if self.0.subsec_nanos() > 0 {
            secs.saturating_add(1).max(1)
        } else {
            secs.max(1)
        }
    }
}

/// One bucket's immutable settings.
#[derive(Debug)]
struct Limit {
    max_tokens: u64,
    tokens_per_fill: u64,
    fill_interval: Duration,
}

/// One bucket's mutable state.
#[derive(Debug)]
struct State {
    tokens: u64,
    /// Start of the interval the bucket has already been credited for.
    last_fill: Instant,
}

/// The rate limits attached to a route.
#[derive(Debug)]
pub struct RateLimiter {
    limits: Vec<Limit>,
    state: Mutex<Vec<State>>,
}

impl RateLimiter {
    /// Compile a route's limits.
    ///
    /// Returns `None` when the route configures none, so the caller can skip
    /// the check rather than take a lock to learn there is nothing to do.
    pub fn new(limits: &[LocalRateLimit], at: &str) -> Result<Option<Self>, RateLimitError> {
        // `tokens` counts LLM tokens, which needs the LLM gateway to exist
        // before it can mean anything. Skipping is what `Config::lint`
        // reports, so it is not silent.
        let applicable: Vec<&LocalRateLimit> = limits
            .iter()
            .filter(|limit| limit.kind == RateLimitKind::Requests)
            .collect();

        if applicable.is_empty() {
            return Ok(None);
        }

        let mut compiled = Vec::with_capacity(applicable.len());
        for (i, limit) in applicable.iter().enumerate() {
            let at = format!("{at}.localRateLimit[{i}]");
            if limit.max_tokens == 0 {
                return Err(RateLimitError::NoCapacity { at });
            }
            if limit.tokens_per_fill == 0 {
                return Err(RateLimitError::NoRefill { at });
            }
            if limit.fill_interval.is_zero() {
                return Err(RateLimitError::NoInterval { at });
            }
            compiled.push(Limit {
                max_tokens: limit.max_tokens,
                tokens_per_fill: limit.tokens_per_fill,
                fill_interval: *limit.fill_interval,
            });
        }

        // Buckets start full, so a gateway that has just restarted does not
        // spend its first interval refusing traffic it has no reason to.
        let now = Instant::now();
        let state = compiled
            .iter()
            .map(|limit| State {
                tokens: limit.max_tokens,
                last_fill: now,
            })
            .collect();

        Ok(Some(RateLimiter {
            limits: compiled,
            state: Mutex::new(state),
        }))
    }

    /// Charge one request against every bucket.
    pub fn check(&self) -> Result<(), RetryAfter> {
        self.check_at(Instant::now())
    }

    /// As [`RateLimiter::check`], at an explicit instant.
    pub fn check_at(&self, now: Instant) -> Result<(), RetryAfter> {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            // A panic elsewhere poisoned the lock. Failing open beats failing
            // every request forever: this limits traffic, it does not secure
            // anything.
            Err(poisoned) => poisoned.into_inner(),
        };

        for (limit, state) in self.limits.iter().zip(state.iter_mut()) {
            refill(limit, state, now);
        }

        // Check every bucket before consuming from any: charging a token to
        // the burst bucket and then refusing on the sustained one bills a
        // request that never ran.
        if let Some(wait) = self
            .limits
            .iter()
            .zip(state.iter())
            .filter(|(_, state)| state.tokens == 0)
            .map(|(limit, state)| wait_for_token(limit, state, now))
            .max()
        {
            return Err(RetryAfter(wait));
        }

        for state in state.iter_mut() {
            state.tokens = state.tokens.saturating_sub(1);
        }
        Ok(())
    }

    /// Tokens left in each bucket, for tests and a health endpoint.
    pub fn available(&self) -> Vec<u64> {
        match self.state.lock() {
            Ok(state) => state.iter().map(|s| s.tokens).collect(),
            Err(poisoned) => poisoned.into_inner().iter().map(|s| s.tokens).collect(),
        }
    }
}

/// Credit whole elapsed intervals, capped at the bucket's capacity.
fn refill(limit: &Limit, state: &mut State, now: Instant) {
    let elapsed = now.saturating_duration_since(state.last_fill);
    let intervals = elapsed.as_nanos() / limit.fill_interval.as_nanos().max(1);
    if intervals == 0 {
        return;
    }

    let credit = u64::try_from(intervals)
        .unwrap_or(u64::MAX)
        .saturating_mul(limit.tokens_per_fill);
    state.tokens = state.tokens.saturating_add(credit).min(limit.max_tokens);

    // Advance by whole intervals only. Setting `last_fill = now` would discard
    // the remainder every time and stretch the effective interval under steady
    // traffic, so the bucket would refill slower than configured.
    let consumed = limit
        .fill_interval
        .saturating_mul(u32::try_from(intervals).unwrap_or(u32::MAX));
    state.last_fill = state
        .last_fill
        .checked_add(consumed)
        .unwrap_or(now)
        .min(now);
}

/// How long until this empty bucket has a token.
fn wait_for_token(limit: &Limit, state: &State, now: Instant) -> Duration {
    let since = now.saturating_duration_since(state.last_fill);
    limit.fill_interval.saturating_sub(since)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentgateway_config::DurationString;

    fn limiter(limits: &[(u64, u64, Duration)]) -> RateLimiter {
        let config: Vec<LocalRateLimit> = limits
            .iter()
            .map(|(max_tokens, tokens_per_fill, interval)| LocalRateLimit {
                max_tokens: *max_tokens,
                tokens_per_fill: *tokens_per_fill,
                fill_interval: DurationString(*interval),
                kind: RateLimitKind::Requests,
            })
            .collect();
        RateLimiter::new(&config, "test")
            .expect("should compile")
            .expect("should be present")
    }

    #[test]
    fn a_bucket_starts_full() {
        // A gateway that just restarted has no reason to spend its first
        // interval refusing traffic.
        let limiter = limiter(&[(3, 1, Duration::from_secs(1))]);
        let now = Instant::now();
        for _ in 0..3 {
            assert!(limiter.check_at(now).is_ok());
        }
        assert!(
            limiter.check_at(now).is_err(),
            "the fourth exceeds the burst"
        );
    }

    #[test]
    fn tokens_come_back_after_an_interval() {
        let limiter = limiter(&[(2, 2, Duration::from_secs(1))]);
        let start = Instant::now();

        assert!(limiter.check_at(start).is_ok());
        assert!(limiter.check_at(start).is_ok());
        assert!(limiter.check_at(start).is_err());

        let later = start + Duration::from_secs(1);
        assert!(limiter.check_at(later).is_ok(), "the bucket refilled");
        assert!(limiter.check_at(later).is_ok());
        assert!(limiter.check_at(later).is_err());
    }

    #[test]
    fn refill_is_capped_at_capacity() {
        // An hour of idleness must not bank an hour of burst.
        let limiter = limiter(&[(5, 5, Duration::from_secs(1))]);
        let start = Instant::now();
        assert!(limiter.check_at(start).is_ok());

        let much_later = start + Duration::from_secs(3600);
        assert!(limiter.check_at(much_later).is_ok());
        assert_eq!(
            limiter.available(),
            vec![4],
            "capacity is 5, so one consumed leaves 4 regardless of how long we idled"
        );
    }

    #[test]
    fn partial_intervals_are_not_discarded() {
        // Advancing `last_fill` to now on every check would throw away the
        // remainder each time and refill slower than configured.
        let limiter = limiter(&[(1, 1, Duration::from_secs(1))]);
        let start = Instant::now();
        assert!(limiter.check_at(start).is_ok());

        // Poll repeatedly inside the interval, then cross it exactly.
        for ms in [100, 300, 600, 900] {
            assert!(limiter.check_at(start + Duration::from_millis(ms)).is_err());
        }
        assert!(
            limiter
                .check_at(start + Duration::from_millis(1000))
                .is_ok(),
            "the token must arrive exactly one interval after the last fill"
        );
    }

    #[test]
    fn every_bucket_must_permit_the_request() {
        // Burst of 10, sustained 1/s. The burst bucket alone would allow ten
        // straight through.
        let limiter = limiter(&[
            (10, 10, Duration::from_secs(10)),
            (1, 1, Duration::from_secs(1)),
        ]);
        let now = Instant::now();

        assert!(limiter.check_at(now).is_ok());
        assert!(
            limiter.check_at(now).is_err(),
            "the sustained bucket refuses even though the burst bucket has room"
        );
    }

    #[test]
    fn a_refused_request_is_not_charged_to_the_bucket_that_allowed_it() {
        let limiter = limiter(&[
            (10, 10, Duration::from_secs(10)),
            (1, 1, Duration::from_secs(1)),
        ]);
        let now = Instant::now();

        assert!(limiter.check_at(now).is_ok());
        assert_eq!(limiter.available(), vec![9, 0]);

        assert!(limiter.check_at(now).is_err());
        assert_eq!(
            limiter.available(),
            vec![9, 0],
            "the refused request must not have spent a token from the roomy bucket"
        );
    }

    #[test]
    fn retry_after_reports_the_longest_wait() {
        let limiter = limiter(&[
            (1, 1, Duration::from_secs(1)),
            (1, 1, Duration::from_secs(60)),
        ]);
        let now = Instant::now();
        assert!(limiter.check_at(now).is_ok());

        let wait = limiter
            .check_at(now + Duration::from_millis(500))
            .expect_err("should be limited");
        assert_eq!(
            wait.seconds(),
            60,
            "coming back when the *shorter* bucket refills would just be refused again"
        );
    }

    #[test]
    fn retry_after_is_never_zero() {
        // `Retry-After: 0` invites an immediate retry, which is the opposite
        // of what a rate limit is asking for.
        assert_eq!(RetryAfter(Duration::from_millis(1)).seconds(), 1);
        assert_eq!(RetryAfter(Duration::ZERO).seconds(), 1);
        assert_eq!(RetryAfter(Duration::from_millis(1500)).seconds(), 2);
    }

    #[test]
    fn token_limits_are_skipped_rather_than_applied_as_requests() {
        // `type: tokens` counts LLM tokens. Treating it as a request limit
        // would silently enforce a completely different policy.
        let config = vec![LocalRateLimit {
            max_tokens: 1000,
            tokens_per_fill: 1000,
            fill_interval: DurationString(Duration::from_secs(60)),
            kind: RateLimitKind::Tokens,
        }];
        let limiter = RateLimiter::new(&config, "test").expect("should compile");
        assert!(
            limiter.is_none(),
            "a token limit is not a request limit and must not stand in for one"
        );
    }

    #[test]
    fn no_limits_means_no_limiter() {
        assert!(
            RateLimiter::new(&[], "test")
                .expect("should compile")
                .is_none()
        );
    }

    #[test]
    fn a_bucket_that_can_never_refill_is_a_config_error() {
        let config = vec![LocalRateLimit {
            max_tokens: 10,
            tokens_per_fill: 0,
            fill_interval: DurationString(Duration::from_secs(1)),
            kind: RateLimitKind::Requests,
        }];
        let err = RateLimiter::new(&config, "route[0]").expect_err("should not compile");
        assert!(err.to_string().contains("route[0]"), "got: {err}");
        assert!(err.to_string().contains("never refills"), "got: {err}");
    }

    #[test]
    fn a_bucket_with_no_capacity_is_a_config_error() {
        let config = vec![LocalRateLimit {
            max_tokens: 0,
            tokens_per_fill: 1,
            fill_interval: DurationString(Duration::from_secs(1)),
            kind: RateLimitKind::Requests,
        }];
        let err = RateLimiter::new(&config, "route[0]").expect_err("should not compile");
        assert!(err.to_string().contains("every request"), "got: {err}");
    }
}
