//! Logging and OpenTelemetry export.
//!
//! Installing the global subscriber is a process-wide, once-only act, so it
//! lives in the binary rather than a library crate. Two shapes:
//!
//! - No `config.tracing`: plain stderr logging, as before.
//! - With it: [`rusty_mcp::otel::init`] stands up an OTLP pipeline and installs
//!   a subscriber feeding both stderr and the collector.
//!
//! # The guard is the whole point
//!
//! Spans and metrics are batched, so whatever is still buffered is lost if the
//! process exits without shutting the provider down — the most common way to
//! end up staring at an empty collector while insisting the code is
//! instrumented. [`Telemetry`] holds the guard and flushes on drop, and the
//! serve loop drops it only after the accept loops have stopped.

use std::sync::Arc;

use agentgateway_config::Config;
use rusty_mcp::otel::{OtelConfig, OtelGuard, metrics::Instruments};

/// Installed telemetry, alive for as long as the gateway is.
pub struct Telemetry {
    guard: Option<Arc<OtelGuard>>,
}

impl Telemetry {
    /// Install logging, and an OTLP pipeline if one is configured.
    ///
    /// `fallback_filter` is used when the config names none; `RUST_LOG` wins
    /// over both.
    pub fn install(config: &Config, fallback_filter: &str) -> anyhow::Result<Self> {
        let filter = config
            .config
            .as_ref()
            .and_then(|c| c.logging.as_ref())
            .and_then(|l| l.filter.clone())
            .unwrap_or_else(|| fallback_filter.to_string());

        let Some(tracing_config) = config.config.as_ref().and_then(|c| c.tracing.as_ref()) else {
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&filter)),
                )
                // stderr, never stdout: a gateway may be launched by something
                // that reads its stdout, and a log line in a protocol stream
                // is a corrupt protocol stream.
                .with_writer(std::io::stderr)
                .init();
            return Ok(Telemetry { guard: None });
        };

        let mut otel = OtelConfig::new(
            tracing_config
                .service_name
                .clone()
                .unwrap_or_else(|| "rusty-agent-gateway".into()),
        );
        if let Some(endpoint) = &tracing_config.endpoint {
            otel = otel.with_endpoint(endpoint);
        }
        if let Some(version) = &tracing_config.service_version {
            otel = otel.with_service_version(version);
        }
        if let Some(ratio) = tracing_config.sample_ratio {
            otel = otel.with_sample_ratio(ratio);
        }
        if tracing_config.metrics == Some(false) {
            otel = otel.without_metrics();
        }

        let guard = rusty_mcp::otel::init(otel, &filter)?;
        tracing::info!(
            endpoint = tracing_config.endpoint.as_deref().unwrap_or("<default>"),
            "exporting traces over OTLP"
        );

        Ok(Telemetry {
            guard: Some(Arc::new(guard)),
        })
    }

    /// The instruments to record request metrics to, if metrics are on.
    pub fn instruments(&self) -> Option<Arc<Instruments>> {
        self.guard
            .as_ref()
            .and_then(|guard| guard.instruments().cloned())
    }

    /// Flush everything buffered. Called before the process exits.
    pub fn shutdown(&self) {
        if let Some(guard) = &self.guard {
            guard.shutdown();
        }
    }
}
