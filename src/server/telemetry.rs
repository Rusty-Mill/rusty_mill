//! Metrics, emitted through the [`metrics`] facade.
//!
//! Every function here is a no-op unless the `metrics` feature is on, and the
//! gate lives *inside* the function rather than at each call site. That keeps
//! the execution path readable — `telemetry::run_started(&agent)` says what it
//! does whether or not anyone is listening — and means a metric can never be
//! emitted on one build and forgotten on another.
//!
//! # Why a facade
//!
//! [`metrics`] records; it does not export. Whichever exporter an operator
//! installs — Prometheus, OpenTelemetry, statsd — receives these, and installing
//! none costs an atomic load per call. That matches how the crate treats the
//! rest of its edges: the router is a plain `axum::Router` so middleware is the
//! operator's choice, and this is the same bargain for telemetry.
//!
//! # Cardinality
//!
//! Labels here are bounded by things an operator configures: the agent name,
//! the terminal status, the store operation. **A run id must never become a
//! label** — one time series per run would sink any backend. Run ids belong on
//! spans, which is where they are.
//!
//! [`metrics`]: https://docs.rs/metrics

use std::time::Duration;

use crate::types::RunStatus;

/// Runs currently executing on this replica, by agent.
#[cfg(feature = "metrics")]
pub(crate) const RUNS_IN_FLIGHT: &str = "acp_runs_in_flight";
/// Runs that reached a terminal state, by agent and status.
#[cfg(feature = "metrics")]
pub(crate) const RUNS_TOTAL: &str = "acp_runs_total";
/// How long runs took to reach a terminal state, by agent and status.
#[cfg(feature = "metrics")]
pub(crate) const RUN_DURATION: &str = "acp_run_duration_seconds";
/// Lease renewals that failed. Several must be missed before a lease lapses,
/// so a nonzero rate here is a warning rather than an outage.
#[cfg(feature = "metrics")]
pub(crate) const LEASE_RENEW_FAILURES: &str = "acp_lease_renew_failures_total";
/// Runs failed because their executing replica stopped renewing.
#[cfg(feature = "metrics")]
pub(crate) const RUNS_REAPED: &str = "acp_runs_reaped_total";
/// Attempts to claim an abandoned run, by whether this replica won.
#[cfg(feature = "metrics")]
pub(crate) const RECOVERY_CLAIMS: &str = "acp_recovery_claims_total";
/// Replacement runs started for a recoverable run.
#[cfg(feature = "metrics")]
pub(crate) const RECOVERIES_STARTED: &str = "acp_recoveries_started_total";
/// Abandoned runs left failed because their attempt budget was spent.
#[cfg(feature = "metrics")]
pub(crate) const RECOVERY_EXHAUSTED: &str = "acp_recovery_exhausted_total";
/// Store operations, by operation name.
#[cfg(feature = "metrics")]
pub(crate) const STORE_OPERATION_DURATION: &str = "acp_store_operation_duration_seconds";
/// Store operations that returned an error, by operation name.
#[cfg(feature = "metrics")]
pub(crate) const STORE_FAILURES: &str = "acp_store_failures_total";
/// Runs executing an agent body right now, excluding those parked awaiting a
/// client. This is what `max_concurrent_runs` is measured against.
#[cfg(feature = "metrics")]
pub(crate) const RUNS_EXECUTING: &str = "acp_runs_executing";
/// Run submissions refused because this replica was at capacity.
///
/// **Unlabelled, deliberately.** The obvious label would be the agent name, but
/// a submission is refused at the door — before the agent is looked up — so the
/// name is whatever the caller sent. Labelling it would let anyone mint an
/// unbounded number of time series by posting nonsense, which is a worse
/// problem than the one the label would solve.
#[cfg(feature = "metrics")]
pub(crate) const RUNS_REJECTED: &str = "acp_runs_rejected_total";

/// Register descriptions and units with whatever recorder is installed.
///
/// Called once when a server is built. Exporters that render help text — the
/// Prometheus one, for instance — show nothing useful without this, and a
/// metric an operator cannot interpret is barely better than no metric.
pub(crate) fn describe() {
    #[cfg(feature = "metrics")]
    {
        metrics::describe_gauge!(
            RUNS_IN_FLIGHT,
            "Runs currently executing on this replica, by agent"
        );
        metrics::describe_counter!(
            RUNS_TOTAL,
            "Runs that reached a terminal state, by agent and status"
        );
        metrics::describe_histogram!(
            RUN_DURATION,
            metrics::Unit::Seconds,
            "Time from run creation to a terminal state"
        );
        metrics::describe_counter!(
            LEASE_RENEW_FAILURES,
            "Lease renewals that failed; several must be missed before a lease lapses"
        );
        metrics::describe_counter!(
            RUNS_REAPED,
            "Runs failed because their executing replica stopped renewing its lease"
        );
        metrics::describe_counter!(
            RECOVERY_CLAIMS,
            "Attempts to claim an abandoned run, by whether this replica won"
        );
        metrics::describe_counter!(
            RECOVERIES_STARTED,
            "Replacement runs started for an abandoned but recoverable run"
        );
        metrics::describe_counter!(
            RECOVERY_EXHAUSTED,
            "Abandoned runs left failed because their recovery attempt budget was spent"
        );
        metrics::describe_histogram!(
            STORE_OPERATION_DURATION,
            metrics::Unit::Seconds,
            "Store operation latency, by operation"
        );
        metrics::describe_counter!(STORE_FAILURES, "Store operations that returned an error");
        metrics::describe_gauge!(
            RUNS_EXECUTING,
            "Runs executing an agent body on this replica, excluding those awaiting a client"
        );
        metrics::describe_counter!(
            RUNS_REJECTED,
            "Run submissions refused because this replica was at its concurrency ceiling"
        );
    }
}

