//! Exporting spans and metrics to an OpenTelemetry collector over OTLP/HTTP.
//!
//! Enabled by the `otel` feature. [`crate::trace`] extracts the caller's trace
//! context and records the ids on `tracing` spans, which is enough to correlate
//! logs. This module goes the rest of the way: [`SpanLayer`] turns those spans
//! into OpenTelemetry spans and makes the ones that carry a caller's ids genuine
//! **children** of the caller's, so a trace renders as one tree across services
//! rather than several disconnected ones.
//!
//! Nothing here uses the OpenTelemetry SDK. Spans and metrics are encoded as
//! OTLP/JSON and POSTed with `rusty_request` from a thread of their own.
//!
//! ```no_run
//! use rusty_mcp_middleware::otel::OtelConfig;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let guard = rusty_mcp_middleware::otel::init(
//!     OtelConfig::new("my-mcp-server").with_endpoint("http://localhost:4318"),
//!     "info",
//! )?;
//!
//! // ... serve ...
//!
//! // Without this, buffered spans die with the process.
//! guard.shutdown();
//! # Ok(())
//! # }
//! ```
//!
//! # Where telemetry goes
//!
//! Each signal has its own URL, resolved as the OpenTelemetry specification
//! says: the signal's own setting (`metrics_endpoint`, or
//! `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` / `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT`),
//! taken as the full URL; else a base URL (`endpoint`, or
//! `OTEL_EXPORTER_OTLP_ENDPOINT`) with `/v1/traces` or `/v1/metrics` added; else
//! `http://localhost:4318`. This is OTLP over HTTP, so the port is 4318, not the
//! gRPC port 4317.
//!
//! # Flush before exiting
//!
//! Spans are batched, so whatever is still in the buffer is lost if the process
//! exits without shutting the exporter down. That is the single most common way
//! to end up staring at an empty collector while insisting the code is
//! instrumented. [`OtelGuard::shutdown_hook`] gives a closure to run
//! from your graceful-shutdown path so it happens on SIGTERM.
//!
//! # Sampling follows the caller
//!
//! The default sampler is parent-based: if the caller sampled the trace, so do
//! we; if it did not, we stay quiet. Deciding independently is how traces end
//! up half-recorded, with gaps exactly where a service made its own choice.

mod batch;
mod instrument;
mod labels;
mod layer;
pub mod metrics;
mod otlp;
mod push;
mod redact;
mod span;
mod worker;

use std::{sync::Arc, time::Duration};

use rusty_log::tracing_subscriber::{EnvFilter, fmt, prelude::*};

pub use self::{layer::SpanLayer, span::AttributeValue};
use crate::otel::{
    batch::{ExportSettings, SpanExporter},
    metrics::Instruments,
    push::{PushSettings, Pusher},
};

/// Instrumentation scope name for this crate's spans and instruments.
const SCOPE: &str = "rusty-mcp";

/// Where OTLP/HTTP is served when nothing else says.
const DEFAULT_BASE_ENDPOINT: &str = "http://localhost:4318";

/// How long finished spans wait for company before they are sent anyway.
const SPAN_BATCH_INTERVAL: Duration = Duration::from_secs(5);

/// Set an attribute on `span`, for a key only known after the span was created.
///
/// A `tracing` field has to be declared when its span is opened; this is for
/// everything else, such as values a policy returns mid-request. It does
/// nothing if the span is not recorded: no [`SpanLayer`] is installed, the span
/// was not sampled, or it has closed. Anything beyond the per-span attribute
/// limit is dropped.
pub fn set_attribute(span: &rusty_log::Span, key: &str, value: impl Into<AttributeValue>) {
    let value = value.into();
    span.with_subscriber(|(id, dispatch)| {
        if let Some(layer) = dispatch.downcast_ref::<SpanLayer>() {
            layer.set_attribute(id, key, value);
        }
    });
}

/// A kind of telemetry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// Spans.
    Traces,
    /// Metrics.
    Metrics,
}

impl Signal {
    /// The path OTLP/HTTP serves the signal on.
    fn path(self) -> &'static str {
        match self {
            Self::Traces => "/v1/traces",
            Self::Metrics => "/v1/metrics",
        }
    }

    /// The signal-specific environment variable naming its full URL.
    fn variable(self) -> &'static str {
        match self {
            Self::Traces => "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
            Self::Metrics => "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT",
        }
    }
}

