//! Process-wide server metrics (`SERVER-001` FR, ADR-0064,
//! `docs/design/SERVER-METRICS-DESIGN.md`): a small, fixed set of atomic
//! counters, readable over the existing wire as [`crate::server::protocol::Request::Metrics`]
//! (`super::protocol::Request::Metrics`), rendered as Prometheus text
//! exposition format — no new dependency, no second listener. See the
//! design document's own "Non-goals" for what this deliberately does not
//! provide (per-`RequestKind` cardinality, latency histograms, a
//! standalone HTTP endpoint).
//!
//! [`crate::server::metrics::ServerMetrics`] lives on [`crate::server::ServeOptions`] (one instance per
//! `serve`/`serve_tables` call, shared via the same `Arc<ServeOptions>`
//! every connection thread already holds) rather than as a separate
//! parameter — `ServeOptions` already carries live, shared, mutable
//! state across connection threads (`rate_limit`'s `FailureTable` is the
//! precedent), not just configuration, so a second live-state parameter
//! would only duplicate that shape.

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

/// `RLH-FR-001` (ADR-0088): the fixed upper bounds, in seconds, of the
/// request-latency histogram's buckets — sixteen, from 100 µs to 10 s
/// in a 1-2.5-5 progression, plus the implicit `+Inf`. Chosen against
/// `benches/server.rs`'s own numbers: an indexed point read sits in the
/// first two buckets, a 100K full scan around 100–250 ms, and nothing
/// this server answers should take ten seconds. Fixed at compile time
/// (`MET-FR-002`'s bounded cardinality), never a setting.
pub const LATENCY_BUCKETS_SECONDS: [f64; 16] = [
    0.0001, 0.00025, 0.0005, 0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5,
    5.0, 10.0,
];

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
    /// `LIM-FR-002` (ADR-0093): accepts closed at once because the
    /// connection cap was reached.
    connections_refused_total: AtomicU64,
    /// `CLP-FR-002` (ADR-0102): `Query` requests that named no `limit`
    /// and were answered as if they had asked for the row cap.
    query_rows_clamped_total: AtomicU64,
    connections_active: AtomicI64,
    /// `QPM-FR-002`: one counter per [`PlanKind`], indexed by
    /// [`PlanKind::index`].
    plans: [AtomicU64; PlanKind::ALL.len()],
    /// `RLH-FR-002` (ADR-0088): the request-latency histogram — one
    /// non-cumulative count per bucket of [`LATENCY_BUCKETS_SECONDS`]
    /// plus one for `+Inf` (cumulated at render), the observation
    /// count, and the sum in whole microseconds (an integer, so it stays
    /// one atomic add; rendered as seconds).
    latency_buckets: [AtomicU64; LATENCY_BUCKETS_SECONDS.len() + 1],
    latency_count: AtomicU64,
    latency_sum_micros: AtomicU64,
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
            connections_refused_total: AtomicU64::new(0),
            query_rows_clamped_total: AtomicU64::new(0),
            connections_active: AtomicI64::new(0),
            plans: Default::default(),
            latency_buckets: Default::default(),
            latency_count: AtomicU64::new(0),
            latency_sum_micros: AtomicU64::new(0),
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

    /// `LIM-FR-002` (ADR-0093): one accept closed at once because
    /// `ServeOptions::max_connections` was already reached — never
    /// counted in `connections_total`, never opened.
    pub(crate) fn record_connection_refused(&self) {
        self.connections_refused_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// `CLP-FR-002` (ADR-0102): one `Query` with no `limit` answered as
    /// if it had asked for `ServeOptions::max_query_rows`.
    pub(crate) fn record_query_clamped(&self) {
        self.query_rows_clamped_total
            .fetch_add(1, Ordering::Relaxed);
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

    /// `RLH-FR-003` (ADR-0088): one dispatched request took `elapsed`
    /// from its frame's arrival to its response being ready — the same
    /// post-dispatch call site as [`ServerMetrics::record_request`], so
    /// the histogram's count is `requests_total`'s population. Three
    /// atomic adds: the one bucket the observation falls in, the count,
    /// and the sum in microseconds (saturating at `u64::MAX` µs, some
    /// 584,000 years).
    pub(crate) fn record_latency(&self, elapsed: Duration) {
        let seconds = elapsed.as_secs_f64();
        let bucket = LATENCY_BUCKETS_SECONDS
            .iter()
            .position(|le| seconds <= *le)
            .unwrap_or(LATENCY_BUCKETS_SECONDS.len());
        self.latency_buckets[bucket].fetch_add(1, Ordering::Relaxed);
        self.latency_count.fetch_add(1, Ordering::Relaxed);
        let micros = u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX);
        self.latency_sum_micros
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |sum| {
                Some(sum.saturating_add(micros))
            })
            .ok();
    }

    /// `RLH-FR-004`: the histogram's text — cumulative `_bucket` lines
    /// in bound order ending at `+Inf`, then `_sum` (seconds, six
    /// decimals) and `_count`.
    fn render_latency(&self) -> String {
        let mut out = String::new();
        let mut cumulative = 0u64;
        for (i, le) in LATENCY_BUCKETS_SECONDS.iter().enumerate() {
            cumulative += self.latency_buckets[i].load(Ordering::Relaxed);
            out.push_str(&format!(
                "dogserver_request_duration_seconds_bucket{{le=\"{le}\"}} {cumulative}\n"
            ));
        }
        cumulative += self.latency_buckets[LATENCY_BUCKETS_SECONDS.len()].load(Ordering::Relaxed);
        out.push_str(&format!(
            "dogserver_request_duration_seconds_bucket{{le=\"+Inf\"}} {cumulative}\n"
        ));
        let sum = self.latency_sum_micros.load(Ordering::Relaxed);
        out.push_str(&format!(
            "dogserver_request_duration_seconds_sum {}.{:06}\n",
            sum / 1_000_000,
            sum % 1_000_000
        ));
        out.push_str(&format!(
            "dogserver_request_duration_seconds_count {}\n",
            self.latency_count.load(Ordering::Relaxed)
        ));
        out
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
             # HELP dogserver_connections_refused_total Accepts closed at once because the connection cap was reached.\n\
             # TYPE dogserver_connections_refused_total counter\n\
             dogserver_connections_refused_total {}\n\
             # HELP dogserver_query_rows_clamped_total Queries with no limit answered as if they had asked for the row cap.\n\
             # TYPE dogserver_query_rows_clamped_total counter\n\
             dogserver_query_rows_clamped_total {}\n\
             # HELP dogserver_query_plans_total Planned reads answered without an error, by the path taken.\n\
             # TYPE dogserver_query_plans_total counter\n\
             {}\
             # HELP dogserver_request_duration_seconds Seconds from a dispatched request's arrival to its response being ready.\n\
             # TYPE dogserver_request_duration_seconds histogram\n\
             {}\
             # HELP dogserver_uptime_seconds Seconds since this process's `ServeOptions` was built.\n\
             # TYPE dogserver_uptime_seconds counter\n\
             dogserver_uptime_seconds {}\n",
            self.requests_total.load(Ordering::Relaxed),
            self.requests_ok_total.load(Ordering::Relaxed),
            self.requests_err_total.load(Ordering::Relaxed),
            self.connections_total.load(Ordering::Relaxed),
            self.connections_active.load(Ordering::Relaxed),
            self.connections_refused_total.load(Ordering::Relaxed),
            self.query_rows_clamped_total.load(Ordering::Relaxed),
            plans,
            self.render_latency(),
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
        assert!(text.contains("dogserver_connections_refused_total 0\n"));
        assert!(text.contains("dogserver_query_rows_clamped_total 0\n"));
        assert!(text.contains("dogserver_uptime_seconds "));
    }

    /// `LIM-FR-002` (ADR-0093): a refused accept moves only its own
    /// counter — never `connections_total` or the active gauge.
    #[test]
    fn a_refused_connection_counts_only_as_refused() {
        let m = ServerMetrics::new();
        m.record_connection_refused();
        let text = m.render();
        assert!(text.contains("dogserver_connections_refused_total 1\n"));
        assert!(text.contains("dogserver_connections_total 0\n"));
        assert!(text.contains("dogserver_connections_active 0\n"));
    }

    /// `CLP-FR-002` (ADR-0102): a clamped query moves only its own
    /// counter.
    #[test]
    fn a_clamped_query_counts_only_as_clamped() {
        let m = ServerMetrics::new();
        m.record_query_clamped();
        let text = m.render();
        assert!(text.contains("dogserver_query_rows_clamped_total 1\n"));
        assert!(text.contains("dogserver_requests_total 0\n"));
        assert!(text.contains("dogserver_connections_refused_total 0\n"));
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

    /// `RLH-FR-001`–`004` (ADR-0088): the buckets are ascending and
    /// fixed; an observation lands in the first bucket whose bound it
    /// does not exceed and the render cumulates them, ending at `+Inf`
    /// with the count; `_sum` is the seconds to six decimals; a fresh
    /// instance renders every line at zero; the histogram sits before
    /// `uptime_seconds`, which stays the last line.
    #[test]
    fn latency_histogram_cumulates_fixed_buckets_and_keeps_uptime_last() {
        assert!(LATENCY_BUCKETS_SECONDS.windows(2).all(|w| w[0] < w[1]));
        let m = ServerMetrics::new();
        let text = m.render();
        assert!(text.contains("dogserver_request_duration_seconds_bucket{le=\"0.0001\"} 0\n"));
        assert!(text.contains("dogserver_request_duration_seconds_bucket{le=\"+Inf\"} 0\n"));
        assert!(text.contains("dogserver_request_duration_seconds_sum 0.000000\n"));
        assert!(text.contains("dogserver_request_duration_seconds_count 0\n"));
        m.record_latency(Duration::from_micros(50)); // <= 0.0001
        m.record_latency(Duration::from_micros(100)); // <= 0.0001 (inclusive)
        m.record_latency(Duration::from_micros(300)); // <= 0.0005
        m.record_latency(Duration::from_millis(30)); // <= 0.05
        m.record_latency(Duration::from_secs(11)); // +Inf only
        let text = m.render();
        assert!(text.contains("dogserver_request_duration_seconds_bucket{le=\"0.0001\"} 2\n"));
        assert!(text.contains("dogserver_request_duration_seconds_bucket{le=\"0.00025\"} 2\n"));
        assert!(text.contains("dogserver_request_duration_seconds_bucket{le=\"0.0005\"} 3\n"));
        assert!(text.contains("dogserver_request_duration_seconds_bucket{le=\"0.025\"} 3\n"));
        assert!(text.contains("dogserver_request_duration_seconds_bucket{le=\"0.05\"} 4\n"));
        assert!(text.contains("dogserver_request_duration_seconds_bucket{le=\"10\"} 4\n"));
        assert!(text.contains("dogserver_request_duration_seconds_bucket{le=\"+Inf\"} 5\n"));
        assert!(text.contains("dogserver_request_duration_seconds_sum 11.030450\n"));
        assert!(text.contains("dogserver_request_duration_seconds_count 5\n"));
        assert_eq!(
            text.matches("# TYPE dogserver_request_duration_seconds histogram\n")
                .count(),
            1
        );
        assert!(
            text.rfind("dogserver_uptime_seconds ").unwrap()
                > text
                    .rfind("dogserver_request_duration_seconds_count")
                    .unwrap(),
            "uptime stays the last line"
        );
    }

    #[test]
    fn render_is_deterministic() {
        let m = ServerMetrics::new();
        m.record_request(true);
        assert_eq!(m.render(), m.render());
    }
}
