//! Request and task metrics.
//!
//! Traces answer "what happened in this one request" — they are what you read
//! *after* being paged. Metrics are what page you. A request rate, an error
//! rate and a p99 per method are not cheap to recover from spans here, by
//! design: sampling is parent-based, so the spans this server records are the
//! ones its *callers* chose to sample, which is not a representative sample of
//! this server's own traffic.
//!
//! ```no_run
//! use std::sync::Arc;
//! use rusty_mcp::otel::{OtelConfig, metrics::McpMetricsLayer};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let guard = rusty_mcp::otel::init(OtelConfig::new("my-mcp-server"), "info")?;
//! let instruments = guard.instruments().expect("metrics are enabled");
//!
//! // Tool names are the only request-derived label, and only these ones.
//! let layer = McpMetricsLayer::new(Arc::clone(&instruments))
//!     .with_known_names(["add", "divide", "slugify"]);
//! # let _ = layer;
//! # Ok(())
//! # }
//! ```
//!
//! # Cardinality is the thing to get right
//!
//! A metrics backend does not degrade gracefully when a label set explodes; it
//! falls over, and it takes the rest of your metrics with it. Every attribute
//! recorded here comes from a **closed set fixed before any request arrives**:
//!
//! - `mcp.method` is matched against the methods the spec defines. Anything
//!   else is recorded as `other`.
//! - `mcp.name` is recorded only for `tools/call` and `prompts/get`, and only
//!   when it appears in the set passed to
//!   [`McpMetricsLayer::with_known_names`]. Anything else is `other`.
//! - `resources/read` carries the **URI** in `Mcp-Name`, which is unbounded.
//!   It is never used as a label. Neither are task ids.
//!
//! Without the allow-list a client could mint labels by calling tools that do
//! not exist — the calls fail, but the labels are recorded before anyone knows
//! that.
//!
//! # What `outcome` can and cannot tell you
//!
//! The layer sees HTTP, so `outcome` is the transport-level result. A tool that
//! returns `isError: true` is still a *successful* HTTP request and counts as
//! `ok` here — JSON-RPC puts application errors in a `200` body, and a
//! middleware that does not parse bodies cannot see them. Count those in the
//! tool itself if you need them.

use std::{
    convert::Infallible,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Instant,
};

use axum::response::{IntoResponse, Response};
use http::Request;
use rusty_http::StatusCode;

use super::instrument::{Counter, Histogram, Meta, Registry, UpDownCounter};
use super::labels::{
    self, HEADER_MCP_METHOD, HEADER_MCP_NAME, KnownNames, decode_header, in_flight_labels,
    request_labels, task_labels,
};
pub use super::labels::{Outcome, TaskOutcome};

/// Latency buckets, in seconds.
///
/// Weighted towards the fast end: an MCP server that is behaving spends most of
/// its time in the low milliseconds, and buckets spread evenly to ten seconds
/// would put nearly every request in the first one.
const LATENCY_BUCKETS: &[f64] = &[
    0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

/// The instruments this crate records to.
///
/// Built by the pipeline and shared by `Arc`: hand [`OtelGuard::instruments`]
/// to the [`McpMetricsLayer`]. Recording is atomic adds; the pipeline's push
/// thread reads the same values and sends them to the collector.
///
/// [`OtelGuard::instruments`]: super::OtelGuard::instruments
pub struct Instruments {
    requests: Arc<Counter>,
    duration: Arc<Histogram>,
    in_flight: Arc<UpDownCounter>,
    tasks_started: Arc<Counter>,
    tasks_finished: Arc<Counter>,
    registry: Arc<Registry>,
}

impl std::fmt::Debug for Instruments {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Instruments").finish_non_exhaustive()
    }
}

impl Instruments {
    /// Define the instruments and register them for export.
    pub(crate) fn new() -> Self {
        let registry = Arc::new(Registry::default());
        let requests = Arc::new(Counter::new(Meta {
            name: "mcp.server.requests",
            description: "MCP requests handled, by method and outcome.",
            unit: "{request}",
        }));
        let duration = Arc::new(Histogram::new(
            Meta {
                name: "mcp.server.request.duration",
                description: "Time to handle an MCP request.",
                unit: "s",
            },
            LATENCY_BUCKETS,
        ));
        let in_flight = Arc::new(UpDownCounter::new(Meta {
            name: "mcp.server.requests.in_flight",
            description: "MCP requests currently being handled.",
            unit: "{request}",
        }));
        let tasks_started = Arc::new(Counter::new(Meta {
            name: "mcp.server.tasks.started",
            description: "Tasks spawned under the tasks extension.",
            unit: "{task}",
        }));
        let tasks_finished = Arc::new(Counter::new(Meta {
            name: "mcp.server.tasks.finished",
            description: "Tasks that settled, by outcome.",
            unit: "{task}",
        }));

        registry.register(&requests);
        registry.register(&duration);
        registry.register(&in_flight);
        registry.register(&tasks_started);
        registry.register(&tasks_finished);

        Self {
            requests,
            duration,
            in_flight,
            tasks_started,
            tasks_finished,
            registry,
        }
    }