impl std::fmt::Display for Signal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Traces => "traces",
            Self::Metrics => "metrics",
        })
    }
}

/// Starting an OTLP pipeline failed.
#[derive(Debug)]
#[non_exhaustive]
pub enum OtelError {
    /// A signal's endpoint is not a valid `http://` or `https://` URL.
    ///
    /// The URL is deliberately not carried: it may hold a credential.
    Endpoint(Signal),
    /// The thread that exports a signal could not be started.
    Thread(Signal, std::io::Error),
}

impl std::fmt::Display for OtelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Endpoint(signal) => write!(
                f,
                "the OTLP {signal} endpoint is not a valid http(s) URL without embedded credentials \
                 (it is not shown, as it may hold a secret)"
            ),
            Self::Thread(signal, e) => {
                write!(f, "could not start the {signal} export thread: {e}")
            }
        }
    }
}

impl std::error::Error for OtelError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Endpoint(_) => None,
            Self::Thread(_, e) => Some(e),
        }
    }
}

/// How to talk to the collector, and what to call ourselves.
#[derive(Clone)]
pub struct OtelConfig {
    /// `service.name` on every span and metric. Required by the OTel semantic
    /// conventions and the first thing you filter by in a collector.
    pub service_name: String,
    /// `service.version`, if you want releases distinguishable in traces.
    pub service_version: Option<String>,
    /// The collector's OTLP/HTTP base URL, such as `http://localhost:4318`.
    /// `/v1/traces` and `/v1/metrics` are added to it. `None` falls back to
    /// `OTEL_EXPORTER_OTLP_ENDPOINT`, then `http://localhost:4318`.
    pub endpoint: Option<String>,
    /// The full URL for **metrics**, `/v1/metrics` included, when it differs
    /// from the base. `None` falls back to `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT`,
    /// then the base.
    pub metrics_endpoint: Option<String>,
    /// Fraction of *root* traces to record, 0.0 to 1.0.
    ///
    /// Only consulted when there is no parent decision to follow; a sampled
    /// caller is always honoured.
    pub sample_ratio: f64,
    /// How long `shutdown` waits for the buffer to drain.
    pub shutdown_timeout: Duration,
    /// Export metrics as well as spans.
    ///
    /// On by default. Metrics are what alerts fire on; traces are what you read
    /// afterwards, and a parent-based sampler means the spans this server
    /// records are the ones its callers chose, not a representative sample of
    /// its own traffic.
    pub metrics: bool,
    /// How often metrics are pushed to the collector.
    pub metrics_interval: Duration,
}

// By hand, so that logging a config cannot print a credential held in an endpoint.
impl std::fmt::Debug for OtelConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let origin = |url: &Option<String>| url.as_deref().map(redact::origin);
        f.debug_struct("OtelConfig")
            .field("service_name", &self.service_name)
            .field("service_version", &self.service_version)
            .field("endpoint", &origin(&self.endpoint))
            .field("metrics_endpoint", &origin(&self.metrics_endpoint))
            .field("sample_ratio", &self.sample_ratio)
            .field("shutdown_timeout", &self.shutdown_timeout)
            .field("metrics", &self.metrics)
            .field("metrics_interval", &self.metrics_interval)
            .finish()
    }
}

impl OtelConfig {
    /// Config for a service called `service_name`, sampling everything.
    pub fn new(service_name: impl Into<String>) -> Self {
        Self {
            service_name: service_name.into(),
            service_version: None,
            endpoint: None,
            metrics_endpoint: None,
            sample_ratio: 1.0,
            shutdown_timeout: Duration::from_secs(5),
            metrics: true,
            metrics_interval: Duration::from_secs(30),
        }
    }

    /// Export spans only.
    pub fn without_metrics(mut self) -> Self {
        self.metrics = false;
        self
    }

    /// How often to push metrics to the collector.
    pub fn with_metrics_interval(mut self, interval: Duration) -> Self {
        self.metrics_interval = interval;
        self
    }

    /// Set `service.version`.
    pub fn with_service_version(mut self, version: impl Into<String>) -> Self {
        self.service_version = Some(version.into());
        self
    }

