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
    collections::BTreeSet,
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

/// The `Mcp-Method` header (SEP-2243).
const HEADER_MCP_METHOD: &str = "mcp-method";
/// The `Mcp-Name` header (SEP-2243).
const HEADER_MCP_NAME: &str = "mcp-name";

/// `rmcp` wraps a header value that will not survive as-is in these sentinels.
const BASE64_PREFIX: &str = "=?base64?";
const BASE64_SUFFIX: &str = "?=";

/// Label used whenever a value is not in its allow-list.
const OTHER: &str = "other";

/// Every method the spec defines, so an unknown one cannot mint a label.
const KNOWN_METHODS: &[&str] = &[
    "completion/complete",
    "initialize",
    "logging/setLevel",
    "notifications/cancelled",
    "notifications/initialized",
    "notifications/progress",
    "notifications/roots/list_changed",
    "ping",
    "prompts/get",
    "prompts/list",
    "resources/list",
    "resources/read",
    "resources/subscribe",
    "resources/templates/list",
    "resources/unsubscribe",
    "roots/list",
    "server/discover",
    "subscriptions/listen",
    "tasks/cancel",
    "tasks/get",
    "tasks/update",
    "tools/call",
    "tools/list",
];

/// Methods whose `Mcp-Name` is a bounded identifier rather than free text.
///
/// `resources/read` is deliberately absent: its name is the URI, which a client
/// chooses. Task methods are absent for the same reason — a task id is unique
/// per task, which is the worst possible label.
const NAMED_METHODS: &[&str] = &["tools/call", "prompts/get"];

/// Latency buckets, in seconds.
///
/// Weighted towards the fast end: an MCP server that is behaving spends most of
/// its time in the low milliseconds, and buckets spread evenly to ten seconds
/// would put nearly every request in the first one.
const LATENCY_BUCKETS: &[f64] = &[
    0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

/// How a request ended, as far as the transport can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Outcome {
    /// A 2xx response.
    Ok,
    /// A 401 or 403 — separated out because an authorization problem is a
    /// different page in the middle of the night than a malformed request.
    Unauthorized,
    /// Any other 4xx.
    ClientError,
    /// A 5xx.
    ServerError,
}

impl Outcome {
    fn from_status(status: http::StatusCode) -> Self {
        match status.as_u16() {
            200..=299 => Self::Ok,
            401 | 403 => Self::Unauthorized,
            400..=499 => Self::ClientError,
            _ => Self::ServerError,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Unauthorized => "unauthorized",
            Self::ClientError => "client_error",
            Self::ServerError => "server_error",
        }
    }
}

/// How a task ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TaskOutcome {
    /// Ran to completion and produced a result.
    Completed,
    /// Settled as cancelled, because its body cooperated with a cancel request.
    Cancelled,
    /// Failed with an error.
    Failed,
    /// Still running when the drain grace period expired, and aborted.
    ///
    /// Worth an alert of its own: a client is polling a task id that will never
    /// produce a result.
    Abandoned,
}

impl TaskOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
            Self::Abandoned => "abandoned",
        }
    }
}

/// The instruments this crate records to.
///
/// Build one per process and share it — `Arc` it into the layer and into
/// [`crate::tasks::TaskSupport`].
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
        self.in_flight
            .add(1, &[KeyValue::new("mcp.method", label(method))]);
    }

    /// A request has finished.
    pub fn request_finished(
        &self,
        method: &str,
        name: Option<&str>,
        outcome: Outcome,
        seconds: f64,
    ) {
        let method = label(method);
        self.in_flight
            .add(-1, &[KeyValue::new("mcp.method", method)]);

        let mut attributes = vec![
            KeyValue::new("mcp.method", method),
            KeyValue::new("mcp.name", name.unwrap_or(OTHER).to_string()),
        ];
        self.duration.record(seconds, &attributes);

        attributes.push(KeyValue::new("mcp.outcome", outcome.as_str()));
        self.requests.add(1, &attributes);
    }

    /// A task was spawned.
    pub fn task_started(&self) {
        self.tasks_started.add(1, &[]);
    }

    /// A task settled.
    pub fn task_finished(&self, outcome: TaskOutcome) {
        self.tasks_finished
            .add(1, &[KeyValue::new("mcp.task.outcome", outcome.as_str())]);
    }

    /// Several tasks were abandoned at once, as at the end of a drain.
    pub fn tasks_abandoned(&self, count: usize) {
        if count > 0 {
            self.tasks_finished.add(
                count as u64,
                &[KeyValue::new(
                    "mcp.task.outcome",
                    TaskOutcome::Abandoned.as_str(),
                )],
            );
        }
    }
}

/// Map a method to itself if the spec defines it, and to `other` if not.
fn label(method: &str) -> &'static str {
    KNOWN_METHODS
        .iter()
        .find(|known| **known == method)
        .copied()
        .unwrap_or(OTHER)
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
    known_names: Arc<BTreeSet<String>>,
}

impl McpMetricsLayer {
    /// Record to `instruments`, with no tool or prompt names labelled.
    pub fn new(instruments: Arc<Instruments>) -> Self {
        Self {
            instruments,
            known_names: Arc::new(BTreeSet::new()),
        }
    }

