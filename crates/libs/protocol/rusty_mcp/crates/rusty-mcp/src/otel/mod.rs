//! Exporting spans to an OpenTelemetry collector over OTLP.
//!
//! Enabled by the `otel` feature. [`crate::trace`] extracts the caller's trace
//! context and records the ids on `tracing` spans, which is enough to correlate
//! logs. This module goes the rest of the way: it stands up a real OTLP
//! pipeline, and — with [`crate::trace::TraceContext::attach_parent`] — makes
//! this server's spans genuine **children** of the caller's, so a trace renders
//! as one tree across services rather than several disconnected ones.
//!
//! ```no_run
//! use rusty_mcp::otel::OtelConfig;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let guard = rusty_mcp::otel::init(
//!     OtelConfig::new("my-mcp-server").with_endpoint("http://localhost:4317"),
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
//! # Flush before exiting
//!
//! Spans are batched, so whatever is still in the buffer is lost if the process
//! exits without shutting the provider down. That is the single most common way
//! to end up staring at an empty collector while insisting the code is
//! instrumented. [`OtelGuard::shutdown_hook`] gives a closure to run
//! from your graceful-shutdown path so it happens on SIGTERM.
//!
//! # Sampling follows the caller
//!
//! The default sampler is parent-based: if the caller sampled the trace, so do
//! we; if it did not, we stay quiet. Deciding independently is how traces end
//! up half-recorded, with gaps exactly where a service made its own choice.

mod instrument;
mod labels;
pub mod metrics;
mod otlp;
mod push;

use std::{sync::Arc, time::Duration};

use opentelemetry::{KeyValue, trace::TracerProvider as _};
use opentelemetry_otlp::{SpanExporter, WithExportConfig as _};
use opentelemetry_sdk::{
    Resource,
    trace::{Sampler, SdkTracerProvider},
};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use crate::otel::{
    metrics::Instruments,
    push::{PushSettings, Pusher},
};

/// Instrumentation scope name for this crate's instruments.
const SCOPE: &str = "rusty-mcp";

/// Where metrics go when nothing else says: OTLP/HTTP on the standard port.
const DEFAULT_METRICS_ENDPOINT: &str = "http://localhost:4318/v1/metrics";

/// Starting an OTLP pipeline failed.
#[derive(Debug)]
#[non_exhaustive]
pub enum OtelError {
    /// The span exporter could not be built — usually a malformed endpoint.
    Exporter(opentelemetry_otlp::ExporterBuildError),
    /// The metrics endpoint is not an `http://` or `https://` URL.
    MetricsEndpoint(String),
    /// The thread that pushes metrics could not be started.
    MetricsThread(std::io::Error),
}

impl std::fmt::Display for OtelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Exporter(e) => write!(f, "could not build the OTLP span exporter: {e}"),
            Self::MetricsEndpoint(url) => {
                write!(f, "the OTLP metrics endpoint `{url}` is not an http(s) URL")
            }
            Self::MetricsThread(e) => write!(f, "could not start the metrics push thread: {e}"),
        }
    }
}

impl std::error::Error for OtelError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Exporter(e) => Some(e),
            Self::MetricsEndpoint(_) => None,
            Self::MetricsThread(e) => Some(e),
        }
    }
}

impl From<opentelemetry_otlp::ExporterBuildError> for OtelError {
    fn from(e: opentelemetry_otlp::ExporterBuildError) -> Self {
        Self::Exporter(e)
    }
}

/// How to talk to the collector, and what to call ourselves.
#[derive(Debug, Clone)]
pub struct OtelConfig {
    /// `service.name` on every span. Required by the OTel semantic conventions
    /// and the first thing you filter by in a collector.
    pub service_name: String,
    /// `service.version`, if you want releases distinguishable in traces.
    pub service_version: Option<String>,
    /// OTLP/gRPC endpoint for **spans**. `None` falls back to
    /// `OTEL_EXPORTER_OTLP_ENDPOINT`, then the OTel default of
    /// `http://localhost:4317`. Metrics have their own, see
    /// [`OtelConfig::metrics_endpoint`].
    pub endpoint: Option<String>,
    /// OTLP/HTTP endpoint for **metrics**, the full URL including `/v1/metrics`.
    /// `None` falls back to `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT`, then
    /// `http://localhost:4318/v1/metrics`.
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