    /// Point metrics at a specific OTLP/HTTP URL (`.../v1/metrics`).
    pub fn with_metrics_endpoint(mut self, url: impl Into<String>) -> Self {
        self.metrics_endpoint = Some(url.into());
        self
    }

    /// Point telemetry at a collector's OTLP/HTTP base URL.
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    /// Fraction of root traces to record. Clamped to `0.0..=1.0`.
    pub fn with_sample_ratio(mut self, ratio: f64) -> Self {
        self.sample_ratio = ratio.clamp(0.0, 1.0);
        self
    }

    /// How long to wait for the buffer to drain on shutdown.
    pub fn with_shutdown_timeout(mut self, timeout: Duration) -> Self {
        self.shutdown_timeout = timeout;
        self
    }

    /// Where `signal` is sent, resolved as the module docs describe.
    fn endpoint_for(&self, signal: Signal) -> Result<String, OtelError> {
        let variable = |name: &str| std::env::var(name).ok();
        let explicit = match signal {
            Signal::Metrics => self.metrics_endpoint.clone(),
            Signal::Traces => None,
        };
        resolve_endpoint(
            signal,
            &Sources {
                explicit,
                signal_variable: variable(signal.variable()),
                base: self.endpoint.clone(),
                base_variable: variable("OTEL_EXPORTER_OTLP_ENDPOINT"),
            },
        )
    }

    /// The resource attributes spans and metrics carry: the process
    /// environment's `OTEL_RESOURCE_ATTRIBUTES`, then this config's
    /// `service.name` and `service.version` over them.
    fn resource_attributes(&self) -> Vec<(String, String)> {
        let from_environment = std::env::var(RESOURCE_ATTRIBUTES_VAR).ok();
        self.resource_attributes_from(from_environment.as_deref())
    }

    fn resource_attributes_from(&self, declared: Option<&str>) -> Vec<(String, String)> {
        let mut attributes = parse_resource_attributes(declared.unwrap_or_default());
        set_attribute_pair(&mut attributes, "service.name", &self.service_name);
        if let Some(version) = &self.service_version {
            set_attribute_pair(&mut attributes, "service.version", version);
        }
        attributes
    }
}

/// The places a signal's URL can come from, most specific first.
struct Sources {
    /// A URL set in code for this signal alone.
    explicit: Option<String>,
    /// The signal's own environment variable.
    signal_variable: Option<String>,
    /// A base URL set in code.
    base: Option<String>,
    /// The base-URL environment variable.
    base_variable: Option<String>,
}

/// Pick the URL for `signal`: a full URL as given, a base with the signal's path
/// added, or the default base. Any of them must parse as an `http(s)` URL.
fn resolve_endpoint(signal: Signal, sources: &Sources) -> Result<String, OtelError> {
    let full = sources
        .explicit
        .as_ref()
        .or(sources.signal_variable.as_ref());
    if let Some(url) = full {
        // Parsed, not just prefix-checked: a malformed URL must fail here, where
        // the error can stay silent about it, not later inside an HTTP client.
        return rusty_http::Url::parse(url)
            .map(|_| url.clone())
            .map_err(|_| OtelError::Endpoint(signal));
    }
    let base = sources
        .base
        .as_deref()
        .or(sources.base_variable.as_deref())
        .unwrap_or(DEFAULT_BASE_ENDPOINT);
    with_path(base, signal.path()).ok_or(OtelError::Endpoint(signal))
}

/// `base` with `path` added to its own path, keeping any query.
fn with_path(base: &str, path: &str) -> Option<String> {
    let url = rusty_http::Url::parse(base).ok()?;
    let prefix = url.path.trim_end_matches('/');
    let query = url
        .query
        .as_ref()
        .map(|q| format!("?{q}"))
        .unwrap_or_default();
    Some(format!(
        "{}://{}{prefix}{path}{query}",
        url.scheme,
        url.host_header()
    ))
}

const RESOURCE_ATTRIBUTES_VAR: &str = "OTEL_RESOURCE_ATTRIBUTES";