    /// What the push thread reads.
    pub(crate) fn registry(&self) -> &Arc<Registry> {
        &self.registry
    }

    /// A request has started. Pair with [`Instruments::request_finished`].
    pub fn request_started(&self, method: &str) {
        self.in_flight.add(1, &in_flight_labels(method));
    }

    /// A request has finished.
    pub fn request_finished(
        &self,
        method: &str,
        name: Option<&str>,
        outcome: Outcome,
        seconds: f64,
    ) {
        self.in_flight.add(-1, &in_flight_labels(method));
        self.duration
            .record(seconds, &labels::duration_labels(method, name));
        self.requests.add(1, &request_labels(method, name, outcome));
    }

    /// A task was spawned.
    pub fn task_started(&self) {
        self.tasks_started.add(1, &[]);
    }

    /// A task settled.
    pub fn task_finished(&self, outcome: TaskOutcome) {
        self.tasks_finished.add(1, &task_labels(outcome));
    }

    /// Several tasks were abandoned at once, as at the end of a drain.
    pub fn tasks_abandoned(&self, count: usize) {
        if count > 0 {
            self.tasks_finished
                .add(count as u64, &task_labels(TaskOutcome::Abandoned));
        }
    }
}

/// Records request metrics for the service it wraps.
///
/// Mount it on the MCP endpoint. Labels come from the SEP-2243 `Mcp-Method` and
/// `Mcp-Name` headers, so no request body is ever parsed.
///
/// This covers the **HTTP transport only**. A stdio server is a single process
/// serving a single client over a pipe, where there is no middleware position
/// to occupy — and rather less to be told, since one client cannot generate the
/// traffic pattern these metrics exist to describe.
#[derive(Debug, Clone)]
pub struct McpMetricsLayer {
    instruments: Arc<Instruments>,
    known_names: Arc<KnownNames>,
}

impl McpMetricsLayer {
    /// Record to `instruments`, with no tool or prompt names labelled.
    pub fn new(instruments: Arc<Instruments>) -> Self {
        Self {
            instruments,
            known_names: Arc::new(KnownNames::default()),
        }
    }

    /// Names that may appear in the `mcp.name` label.
    ///
    /// Pass the tool and prompt names this server actually serves. Anything
    /// else — including a call to a tool that does not exist — is labelled
    /// `other`, which is what keeps a client from minting unbounded labels.
    pub fn with_known_names(mut self, names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.known_names = Arc::new(KnownNames::new(names));
        self
    }
}

impl<S> tower_layer::Layer<S> for McpMetricsLayer {
    type Service = McpMetrics<S>;

    fn layer(&self, inner: S) -> Self::Service {
        McpMetrics {
            inner,
            instruments: Arc::clone(&self.instruments),
            known_names: Arc::clone(&self.known_names),
        }
    }
}

/// Service produced by [`McpMetricsLayer`].
#[derive(Debug, Clone)]
pub struct McpMetrics<S> {
    inner: S,
    instruments: Arc<Instruments>,
    known_names: Arc<KnownNames>,
}

impl<S, ReqBody> tower_service::Service<Request<ReqBody>> for McpMetrics<S>
where
    S: tower_service::Service<Request<ReqBody>, Error = Infallible> + Clone + Send + 'static,
    S::Response: IntoResponse,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
{
    type Response = Response;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Response, Infallible>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request<ReqBody>) -> Self::Future {
        // Swap in the readied service: `self.inner` is the one that passed
        // `poll_ready`, and the fresh clone may not be ready yet.
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        let method = method_of(&request);
        let name = self
            .known_names
            .name_label(&method, text_header(&request, HEADER_MCP_NAME));
        let instruments = Arc::clone(&self.instruments);

        Box::pin(async move {
            instruments.request_started(&method);
            let started = Instant::now();

            let response = inner.call(request).await?.into_response();

            instruments.request_finished(
                &method,
                name.as_deref(),
                Outcome::from_status(StatusCode::from_u16(response.status().as_u16())),
                started.elapsed().as_secs_f64(),
            );

            Ok(response)
        })
    }
}