    /// Point spans at a specific OTLP/gRPC endpoint.
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

    /// Where metrics are pushed: explicit, then the environment, then the default.
    fn metrics_url(&self) -> Result<String, OtelError> {
        let url = self
            .metrics_endpoint
            .clone()
            .or_else(|| std::env::var("OTEL_EXPORTER_OTLP_METRICS_ENDPOINT").ok())
            .unwrap_or_else(|| DEFAULT_METRICS_ENDPOINT.to_owned());
        if url.starts_with("http://") || url.starts_with("https://") {
            Ok(url)
        } else {
            Err(OtelError::MetricsEndpoint(url))
        }
    }

    /// The resource attributes metrics carry.
    fn resource_attributes(&self) -> Vec<(String, String)> {
        let mut attributes = vec![("service.name".to_owned(), self.service_name.clone())];
        if let Some(version) = &self.service_version {
            attributes.push(("service.version".to_owned(), version.clone()));
        }
        attributes
    }

    fn resource(&self) -> Resource {
        let mut builder = Resource::builder().with_service_name(self.service_name.clone());
        if let Some(version) = &self.service_version {
            builder = builder.with_attribute(KeyValue::new("service.version", version.clone()));
        }
        builder.build()
    }
}

/// Keeps the pipeline alive and flushes it on the way out.
///
/// Hold it for as long as the server runs. Dropping it flushes on a best-effort
/// basis, but call [`OtelGuard::shutdown`] explicitly where you can — the
/// blocking flush is the only version that reports failure.
#[derive(Debug)]
pub struct OtelGuard {
    provider: SdkTracerProvider,
    pusher: Option<Pusher>,
    instruments: Option<Arc<Instruments>>,
    timeout: Duration,
}

impl OtelGuard {
    /// Flush buffered telemetry and stop the pipeline.
    ///
    /// Idempotent, and safe to call from a shutdown hook.
    pub fn shutdown(&self) {
        if let Err(err) = self.provider.shutdown_with_timeout(self.timeout) {
            // Worth a loud line: it means telemetry was silently dropped, which
            // otherwise looks like the code was never instrumented.
            tracing::error!(%err, "failed to flush spans before shutdown");
        }
        if let Some(pusher) = &self.pusher
            && !pusher.shutdown(self.timeout)
        {
            tracing::error!("failed to flush metrics before shutdown");
        }
    }

    /// Flush without stopping, for a checkpoint mid-run.
    pub fn flush(&self) {
        if let Err(err) = self.provider.force_flush() {
            tracing::warn!(%err, "failed to flush spans");
        }
        if let Some(pusher) = &self.pusher
            && !pusher.flush(self.timeout)
        {
            tracing::warn!("failed to flush metrics");
        }
    }

    /// The instruments this crate records to, when metrics are enabled.
    ///
    /// Share this: `Arc` it into [`metrics::McpMetricsLayer`] for request
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
    /// # use rusty_mcp::otel::{OtelConfig, self};
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

    /// The underlying provider, for anything this wrapper does not cover.
    pub fn provider(&self) -> &SdkTracerProvider {
        &self.provider
    }
}

impl Drop for OtelGuard {
    fn drop(&mut self) {
        // Best effort. An explicit `shutdown()` is better because it can report
        // failure; this only catches the case where nobody called it.
        let _ = self.provider.shutdown_with_timeout(self.timeout);
        if let Some(pusher) = &self.pusher {
            let _ = pusher.shutdown(self.timeout);
        }
    }
}

