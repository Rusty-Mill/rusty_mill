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
use opentelemetry::{
    KeyValue,
    metrics::{Counter, Histogram, Meter, UpDownCounter},
};

use super::labels::{
    self, HEADER_MCP_METHOD, KnownNames, Label, header, in_flight_labels, request_labels,
    task_labels,
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
/// Build one per process and share it — `Arc` it into the layer.
#[derive(Debug, Clone)]
pub struct Instruments {
    requests: Counter<u64>,
    duration: Histogram<f64>,
    in_flight: UpDownCounter<i64>,
    tasks_started: Counter<u64>,
    tasks_finished: Counter<u64>,
}

impl Instruments {
    /// Define the instruments on `meter`.
    pub fn new(meter: &Meter) -> Self {
        Self {
            requests: meter
                .u64_counter("mcp.server.requests")
                .with_description("MCP requests handled, by method and outcome.")
                .with_unit("{request}")
                .build(),
            duration: meter
                .f64_histogram("mcp.server.request.duration")
                .with_description("Time to handle an MCP request.")
                .with_unit("s")
                .with_boundaries(LATENCY_BUCKETS.to_vec())
                .build(),
            in_flight: meter
                .i64_up_down_counter("mcp.server.requests.in_flight")
                .with_description("MCP requests currently being handled.")
                .with_unit("{request}")
                .build(),
            tasks_started: meter
                .u64_counter("mcp.server.tasks.started")
                .with_description("Tasks spawned under the tasks extension.")
                .with_unit("{task}")
                .build(),
            tasks_finished: meter
                .u64_counter("mcp.server.tasks.finished")
                .with_description("Tasks that settled, by outcome.")
                .with_unit("{task}")
                .build(),
        }
    }

    /// A request has started. Pair with [`Instruments::request_finished`].
    pub fn request_started(&self, method: &str) {
        self.in_flight.add(1, &key_values(in_flight_labels(method)));
    }

    /// A request has finished.
    pub fn request_finished(
        &self,
        method: &str,
        name: Option<&str>,
        outcome: Outcome,
        seconds: f64,
    ) {
        self.in_flight
            .add(-1, &key_values(in_flight_labels(method)));
        self.duration
            .record(seconds, &key_values(labels::duration_labels(method, name)));
        self.requests
            .add(1, &key_values(request_labels(method, name, outcome)));
    }

    /// A task was spawned.
    pub fn task_started(&self) {
        self.tasks_started.add(1, &[]);
    }

    /// A task settled.
    pub fn task_finished(&self, outcome: TaskOutcome) {
        self.tasks_finished
            .add(1, &key_values(task_labels(outcome)));
    }

    /// Several tasks were abandoned at once, as at the end of a drain.
    pub fn tasks_abandoned(&self, count: usize) {
        if count > 0 {
            self.tasks_finished.add(
                count as u64,
                &key_values(task_labels(TaskOutcome::Abandoned)),
            );
        }
    }
}

/// Hand the core's labels to the backend.
fn key_values(labels: Vec<Label>) -> Vec<KeyValue> {
    labels
        .into_iter()
        .map(|(key, value)| KeyValue::new(key, value))
        .collect()
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

        let method = header(request.headers(), HEADER_MCP_METHOD).unwrap_or_default();
        let name = self.known_names.name_label(&method, request.headers());
        let instruments = Arc::clone(&self.instruments);

        Box::pin(async move {
            instruments.request_started(&method);
            let started = Instant::now();

            let response = inner.call(request).await?.into_response();

            instruments.request_finished(
                &method,
                name.as_deref(),
                Outcome::from_status(response.status()),
                started.elapsed().as_secs_f64(),
            );

            Ok(response)
        })
    }
}