/// The request's MCP method, decoded if `rmcp` base64-wrapped it; empty if absent
/// or unreadable (which labels as `other`).
fn method_of<ReqBody>(request: &Request<ReqBody>) -> String {
    text_header(request, HEADER_MCP_METHOD)
        .and_then(decode_header)
        .unwrap_or_default()
}

/// A header value as text, if it has one.
fn text_header<'a, ReqBody>(request: &'a Request<ReqBody>, name: &str) -> Option<&'a str> {
    request.headers().get(name)?.to_str().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_method_is_decoded_when_wrapped_and_empty_when_unreadable() {
        let wrapped = format!(
            "=?base64?{}?=",
            rusty_base64::encode_standard(b"tools/call")
        );
        let build = |value: &str| {
            Request::builder()
                .header("mcp-method", value)
                .body(())
                .expect("request")
        };

        assert_eq!(method_of(&build("tools/list")), "tools/list");
        assert_eq!(method_of(&build(&wrapped)), "tools/call");
        assert_eq!(method_of(&build("=?base64?!!!?=")), "");
        assert_eq!(method_of(&Request::new(())), "");
    }

    #[test]
    fn a_present_text_header_is_read_and_anything_else_is_not() {
        let request = Request::builder()
            .header("mcp-method", "tools/call")
            .header(
                "mcp-name",
                http::HeaderValue::from_bytes(b"\xff").expect("opaque"),
            )
            .body(())
            .expect("request");

        assert_eq!(text_header(&request, "mcp-method"), Some("tools/call"));
        assert_eq!(text_header(&request, "mcp-name"), None, "not text");
        assert_eq!(text_header(&request, "absent"), None);
    }

    use crate::otel::instrument::Data;

    /// Sum of every `in_flight` data point right now.
    fn in_flight(instruments: &Instruments) -> i64 {
        instruments
            .registry()
            .collect()
            .iter()
            .filter(|metric| metric.name == "mcp.server.requests.in_flight")
            .map(|metric| match &metric.data {
                Data::Sum { points, .. } => points.iter().map(|(_, value)| value).sum::<i64>(),
                other => panic!("unexpected aggregation: {other:?}"),
            })
            .sum()
    }

    #[test]
    fn in_flight_is_one_while_a_request_is_open_and_zero_after() {
        // A gauge that only ever climbed would pass every other test.
        let instruments = Instruments::new();

        instruments.request_started("tools/call");
        assert_eq!(in_flight(&instruments), 1);

        instruments.request_finished("tools/call", None, Outcome::Ok, 0.01);
        assert_eq!(in_flight(&instruments), 0);
    }

    #[test]
    fn a_finished_request_is_counted_and_timed_under_the_same_labels() {
        let instruments = Instruments::new();
        instruments.request_started("ping");
        instruments.request_finished("ping", None, Outcome::Ok, 0.2);

        let metrics = instruments.registry().collect();
        let counted = metrics
            .iter()
            .find(|m| m.name == "mcp.server.requests")
            .expect("requests");
        let timed = metrics
            .iter()
            .find(|m| m.name == "mcp.server.request.duration")
            .expect("duration");
        match (&counted.data, &timed.data) {
            (
                Data::Sum { points: counts, .. },
                Data::Histogram {
                    points: timings, ..
                },
            ) => {
                assert_eq!(counts[0].1, 1);
                assert_eq!(timings[0].count, 1);
                // The outcome is on the counter and not on the histogram.
                assert_eq!(counts[0].0.len(), timings[0].attributes.len() + 1);
            }
            other => panic!("unexpected shapes: {other:?}"),
        }
    }

    #[test]
    fn abandoned_tasks_are_counted_in_bulk_and_zero_is_ignored() {
        let instruments = Instruments::new();
        instruments.tasks_abandoned(0);
        instruments.tasks_abandoned(3);

        let finished = instruments
            .registry()
            .collect()
            .into_iter()
            .find(|m| m.name == "mcp.server.tasks.finished")
            .expect("finished");
        match finished.data {
            Data::Sum { points, .. } => assert_eq!(points.len(), 1),
            other => panic!("unexpected: {other:?}"),
        }
    }
}
