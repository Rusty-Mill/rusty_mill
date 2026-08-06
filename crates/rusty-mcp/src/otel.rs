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
//! instrumented. [`OtelGuard::shutdown_hook`] wires the flush into
//! [`crate::ServerConfig::with_shutdown_hook`] so it happens on SIGTERM.
//!
//! # Sampling follows the caller
//!
//! The default sampler is parent-based: if the caller sampled the trace, so do
//! we; if it did not, we stay quiet. Deciding independently is how traces end
//! up half-recorded, with gaps exactly where a service made its own choice.

use std::time::Duration;

use opentelemetry::{KeyValue, trace::TracerProvider as _};
use opentelemetry_otlp::{SpanExporter, WithExportConfig as _};
use opentelemetry_sdk::{
    Resource,
    trace::{Sampler, SdkTracerProvider},
};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

/// Starting an OTLP pipeline failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum OtelError {
    /// The exporter could not be built — usually a malformed endpoint.
    #[error("could not build the OTLP span exporter: {0}")]
    Exporter(#[from] opentelemetry_otlp::ExporterBuildError),
}

/// How to talk to the collector, and what to call ourselves.
#[derive(Debug, Clone)]
pub struct OtelConfig {
    /// `service.name` on every span. Required by the OTel semantic conventions
    /// and the first thing you filter by in a collector.
    pub service_name: String,
    /// `service.version`, if you want releases distinguishable in traces.
    pub service_version: Option<String>,
    /// OTLP/gRPC endpoint. `None` falls back to `OTEL_EXPORTER_OTLP_ENDPOINT`,
    /// then the OTel default of `http://localhost:4317`.
    pub endpoint: Option<String>,
    /// Fraction of *root* traces to record, 0.0 to 1.0.
    ///
    /// Only consulted when there is no parent decision to follow; a sampled
    /// caller is always honoured.
    pub sample_ratio: f64,
    /// How long `shutdown` waits for the buffer to drain.
    pub shutdown_timeout: Duration,
}

impl OtelConfig {
    /// Config for a service called `service_name`, sampling everything.
    pub fn new(service_name: impl Into<String>) -> Self {
        Self {
            service_name: service_name.into(),
            service_version: None,
            endpoint: None,
            sample_ratio: 1.0,
            shutdown_timeout: Duration::from_secs(5),
        }
    }

    /// Set `service.version`.
    pub fn with_service_version(mut self, version: impl Into<String>) -> Self {
        self.service_version = Some(version.into());
        self
    }

    /// Point at a specific OTLP/gRPC endpoint.
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
    timeout: Duration,
}

impl OtelGuard {
    /// Flush buffered spans and stop the pipeline.
    ///
    /// Idempotent, and safe to call from a shutdown hook.
    pub fn shutdown(&self) {
        if let Err(err) = self.provider.shutdown_with_timeout(self.timeout) {
            // Worth a loud line: it means telemetry was silently dropped, which
            // otherwise looks like the code was never instrumented.
            tracing::error!(%err, "failed to flush spans before shutdown");
        }
    }

    /// Flush without stopping, for a checkpoint mid-run.
    pub fn flush(&self) {
        if let Err(err) = self.provider.force_flush() {
            tracing::warn!(%err, "failed to flush spans");
        }
    }

    /// A shutdown hook for [`crate::ServerConfig::with_shutdown_hook`].
    ///
    /// ```no_run
    /// # use std::sync::Arc;
    /// # use rusty_mcp::{ServerConfig, otel::{OtelConfig, self}};
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let guard = Arc::new(otel::init(OtelConfig::new("my-server"), "info")?);
    /// let config = ServerConfig::stdio().with_shutdown_hook(guard.shutdown_hook());
    /// # let _ = config;
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

    Ok((
        OtelGuard {
            provider,
            timeout: config.shutdown_timeout,
        },
        tracer,
    ))
}

/// Start an OTLP pipeline and install a subscriber that feeds it.
///
/// Replaces [`crate::telemetry::init`] rather than complementing it: this
/// installs the global subscriber, with both a stderr layer (so stdio servers
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