/// This replica is now running `executing` agent bodies.
///
/// Set rather than incremented: the count already exists as one number under a
/// lock, and deriving a gauge from paired increments would let a missed
/// decrement — a run that ends on a path nobody thought about — drift the gauge
/// away from the value the admission check actually uses. An operator tuning a
/// ceiling against a lying gauge is worse served than one with no gauge.
pub(crate) fn runs_executing(executing: usize) {
    #[cfg(feature = "metrics")]
    metrics::gauge!(RUNS_EXECUTING).set(executing as f64);
    #[cfg(not(feature = "metrics"))]
    let _ = executing;
}

/// A run submission was refused for want of capacity.
pub(crate) fn run_rejected() {
    #[cfg(feature = "metrics")]
    metrics::counter!(RUNS_REJECTED).increment(1);
}

/// A run began executing on this replica.
pub(crate) fn run_started(agent: &str) {
    #[cfg(feature = "metrics")]
    metrics::gauge!(RUNS_IN_FLIGHT, "agent" => agent.to_string()).increment(1.0);
    #[cfg(not(feature = "metrics"))]
    let _ = agent;
}

/// A run reached a terminal state, having run for `elapsed`.
pub(crate) fn run_finished(agent: &str, status: RunStatus, elapsed: Option<Duration>) {
    #[cfg(feature = "metrics")]
    {
        metrics::gauge!(RUNS_IN_FLIGHT, "agent" => agent.to_string()).decrement(1.0);
        metrics::counter!(
            RUNS_TOTAL,
            "agent" => agent.to_string(),
            "status" => status.to_string(),
        )
        .increment(1);
        if let Some(elapsed) = elapsed {
            metrics::histogram!(
                RUN_DURATION,
                "agent" => agent.to_string(),
                "status" => status.to_string(),
            )
            .record(elapsed.as_secs_f64());
        }
    }
    #[cfg(not(feature = "metrics"))]
    let _ = (agent, status, elapsed);
}

/// A lease renewal failed.
pub(crate) fn lease_renew_failed() {
    #[cfg(feature = "metrics")]
    metrics::counter!(LEASE_RENEW_FAILURES).increment(1);
}

/// This replica tried to claim an abandoned run.
pub(crate) fn recovery_claim(won: bool) {
    #[cfg(feature = "metrics")]
    metrics::counter!(RECOVERY_CLAIMS, "outcome" => if won { "won" } else { "lost" }).increment(1);
    #[cfg(not(feature = "metrics"))]
    let _ = won;
}

/// An abandoned run was failed.
pub(crate) fn run_reaped(agent: &str) {
    #[cfg(feature = "metrics")]
    metrics::counter!(RUNS_REAPED, "agent" => agent.to_string()).increment(1);
    #[cfg(not(feature = "metrics"))]
    let _ = agent;
}

/// A replacement run was started for an abandoned one.
pub(crate) fn recovery_started(agent: &str) {
    #[cfg(feature = "metrics")]
    metrics::counter!(RECOVERIES_STARTED, "agent" => agent.to_string()).increment(1);
    #[cfg(not(feature = "metrics"))]
    let _ = agent;
}

/// An abandoned run was not replaced because its attempt budget was spent.
pub(crate) fn recovery_exhausted(agent: &str) {
    #[cfg(feature = "metrics")]
    metrics::counter!(RECOVERY_EXHAUSTED, "agent" => agent.to_string()).increment(1);
    #[cfg(not(feature = "metrics"))]
    let _ = agent;
}

/// A store operation finished.
///
/// Gated with the emitting: its only caller is `MeteredStore`, which is itself
/// behind this feature.
#[cfg(feature = "metrics")]
pub(crate) fn store_operation(operation: &'static str, elapsed: Duration, failed: bool) {
    metrics::histogram!(STORE_OPERATION_DURATION, "operation" => operation)
        .record(elapsed.as_secs_f64());
    if failed {
        metrics::counter!(STORE_FAILURES, "operation" => operation).increment(1);
    }
}
