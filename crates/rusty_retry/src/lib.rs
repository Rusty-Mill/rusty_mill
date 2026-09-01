//! Exponential backoff with jitter, shared by `rusty_request` and
//! `rusty-acp`'s own retry policies.
//!
//! This crate is deliberately narrow: it computes *how long to wait*
//! before a retry. It has no opinion on *whether* to retry -- which
//! statuses/errors are retryable, which HTTP methods or request types are
//! safe to repeat, and how a server's `Retry-After` header is parsed (a
//! plain delta-seconds count parses the same everywhere and lives here;
//! the HTTP-date form doesn't, since callers already pull in different
//! date-parsing dependencies for other reasons). Each caller keeps that
//! policy layer to itself -- `rusty_request`'s is HTTP-method-idempotency
//! based, `rusty-acp`'s is ACP-request-type based, and forcing those to
//! converge would change real behavior neither crate asked for.
//!
//! [`Backoff::Exponential`]'s `jitter` field unifies what were two
//! independent representations before this crate existed:
//! `rusty_request`'s `jitter: bool` (full jitter on/off) and
//! `rusty-acp`'s `jitter: f64` (a randomized fraction of the delay). The
//! `f64` form is a strict generalization -- `0.0` reproduces "no jitter",
//! `1.0` reproduces `rusty_request`'s old "full jitter" (uniform over
//! `[0, capped_delay)`), and anything between reproduces `rusty-acp`'s
//! partial-jitter formula exactly.

use std::time::Duration;

/// How long to wait before the next retry attempt.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Backoff {
    /// The same delay before every retry.
    Fixed(Duration),
    /// `base * 2^attempt`, capped at `max`. `jitter` (clamped to
    /// `0.0..=1.0`) randomizes a fraction of the capped delay: `0.0` is
    /// no jitter, `1.0` randomizes the whole delay (uniform over
    /// `[0, capped)`), and values between interpolate -- `0.5` yields a
    /// delay uniform over `[capped/2, capped)`.
    Exponential {
        base: Duration,
        max: Duration,
        jitter: f64,
    },
}

impl Backoff {
    pub fn fixed(delay: Duration) -> Self {
        Backoff::Fixed(delay)
    }

    pub fn exponential(base: Duration, max: Duration, jitter: f64) -> Self {
        Backoff::Exponential { base, max, jitter }
    }

    /// The delay before retrying after `attempt` prior failures (0 for the
    /// first retry).
    pub fn delay_for(&self, attempt: u32) -> Duration {
        match *self {
            Backoff::Fixed(d) => d,
            Backoff::Exponential { base, max, jitter } => {
                // Exponent capped well below where `1u32 << exp` could
                // overflow -- `.min(max)` below makes any exponent past a
                // handful of attempts equivalent anyway.
                let factor = 1u32 << attempt.min(20);
                let capped = base.checked_mul(factor).unwrap_or(max).min(max);
                let jitter = jitter.clamp(0.0, 1.0);
                if jitter == 0.0 {
                    capped
                } else {
                    let fixed = capped.mul_f64(1.0 - jitter);
                    fixed.saturating_add(capped.mul_f64(jitter * random_fraction()))
                }
            }
        }
    }
}

/// A tiny non-cryptographic random source, used only where the goal is
/// avoiding collisions/lockstep (retry-backoff jitter), never anything
/// security-sensitive. Built from `std`'s own `RandomState` (already
/// randomly seeded from OS randomness per the stdlib docs) perturbed with
/// the current time, rather than pulling in a `rand` crate -- the same
/// technique `rusty_request`'s own (still-used-elsewhere) RNG module
/// applies for its multipart boundary generation.
fn next_u64() -> u64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    use std::time::{SystemTime, UNIX_EPOCH};

    let mut hasher = RandomState::new().build_hasher();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    hasher.write_u128(nanos);
    hasher.finish()
}

/// A uniform random fraction in `[0, 1)`, using the standard 53-bit
/// (`f64` mantissa width) technique.
fn random_fraction() -> f64 {
    (next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
}

/// Parses a `Retry-After` value's delta-seconds form (`Retry-After: 120`),
/// per RFC 9110 §10.2.3. Returns `None` for the alternative HTTP-date
/// form -- callers that need it already have a date parser (each pulls in
/// a different one for other reasons) and should fall back to it.
pub fn retry_after_seconds(value: &str) -> Option<Duration> {
    value.trim().parse::<u64>().ok().map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_backoff_is_constant() {
        let b = Backoff::fixed(Duration::from_millis(50));
        assert_eq!(b.delay_for(0), Duration::from_millis(50));
        assert_eq!(b.delay_for(5), Duration::from_millis(50));
    }

    #[test]
    fn exponential_backoff_without_jitter_doubles_and_caps() {
        let b = Backoff::exponential(Duration::from_millis(100), Duration::from_secs(1), 0.0);
        assert_eq!(b.delay_for(0), Duration::from_millis(100));
        assert_eq!(b.delay_for(1), Duration::from_millis(200));
        assert_eq!(b.delay_for(2), Duration::from_millis(400));
        assert_eq!(b.delay_for(10), Duration::from_secs(1));
    }

    #[test]
    fn full_jitter_never_exceeds_the_cap() {
        let b = Backoff::exponential(Duration::from_millis(100), Duration::from_secs(1), 1.0);
        for attempt in 0..10 {
            assert!(b.delay_for(attempt) <= Duration::from_secs(1));
        }
    }

    // Mirrors rusty-acp's own `jitter_stays_within_the_backoff` -- pinning
    // the exact behavior rusty-acp depends on now that this crate computes
    // it.
    #[test]
    fn half_jitter_stays_within_the_upper_half_of_the_backoff() {
        let b = Backoff::exponential(Duration::from_millis(100), Duration::from_secs(5), 0.5);
        let delays: Vec<_> = (0..64).map(|_| b.delay_for(0)).collect();
        for delay in &delays {
            assert!(
                *delay >= Duration::from_millis(50) && *delay <= Duration::from_millis(100),
                "{delay:?} outside the jittered range"
            );
        }
        assert!(
            delays
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                > 1
        );
    }

    #[test]
    fn no_jitter_is_deterministic() {
        let b = Backoff::exponential(Duration::from_millis(100), Duration::from_secs(5), 0.0);
        assert_eq!(b.delay_for(1), b.delay_for(1));
    }

    #[test]
    fn retry_after_seconds_parses_delta_seconds() {
        assert_eq!(retry_after_seconds("120"), Some(Duration::from_secs(120)));
        assert_eq!(retry_after_seconds(" 5 "), Some(Duration::from_secs(5)));
    }

    #[test]
    fn retry_after_seconds_rejects_dates_and_garbage() {
        assert_eq!(retry_after_seconds("Fri, 31 Dec 1999 23:59:59 GMT"), None);
        assert_eq!(retry_after_seconds("not a number"), None);
    }
}