/// `key1=value1,key2=value2`, read as the OpenTelemetry SDKs' environment
/// resource detector does: an entry without `=` is skipped, keys and values are
/// trimmed, and a repeated key keeps its last value.
fn parse_resource_attributes(text: &str) -> Vec<(String, String)> {
    let mut attributes = Vec::new();
    for entry in text.split_terminator(',') {
        if let Some((key, value)) = entry.split_once('=') {
            set_attribute_pair(&mut attributes, key.trim(), value.trim());
        }
    }
    attributes
}

/// Set `key`, replacing an earlier value in place.
fn set_attribute_pair(attributes: &mut Vec<(String, String)>, key: &str, value: &str) {
    match attributes.iter_mut().find(|(k, _)| k == key) {
        Some(existing) => existing.1 = value.to_owned(),
        None => attributes.push((key.to_owned(), value.to_owned())),
    }
}

/// Keeps the pipeline alive and flushes it on the way out.
///
/// Hold it for as long as the server runs. Dropping it flushes on a best-effort
/// basis, but call [`OtelGuard::shutdown`] explicitly where you can — the
/// blocking flush is the only version that reports failure.
#[derive(Debug)]
pub struct OtelGuard {
    spans: Arc<SpanExporter>,
    pusher: Option<Pusher>,
    instruments: Option<Arc<Instruments>>,
    timeout: Duration,
}

impl OtelGuard {
    /// Flush buffered telemetry and stop the pipeline.
    ///
    /// Idempotent, and safe to call from a shutdown hook.
    pub fn shutdown(&self) {
        if !self.spans.shutdown(self.timeout) {
            // Worth a loud line: it means telemetry was silently dropped, which
            // otherwise looks like the code was never instrumented.
            rusty_log::error!("failed to flush spans before shutdown");
        }
        if let Some(pusher) = &self.pusher
            && !pusher.shutdown(self.timeout)
        {
            rusty_log::error!("failed to flush metrics before shutdown");
        }
    }

    /// Flush without stopping, for a checkpoint mid-run.
    pub fn flush(&self) {
        if !self.spans.flush(self.timeout) {
            rusty_log::warn!("failed to flush spans");
        }
        if let Some(pusher) = &self.pusher
            && !pusher.flush(self.timeout)
        {
            rusty_log::warn!("failed to flush metrics");
        }
    }

    /// The instruments this crate records to, when metrics are enabled.
    ///
    /// Share this: `Arc` it into [`metrics::McpMetrics`] for request
    /// metrics. It is a no-op if you never wire it up — an instrument that
    /// nothing records to exports nothing.
    pub fn instruments(&self) -> Option<&Arc<Instruments>> {
        self.instruments.as_ref()
    }

    /// A closure that flushes and shuts the pipeline down, for the process's
    /// graceful-shutdown path.
    ///
    /// ```no_run
    /// # use std::sync::Arc;
    /// # use rusty_mcp_middleware::otel::{OtelConfig, self};
    /// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// let guard = Arc::new(otel::init(OtelConfig::new("my-server"), "info")?);
    /// let hook = guard.shutdown_hook();
    /// // ... serve until SIGTERM, then:
    /// hook().await;
    /// # Ok(())
    /// # }
    /// ```
    pub fn shutdown_hook(
        self: &std::sync::Arc<Self>,
    ) -> impl Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
    + Send
    + Sync
    + 'static {
        let guard = std::sync::Arc::clone(self);
        move || {
            let guard = std::sync::Arc::clone(&guard);
            Box::pin(async move {
                // Blocking flush on the async path: shutdown is the one moment
                // where waiting beats losing the data.
                guard.shutdown();
            })
        }
    }
}

impl Drop for OtelGuard {
    fn drop(&mut self) {
        // Best effort. An explicit `shutdown()` is better because it can report
        // failure; this only catches the case where nobody called it. The span
        // exporter is shared with the layer, so it would not stop on its own.
        let _ = self.spans.shutdown(self.timeout);
        if let Some(pusher) = &self.pusher {
            let _ = pusher.shutdown(self.timeout);
        }
    }
}

