//! Process-wide server metrics (`SERVER-001` FR, ADR-0064,
//! `docs/design/SERVER-METRICS-DESIGN.md`): a small, fixed set of atomic
//! counters, readable over the existing wire as [`Request::Metrics`]
//! (`super::protocol::Request::Metrics`), rendered as Prometheus text
//! exposition format — no new dependency, no second listener. See the
//! design document's own "Non-goals" for what this deliberately does not
//! provide (per-`RequestKind` cardinality, latency histograms, a
//! standalone HTTP endpoint).
//!
//! [`ServerMetrics`] lives on [`super::ServeOptions`] (one instance per
//! `serve`/`serve_tables` call, shared via the same `Arc<ServeOptions>`
//! every connection thread already holds) rather than as a separate
//! parameter — `ServeOptions` already carries live, shared, mutable
//! state across connection threads (`rate_limit`'s `FailureTable` is the
//! precedent), not just configuration, so a second live-state parameter
//! would only duplicate that shape.

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::SystemTime;

/// Six atomics, never growing with new `Request` variants — deliberately
/// bounded cardinality, `MET-FR-002`. Every field is incremented from
/// exactly one call site each (`handle_connection`'s connection-open/
/// close and its one post-dispatch access-log call site, `MET-FR-003`),
/// so no ordering between fields is guaranteed beyond
/// `requests_total == requests_ok_total + requests_err_total` — the one
/// invariant this type's own tests check.
#[derive(Debug)]
pub struct ServerMetrics {
    requests_total: AtomicU64,
    requests_ok_total: AtomicU64,
    requests_err_total: AtomicU64,
    connections_total: AtomicU64,
    connections_active: AtomicI64,
    started_at: SystemTime,
}

impl Default for ServerMetrics {
    /// Stamps `started_at` at construction time — once per `serve`/
    /// `serve_tables` call, since `ServeOptions::new`/`from_env` each
    /// build exactly one.
    fn default() -> Self {
        Self {
            requests_total: AtomicU64::new(0),
            requests_ok_total: AtomicU64::new(0),
            requests_err_total: AtomicU64::new(0),
            connections_total: AtomicU64::new(0),
            connections_active: AtomicI64::new(0),
            started_at: SystemTime::now(),
        }
    }
}

impl ServerMetrics {
    pub fn new() -> Self {
        Self::default()
    }

    /// One accepted connection — `handle_connection`'s own admission
    /// point, right after the `Admitted` audit event.
    pub(crate) fn record_connection_opened(&self) {
        self.connections_total.fetch_add(1, Ordering::Relaxed);
        self.connections_active.fetch_add(1, Ordering::Relaxed);
    }

    /// The matching close — always called exactly once per
    /// [`ServerMetrics::record_connection_opened`], via
    /// [`ConnectionMetricsGuard`]'s `Drop`.
    fn record_connection_closed(&self) {
        self.connections_active.fetch_sub(1, Ordering::Relaxed);
    }

    /// One dispatched request reaching the same call site
    /// [`super::access::AccessEvent`] is recorded at — `MET-FR-003`.
    pub(crate) fn record_request(&self, ok: bool) {
        self.requests_total.fetch_add(1, Ordering::Relaxed);
        if ok {
            self.requests_ok_total.fetch_add(1, Ordering::Relaxed);
        } else {
            self.requests_err_total.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// `MET-FR-004`: Prometheus text exposition format — one `# HELP`/
    /// `# TYPE` pair per metric, then one sample line, fields in a fixed,
    /// tested order. `uptime_seconds` is computed fresh on every call
    /// from `started_at`, not itself an atomic.
    pub fn render(&self) -> String {
        let uptime = SystemTime::now()
            .duration_since(self.started_at)
            .unwrap_or_default()
            .as_secs();
        format!(
            "# HELP dogserver_requests_total Total requests dispatched.\n\
             # TYPE dogserver_requests_total counter\n\
             dogserver_requests_total {}\n\
             # HELP dogserver_requests_ok_total Requests answered without an error code.\n\
             # TYPE dogserver_requests_ok_total counter\n\
             dogserver_requests_ok_total {}\n\
             # HELP dogserver_requests_err_total Requests answered with an error code.\n\
             # TYPE dogserver_requests_err_total counter\n\
             dogserver_requests_err_total {}\n\
             # HELP dogserver_connections_total Total connections accepted.\n\
             # TYPE dogserver_connections_total counter\n\
             dogserver_connections_total {}\n\
             # HELP dogserver_connections_active Connections currently open.\n\
             # TYPE dogserver_connections_active gauge\n\
             dogserver_connections_active {}\n\
             # HELP dogserver_uptime_seconds Seconds since this process's `ServeOptions` was built.\n\
             # TYPE dogserver_uptime_seconds counter\n\
             dogserver_uptime_seconds {}\n",
            self.requests_total.load(Ordering::Relaxed),
            self.requests_ok_total.load(Ordering::Relaxed),
            self.requests_err_total.load(Ordering::Relaxed),
            self.connections_total.load(Ordering::Relaxed),
            self.connections_active.load(Ordering::Relaxed),
            uptime,
        )
    }
}

/// `handle_connection`'s own connection-close counterpart to
/// [`ServerMetrics::record_connection_opened`] — the `DisconnectAudit`
/// precedent (`ADR-0029`): a drop guard, not a per-`return` call, so
/// every one of `handle_connection`'s several return paths decrements
/// exactly once.
pub(crate) struct ConnectionMetricsGuard<'a>(pub(crate) &'a ServerMetrics);

impl Drop for ConnectionMetricsGuard<'_> {
    fn drop(&mut self) {
        self.0.record_connection_closed();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_metrics_render_all_zero_and_a_real_uptime() {
        let m = ServerMetrics::new();
        let text = m.render();
        assert!(text.contains("dogserver_requests_total 0\n"));
        assert!(text.contains("dogserver_requests_ok_total 0\n"));
        assert!(text.contains("dogserver_requests_err_total 0\n"));
        assert!(text.contains("dogserver_connections_total 0\n"));
        assert!(text.contains("dogserver_connections_active 0\n"));
        assert!(text.contains("dogserver_uptime_seconds "));
    }

    #[test]
    fn requests_total_is_always_the_ok_err_split() {
        let m = ServerMetrics::new();
        m.record_request(true);
        m.record_request(true);
        m.record_request(false);
        let text = m.render();
        assert!(text.contains("dogserver_requests_total 3\n"));
        assert!(text.contains("dogserver_requests_ok_total 2\n"));
        assert!(text.contains("dogserver_requests_err_total 1\n"));
    }

    #[test]
    fn connection_guard_drop_always_decrements_active() {
        let m = ServerMetrics::new();
        m.record_connection_opened();
        m.record_connection_opened();
        assert!(m.render().contains("dogserver_connections_active 2\n"));
        assert!(m.render().contains("dogserver_connections_total 2\n"));
        {
            let _guard = ConnectionMetricsGuard(&m);
        }
        assert!(m.render().contains("dogserver_connections_active 1\n"));
        // `connections_total` is monotonic — never decremented.
        assert!(m.render().contains("dogserver_connections_total 2\n"));
    }

    #[test]
    fn render_is_deterministic() {
        let m = ServerMetrics::new();
        m.record_request(true);
        assert_eq!(m.render(), m.render());
    }
}