/// Start an OTLP pipeline and hand back its tracer, installing nothing.
///
/// Use this when the process already builds its own subscriber — a common case,
/// since [`init`] can only win once per process and a second call would leave
/// the second provider fed by nothing:
///
/// ```no_run
/// use tracing_subscriber::prelude::*;
/// # use rusty_mcp::otel::OtelConfig;
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let (guard, tracer) = rusty_mcp::otel::pipeline(OtelConfig::new("my-server"))?;
///
/// tracing_subscriber::registry()
///     .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
///     .with(tracing_opentelemetry::layer().with_tracer(tracer))
///     .init();
/// # let _ = guard;
/// # Ok(())
/// # }
/// ```
pub fn pipeline(
    config: OtelConfig,
) -> Result<(OtelGuard, opentelemetry_sdk::trace::SdkTracer), OtelError> {
    let mut exporter = SpanExporter::builder().with_tonic();
    if let Some(endpoint) = &config.endpoint {
        exporter = exporter.with_endpoint(endpoint.clone());
    }
    let exporter = exporter.build()?;

    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(config.resource())
        // Parent-based: honour the caller's decision, and fall back to the
        // ratio only for traces that start here.
        .with_sampler(Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(
            config.sample_ratio,
        ))))
        .build();

    let tracer = provider.tracer(config.service_name.clone());

    let (pusher, instruments) = if config.metrics {
        let instruments = Arc::new(Instruments::new());
        let pusher = Pusher::spawn(PushSettings {
            endpoint: config.metrics_url()?,
            resource: config.resource_attributes(),
            scope: SCOPE,
            interval: config.metrics_interval,
            request_timeout: config.shutdown_timeout,
            registry: Arc::clone(instruments.registry()),
        })
        .map_err(OtelError::MetricsThread)?;
        (Some(pusher), Some(instruments))
    } else {
        (None, None)
    };

    Ok((
        OtelGuard {
            provider,
            pusher,
            instruments,
            timeout: config.shutdown_timeout,
        },
        tracer,
    ))
}

/// Start an OTLP pipeline and install a subscriber that feeds it.
///
/// Installs the global subscriber, with both a stderr layer (so stdio servers
/// still log where they should) and the OpenTelemetry layer.
///
/// `filter` is the fallback log directive; `RUST_LOG` wins when set.
pub fn init(config: OtelConfig, filter: &str) -> Result<OtelGuard, OtelError> {
    let service_name = config.service_name.clone();
    let endpoint = config.endpoint.clone();
    let sample_ratio = config.sample_ratio;

    let (guard, tracer) = pipeline(config)?;

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(filter));

    let installed = tracing_subscriber::registry()
        .with(fmt::layer().with_writer(std::io::stderr).with_ansi(false))
        .with(tracing_opentelemetry::layer().with_tracer(tracer))
        .with(env_filter)
        .try_init();

    if installed.is_err() {
        // A subscriber was already installed, so spans will not reach the layer
        // we just built. Worth saying plainly rather than exporting nothing and
        // leaving the operator to wonder.
        tracing::warn!("a tracing subscriber was already installed; spans will not be exported");
    }

    tracing::info!(
        service = %service_name,
        endpoint = endpoint.as_deref().unwrap_or("<default>"),
        sample_ratio,
        "exporting spans over OTLP"
    );

    Ok(guard)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn the_resource_carries_the_service_name() {
        let resource = OtelConfig::new("my-server")
            .with_service_version("1.2.3")
            .resource();

        let name = resource
            .get(&opentelemetry::Key::from_static_str("service.name"))
            .map(|v| v.to_string());
        assert_eq!(name.as_deref(), Some("my-server"));

        let version = resource
            .get(&opentelemetry::Key::from_static_str("service.version"))
            .map(|v| v.to_string());
        assert_eq!(version.as_deref(), Some("1.2.3"));
    }

    #[test]
    fn defaults_sample_everything_and_use_the_env_endpoint() {
        let config = OtelConfig::new("s");
        assert_eq!(config.sample_ratio, 1.0);
        assert!(config.endpoint.is_none(), "None means fall back to the env");
    }
}