/// Start an OTLP pipeline and hand back the layer that feeds it, installing
/// nothing.
///
/// Use this when the process already builds its own subscriber — a common case,
/// since [`init`] can only win once per process and a second call would leave
/// the second pipeline fed by nothing:
///
/// ```no_run
/// use rusty_log::tracing_subscriber::prelude::*;
/// # use rusty_mcp_middleware::otel::OtelConfig;
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let (guard, layer) = rusty_mcp_middleware::otel::pipeline(OtelConfig::new("my-server"))?;
///
/// rusty_log::tracing_subscriber::registry()
///     .with(rusty_log::tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
///     .with(layer)
///     .init();
/// # let _ = guard;
/// # Ok(())
/// # }
/// ```
pub fn pipeline(config: OtelConfig) -> Result<(OtelGuard, SpanLayer), OtelError> {
    let resource = config.resource_attributes();

    let spans = Arc::new(
        batch::spawn(ExportSettings {
            endpoint: config.endpoint_for(Signal::Traces)?,
            resource: resource.clone(),
            scope: SCOPE,
            interval: SPAN_BATCH_INTERVAL,
            request_timeout: config.shutdown_timeout,
        })
        .map_err(|e| OtelError::Thread(Signal::Traces, e))?,
    );
    let layer = SpanLayer::new(config.sample_ratio, {
        let spans = Arc::clone(&spans);
        Box::new(move |span| spans.submit(span))
    });

    let (pusher, instruments) = if config.metrics {
        let instruments = Arc::new(Instruments::new());
        let pusher = push::spawn(PushSettings {
            endpoint: config.endpoint_for(Signal::Metrics)?,
            resource,
            scope: SCOPE,
            interval: config.metrics_interval,
            request_timeout: config.shutdown_timeout,
            registry: Arc::clone(instruments.registry()),
        })
        .map_err(|e| OtelError::Thread(Signal::Metrics, e))?;
        (Some(pusher), Some(instruments))
    } else {
        (None, None)
    };

    Ok((
        OtelGuard {
            spans,
            pusher,
            instruments,
            timeout: config.shutdown_timeout,
        },
        layer,
    ))
}