    /// Names that may appear in the `mcp.name` label.
    ///
    /// Pass the tool and prompt names this server actually serves. Anything
    /// else — including a call to a tool that does not exist — is labelled
    /// `other`, which is what keeps a client from minting unbounded labels.
    pub fn with_known_names(mut self, names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.known_names = Arc::new(names.into_iter().map(Into::into).collect());
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
    known_names: Arc<BTreeSet<String>>,
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

        let method = header(&request, HEADER_MCP_METHOD).unwrap_or_default();
        let name = self.name_label(&method, &request);
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

impl<S> McpMetrics<S> {
    /// The `mcp.name` label for this request, if one is safe to record.
    fn name_label<ReqBody>(&self, method: &str, request: &Request<ReqBody>) -> Option<String> {
        if !NAMED_METHODS.contains(&method) {
            return None;
        }

        let name = header(request, HEADER_MCP_NAME)?;
        self.known_names.contains(&name).then_some(name)
    }
}

/// A header value, decoded if `rmcp` base64-wrapped it.
fn header<ReqBody>(request: &Request<ReqBody>, name: &str) -> Option<String> {
    let raw = request.headers().get(name)?.to_str().ok()?;

    let Some(encoded) = raw
        .strip_prefix(BASE64_PREFIX)
        .and_then(|rest| rest.strip_suffix(BASE64_SUFFIX))
    else {
        return Some(raw.to_string());
    };

    let bytes = rusty_base64::decode_standard(encoded).ok()?;
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request_with(headers: &[(&str, &str)]) -> Request<()> {
        let mut builder = Request::builder();
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        builder.body(()).expect("request")
    }

    fn layer_with_names(names: &[&str]) -> McpMetrics<()> {
        let known_names = Arc::new(names.iter().map(|n| (*n).to_string()).collect());
        McpMetrics {
            inner: (),
            // Never recorded to in these tests; only `name_label` is exercised.
            instruments: Arc::new(Instruments::new(&opentelemetry::global::meter("test-only"))),
            known_names,
        }
    }

    #[test]
    fn a_known_method_keeps_its_name() {
        assert_eq!(label("tools/call"), "tools/call");
        assert_eq!(label("resources/read"), "resources/read");
    }

    #[test]
    fn an_unknown_method_cannot_mint_a_label() {
        // Otherwise anyone who can reach the endpoint owns your label space.
        assert_eq!(label("tools/../../etc/passwd"), OTHER);
        assert_eq!(label(""), OTHER);
        assert_eq!(label("a-method-invented-by-a-client"), OTHER);
    }

    #[test]
    fn a_known_tool_name_is_labelled() {
        let service = layer_with_names(&["add", "divide"]);
        let request = request_with(&[("mcp-method", "tools/call"), ("mcp-name", "add")]);

        assert_eq!(
            service.name_label("tools/call", &request).as_deref(),
            Some("add")
        );
    }

    #[test]
    fn an_unknown_tool_name_is_dropped() {
        // The call will fail, but the label would already have been recorded.
        let service = layer_with_names(&["add"]);
        let request = request_with(&[("mcp-method", "tools/call"), ("mcp-name", "a1b2c3d4")]);

        assert_eq!(service.name_label("tools/call", &request), None);
    }

    #[test]
    fn a_resource_uri_is_never_labelled() {
        // `Mcp-Name` for `resources/read` is the URI. Unbounded by definition,
        // and the single easiest way to take a metrics backend down.
        let service = layer_with_names(&["db://tables/users"]);
        let request = request_with(&[
            ("mcp-method", "resources/read"),
            ("mcp-name", "db://tables/users"),
        ]);

        assert_eq!(service.name_label("resources/read", &request), None);
    }

    #[test]
    fn a_task_id_is_never_labelled() {
        let service = layer_with_names(&["task-1234"]);
        let request = request_with(&[("mcp-method", "tasks/get"), ("mcp-name", "task-1234")]);

        assert_eq!(service.name_label("tasks/get", &request), None);
    }

    #[test]
    fn a_base64_wrapped_header_is_decoded() {
        let encoded = rusty_base64::encode_standard(b"a prompt");
        let wrapped = format!("{BASE64_PREFIX}{encoded}{BASE64_SUFFIX}");

        let service = layer_with_names(&["a prompt"]);
        let request = request_with(&[("mcp-method", "prompts/get"), ("mcp-name", &wrapped)]);

        assert_eq!(
            service.name_label("prompts/get", &request).as_deref(),
            Some("a prompt")
        );
    }

    #[test]
    fn a_malformed_base64_header_is_dropped_rather_than_recorded_raw() {
        let wrapped = format!("{BASE64_PREFIX}!!!not-base64!!!{BASE64_SUFFIX}");
        let service = layer_with_names(&["add"]);
        let request = request_with(&[("mcp-method", "tools/call"), ("mcp-name", &wrapped)]);

        assert_eq!(service.name_label("tools/call", &request), None);
    }

    #[test]
    fn a_missing_name_header_is_not_an_error() {
        let service = layer_with_names(&["add"]);
        let request = request_with(&[("mcp-method", "tools/call")]);

        assert_eq!(service.name_label("tools/call", &request), None);
    }

    #[test]
    fn outcomes_map_from_status() {
        use http::StatusCode;

        assert_eq!(Outcome::from_status(StatusCode::OK), Outcome::Ok);
        assert_eq!(Outcome::from_status(StatusCode::ACCEPTED), Outcome::Ok);
        assert_eq!(
            Outcome::from_status(StatusCode::UNAUTHORIZED),
            Outcome::Unauthorized
        );
        assert_eq!(
            Outcome::from_status(StatusCode::FORBIDDEN),
            Outcome::Unauthorized
        );
        assert_eq!(
            Outcome::from_status(StatusCode::PAYLOAD_TOO_LARGE),
            Outcome::ClientError
        );
        assert_eq!(
            Outcome::from_status(StatusCode::INTERNAL_SERVER_ERROR),
            Outcome::ServerError
        );
    }
}
