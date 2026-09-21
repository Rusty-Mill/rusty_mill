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

/// Six atomics plus one fixed family of [`PlanKind::ALL`] atomics
/// (`QPM-FR-002`, ADR-0086), never growing with new `Request` variants
/// — deliberately bounded cardinality, `MET-FR-002`. Every field is
/// incremented from exactly one call site each (`handle_connection`'s
/// connection-open/close and its one post-dispatch access-log call
/// site, `MET-FR-003`; the plan family from that same site, once per
/// planned read answered without an error), so no ordering between
/// fields is guaranteed beyond `requests_total == requests_ok_total +
/// requests_err_total` — the one invariant this type's own tests check.
#[derive(Debug)]
pub struct ServerMetrics {
    requests_total: AtomicU64,
    requests_ok_total: AtomicU64,
    requests_err_total: AtomicU64,
    connections_total: AtomicU64,
    connections_active: AtomicI64,
    /// `QPM-FR-002`: one counter per [`PlanKind`], indexed by
    /// [`PlanKind::index`].
    plans: [AtomicU64; PlanKind::ALL.len()],
    started_at: SystemTime,
}

/// `QPM-FR-001` (ADR-0086): the path a planned read took — the
/// `plan="…"` label of `dogserver_query_plans_total`. The four
/// `QueryPlan` variants (`ADR-0073`/`0075`/`0078`) plus the three
/// walk-only paths that answer before any candidate step
/// (`ADR-0076`/`0081`/`0082`). Classified by `serve::plan_of`, a pure
/// function of the request and the adapter's declarations; recorded
/// once per read answered without an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanKind {
    FullScan,
    IndexEq,
    IndexRange,
    IndexIntersect,
    BoundedWalk,
    CountedWalk,
    KeyedWalk,
}

impl PlanKind {
    /// Every kind, in render order.
    pub const ALL: [PlanKind; 7] = [
        PlanKind::FullScan,
        PlanKind::IndexEq,
        PlanKind::IndexRange,
        PlanKind::IndexIntersect,
        PlanKind::BoundedWalk,
        PlanKind::CountedWalk,
        PlanKind::KeyedWalk,
    ];

    /// The `plan` label value — stable, snake_case, part of the
    /// rendered text.
    pub fn label(self) -> &'static str {
        match self {
            PlanKind::FullScan => "full_scan",
            PlanKind::IndexEq => "index_eq",
            PlanKind::IndexRange => "index_range",
            PlanKind::IndexIntersect => "index_intersect",
            PlanKind::BoundedWalk => "bounded_walk",
            PlanKind::CountedWalk => "counted_walk",
            PlanKind::KeyedWalk => "keyed_walk",
        }
    }

    fn index(self) -> usize {
        match self {
            PlanKind::FullScan => 0,
            PlanKind::IndexEq => 1,
            PlanKind::IndexRange => 2,
            PlanKind::IndexIntersect => 3,
            PlanKind::BoundedWalk => 4,
            PlanKind::CountedWalk => 5,
            PlanKind::KeyedWalk => 6,
        }
    }
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
            plans: Default::default(),
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
    /// `QPM-FR-003` (ADR-0086): one planned read took `kind` and was
    /// answered without an error — the same post-dispatch call site as
    /// [`ServerMetrics::record_request`].
    pub(crate) fn record_plan(&self, kind: PlanKind) {
        self.plans[kind.index()].fetch_add(1, Ordering::Relaxed);
    }

    pub fn render(&self) -> String {
        let uptime = SystemTime::now()
            .duration_since(self.started_at)
            .unwrap_or_default()
            .as_secs();
        let plans: String = PlanKind::ALL
            .iter()
            .map(|kind| {
                format!(
                    "dogserver_query_plans_total{{plan=\"{}\"}} {}\n",
                    kind.label(),
                    self.plans[kind.index()].load(Ordering::Relaxed)
                )
            })
            .collect();
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
             # HELP dogserver_query_plans_total Planned reads answered without an error, by the path taken.\n\
             # TYPE dogserver_query_plans_total counter\n\
             {}\
             # HELP dogserver_uptime_seconds Seconds since this process's `ServeOptions` was built.\n\
             # TYPE dogserver_uptime_seconds counter\n\
             dogserver_uptime_seconds {}\n",
            self.requests_total.load(Ordering::Relaxed),
            self.requests_ok_total.load(Ordering::Relaxed),
            self.requests_err_total.load(Ordering::Relaxed),
            self.connections_total.load(Ordering::Relaxed),
            self.connections_active.load(Ordering::Relaxed),
            plans,
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

    /// `QPM-FR-002`/`QPM-FR-003` (ADR-0086): every kind renders at zero
    /// on a fresh instance, each `record_plan` moves exactly its own
    /// label, and the family sits between `connections_active` and
    /// `uptime_seconds` (still the last line) under one `HELP`/`TYPE`
    /// pair.
    #[test]
    fn plan_family_renders_every_kind_and_counts_each_on_its_own_label() {
        let m = ServerMetrics::new();
        let text = m.render();
        for kind in PlanKind::ALL {
            assert!(
                text.contains(&format!(
                    "dogserver_query_plans_total{{plan=\"{}\"}} 0\n",
                    kind.label()
                )),
                "{kind:?} in:\n{text}"
            );
        }
        assert_eq!(
            text.matches("# TYPE dogserver_query_plans_total counter\n")
                .count(),
            1
        );
        assert!(
            text.rfind("dogserver_uptime_seconds ").unwrap()
                > text.rfind("dogserver_query_plans_total").unwrap(),
            "uptime stays the last line"
        );
        m.record_plan(PlanKind::IndexRange);
        m.record_plan(PlanKind::IndexRange);
        m.record_plan(PlanKind::KeyedWalk);
        let text = m.render();
        assert!(text.contains("dogserver_query_plans_total{plan=\"index_range\"} 2\n"));
        assert!(text.contains("dogserver_query_plans_total{plan=\"keyed_walk\"} 1\n"));
        assert!(text.contains("dogserver_query_plans_total{plan=\"full_scan\"} 0\n"));
        // The labels are distinct and stable.
        let labels: std::collections::BTreeSet<&str> =
            PlanKind::ALL.iter().map(|k| k.label()).collect();
        assert_eq!(labels.len(), PlanKind::ALL.len());
    }

    #[test]
    fn render_is_deterministic() {
        let m = ServerMetrics::new();
        m.record_request(true);
        assert_eq!(m.render(), m.render());
    }
}
