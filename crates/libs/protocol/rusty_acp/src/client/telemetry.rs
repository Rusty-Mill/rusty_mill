//! Client metrics, emitted through the [`metrics`] facade.
//!
//! The same shape as the server's: every function is a no-op unless the
//! `metrics` feature is on, and the gate lives *inside* the function rather
//! than at each call site, so the calling code reads the same either way and a
//! metric cannot be emitted on one build and forgotten on another.
//!
//! # What these are for
//!
//! The client's whole job in these paths is to *not* surface something to the
//! caller: it retries with backoff, honours `Retry-After`, reconnects a dropped
//! stream from its last event, and tolerates transient failures while polling.
//! Each of those decisions is invisible by design, which is exactly why they
//! need counting. A caller timing `run_sync` otherwise cannot tell a slow agent
//! from a fast one behind four retries.
//!
//! # Cardinality
//!
//! The same rule the server keeps: **no run ids, and nothing a caller
//! controls**. Labels here are the request method and the retry outcome, both
//! drawn from a fixed set. Run ids belong on spans, which is where they are.
//!
//! [`metrics`]: https://docs.rs/metrics

use std::time::Duration;

/// Requests sent, by HTTP method. Counts attempts, so the difference between
/// this and the server's request count is what the retry policy added.
#[cfg(feature = "metrics")]
pub(crate) const REQUESTS: &str = "acp_client_requests_total";
/// Retries taken, by why the previous attempt was retryable.
#[cfg(feature = "metrics")]
pub(crate) const RETRIES: &str = "acp_client_retries_total";
/// Requests that exhausted the retry policy and failed.
#[cfg(feature = "metrics")]
pub(crate) const RETRIES_EXHAUSTED: &str = "acp_client_retries_exhausted_total";
/// Time spent asleep in backoff rather than waiting on a server.
///
/// The one an operator reaches for first: latency that looks like a slow agent
/// and is not.
#[cfg(feature = "metrics")]
pub(crate) const BACKOFF_SECONDS: &str = "acp_client_backoff_seconds_total";
/// Stream reconnections attempted.
#[cfg(feature = "metrics")]
pub(crate) const RECONNECTS: &str = "acp_client_stream_reconnects_total";
/// Streams that ended because reconnection was exhausted or switched off.
#[cfg(feature = "metrics")]
pub(crate) const STREAMS_ABANDONED: &str = "acp_client_streams_abandoned_total";

/// Register descriptions, so an exporter has units and help text.
///
/// Idempotent, and called from the client builder rather than once globally:
/// per-client costs nothing and needs no initialisation call a caller could
/// forget.
pub(crate) fn describe() {
    #[cfg(feature = "metrics")]
    {
        metrics::describe_counter!(REQUESTS, "ACP requests sent, including retried attempts");
        metrics::describe_counter!(RETRIES, "Requests retried by the client's retry policy");
        metrics::describe_counter!(
            RETRIES_EXHAUSTED,
            "Requests that failed after exhausting the retry policy"
        );
        metrics::describe_counter!(
            BACKOFF_SECONDS,
            metrics::Unit::Seconds,
            "Time spent in retry backoff rather than waiting on a server"
        );
        metrics::describe_counter!(RECONNECTS, "Dropped event streams the client reconnected");
        metrics::describe_counter!(
            STREAMS_ABANDONED,
            "Event streams that ended because reconnection was exhausted or disabled"
        );
    }
}

/// A request attempt was sent.
pub(crate) fn request_sent(method: &str) {
    #[cfg(feature = "metrics")]
    metrics::counter!(REQUESTS, "method" => method.to_string()).increment(1);
    #[cfg(not(feature = "metrics"))]
    let _ = method;
}

/// An attempt is being retried after `delay`.
///
/// `reason` is one of a fixed set — a retryable status or a transport failure —
/// so it stays a label rather than becoming one series per error string.
pub(crate) fn retried(reason: &'static str, delay: Duration) {
    #[cfg(feature = "metrics")]
    {
        metrics::counter!(RETRIES, "reason" => reason).increment(1);
        metrics::counter!(BACKOFF_SECONDS).increment(delay.as_secs_f64() as u64);
        // Sub-second backoffs are the common case, so seconds alone would round
        // most of them to nothing. Recorded as a histogram too, where the
        // exporter can keep the resolution.
        metrics::histogram!("acp_client_backoff_duration_seconds").record(delay.as_secs_f64());
    }
    #[cfg(not(feature = "metrics"))]
    {
        let _ = (reason, delay);
    }
}

/// A request gave up after using every attempt the policy allowed.
pub(crate) fn retries_exhausted(method: &str) {
    #[cfg(feature = "metrics")]
    metrics::counter!(RETRIES_EXHAUSTED, "method" => method.to_string()).increment(1);
    #[cfg(not(feature = "metrics"))]
    let _ = method;
}

/// A dropped stream is being reconnected.
pub(crate) fn stream_reconnected() {
    #[cfg(feature = "metrics")]
    metrics::counter!(RECONNECTS).increment(1);
}

/// A stream ended without reaching its terminal event.
pub(crate) fn stream_abandoned(reason: &'static str) {
    #[cfg(feature = "metrics")]
    metrics::counter!(STREAMS_ABANDONED, "reason" => reason).increment(1);
    #[cfg(not(feature = "metrics"))]
    let _ = reason;
}