/// Start an OTLP pipeline and install a subscriber that feeds it.
///
/// Installs the global subscriber, with both a stderr layer (so stdio servers
/// still log where they should) and the [`SpanLayer`].
///
/// `filter` is the fallback log directive; `RUST_LOG` wins when set.
pub fn init(config: OtelConfig, filter: &str) -> Result<OtelGuard, OtelError> {
    let service_name = config.service_name.clone();
    let endpoint = config.endpoint_for(Signal::Traces)?;
    let sample_ratio = config.sample_ratio;

    let (guard, layer) = pipeline(config)?;

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(filter));

    let installed = rusty_log::tracing_subscriber::registry()
        .with(fmt::layer().with_writer(std::io::stderr).with_ansi(false))
        .with(layer)
        .with(env_filter)
        .try_init();

    if installed.is_err() {
        // A subscriber was already installed, so spans will not reach the layer
        // we just built. Worth saying plainly rather than exporting nothing and
        // leaving the operator to wonder.
        rusty_log::warn!("a tracing subscriber was already installed; spans will not be exported");
    }

    rusty_log::info!(
        service = %service_name,
        endpoint = %redact::origin(&endpoint),
        sample_ratio,
        "exporting spans over OTLP"
    );

    Ok(guard)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sources() -> Sources {
        Sources {
            explicit: None,
            signal_variable: None,
            base: None,
            base_variable: None,
        }
    }

    fn resolved(signal: Signal, sources: &Sources) -> String {
        resolve_endpoint(signal, sources).expect("a valid endpoint")
    }

    #[test]
    fn the_default_is_otlp_http_on_4318() {
        assert_eq!(
            resolved(Signal::Traces, &sources()),
            "http://localhost:4318/v1/traces"
        );
        assert_eq!(
            resolved(Signal::Metrics, &sources()),
            "http://localhost:4318/v1/metrics"
        );
    }

    #[test]
    fn a_base_url_gets_the_signals_path_added() {
        let base = Sources {
            base: Some("https://collector.example:4318/otlp/".to_owned()),
            ..sources()
        };
        assert_eq!(
            resolved(Signal::Traces, &base),
            "https://collector.example:4318/otlp/v1/traces"
        );
        assert_eq!(
            resolved(Signal::Metrics, &base),
            "https://collector.example:4318/otlp/v1/metrics"
        );
    }

    #[test]
    fn a_base_urls_query_stays_after_the_path() {
        let base = Sources {
            base: Some("http://h:1/?tenant=a".to_owned()),
            ..sources()
        };
        assert_eq!(
            resolved(Signal::Traces, &base),
            "http://h:1/v1/traces?tenant=a"
        );
    }

    #[test]
    fn a_full_url_is_used_as_given() {
        let sources = Sources {
            signal_variable: Some("http://h:1/custom/traces".to_owned()),
            base: Some("http://ignored:1".to_owned()),
            ..sources()
        };
        assert_eq!(
            resolved(Signal::Traces, &sources),
            "http://h:1/custom/traces"
        );
    }

    #[test]
    fn the_most_specific_source_wins() {
        let all = Sources {
            explicit: Some("http://explicit/v1/metrics".to_owned()),
            signal_variable: Some("http://signal-env/v1/metrics".to_owned()),
            base: Some("http://base".to_owned()),
            base_variable: Some("http://base-env".to_owned()),
        };
        assert_eq!(
            resolved(Signal::Metrics, &all),
            "http://explicit/v1/metrics"
        );

        let without_explicit = Sources {
            explicit: None,
            ..all
        };
        assert_eq!(
            resolved(Signal::Metrics, &without_explicit),
            "http://signal-env/v1/metrics"
        );

        let without_signal = Sources {
            signal_variable: None,
            ..without_explicit
        };
        assert_eq!(
            resolved(Signal::Metrics, &without_signal),
            "http://base/v1/metrics"
        );

        let only_env = Sources {
            base: None,
            ..without_signal
        };
        assert_eq!(
            resolved(Signal::Metrics, &only_env),
            "http://base-env/v1/metrics"
        );
    }

    #[test]
    fn a_bad_endpoint_is_refused_without_naming_it() {
        let bad = [
            "localhost:4318",
            "ftp://h/x",
            "http://u:SECRET@h/x",
            "SECRET",
        ];
        for url in bad {
            for sources in [
                Sources {
                    explicit: Some(url.to_owned()),
                    ..sources()
                },
                Sources {
                    signal_variable: Some(url.to_owned()),
                    ..sources()
                },
                Sources {
                    base: Some(url.to_owned()),
                    ..sources()
                },
            ] {
                let err = resolve_endpoint(Signal::Metrics, &sources).expect_err(url);
                assert!(matches!(err, OtelError::Endpoint(Signal::Metrics)));
                assert!(!err.to_string().contains("SECRET"), "{err}");
            }
        }
    }

    #[test]
    fn the_environment_resource_is_parsed_like_the_sdks_do() {
        let attributes = parse_resource_attributes(" a = 1 ,b=2,no-equals,a=3,c=x=y");
        assert_eq!(
            attributes,
            [("a", "3"), ("b", "2"), ("c", "x=y")].map(|(k, v)| (k.to_owned(), v.to_owned()))
        );
        assert!(parse_resource_attributes("").is_empty());
    }

    #[test]
    fn the_config_wins_over_the_environment_resource() {
        let attributes = OtelConfig::new("mine")
            .with_service_version("2.0")
            .resource_attributes_from(Some(
                "service.instance.id=i-1,service.name=theirs,service.version=1.0",
            ));
        let get = |key: &str| {
            attributes
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("service.name"), Some("mine"));
        assert_eq!(get("service.version"), Some("2.0"));
        assert_eq!(get("service.instance.id"), Some("i-1"));
        assert_eq!(
            attributes
                .iter()
                .filter(|(k, _)| k == "service.name")
                .count(),
            1
        );
    }

    #[test]
    fn the_sample_ratio_is_clamped() {
        assert_eq!(
            OtelConfig::new("s").with_sample_ratio(2.0).sample_ratio,
            1.0
        );
        assert_eq!(
            OtelConfig::new("s").with_sample_ratio(-1.0).sample_ratio,
            0.0
        );
        assert_eq!(
            OtelConfig::new("s").with_sample_ratio(0.25).sample_ratio,
            0.25
        );
    }

    #[test]
    fn defaults_sample_everything_and_defer_to_the_environment_for_the_endpoint() {
        let config = OtelConfig::new("s");
        assert_eq!(config.sample_ratio, 1.0);
        assert!(config.endpoint.is_none(), "None means fall back to the env");
    }
}
