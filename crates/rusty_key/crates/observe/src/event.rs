//! The unified lifecycle event ([`KernelEvent`]) and its pull-based OTLP
//! exporter (ADR-0034, Phase 7B).
//!
//! `KernelEvent` is the single fixed enum the turn cycle emits. Two consumers
//! subscribe to the same stream — the [`Tracer`](crate::Tracer), which builds
//! the rich `Episode`, and the [`OtlpExporter`], which accumulates
//! token/cost/latency telemetry. Neither owns the wire format, so adding an
//! observability backend never changes the kernel contract.
//!
//! The exporter is **pull-based**: it accumulates metrics in-process at the
//! boundary and exposes a [`OtlpExporter::snapshot`] for a collector/operator to
//! scrape. Nothing is pushed from inside a tool side-effect, so a future
//! `sandboxed` `ToolExecutor` (ADR-0030) cannot blind operators. The event
//! never carries tool arguments, so telemetry cannot leak secrets.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::ToolStatus;

/// The fixed lifecycle enum every turn emits (ADR-0034). New lifecycle points
/// add a variant here rather than introducing a second event type, keeping the
/// `Tracer` and the exporter in lockstep over one schema. Deliberately carries
/// no tool arguments — only the attributes telemetry needs.
#[derive(Debug, Clone, PartialEq)]
pub enum KernelEvent {
    /// A new turn began; consumers reset per-turn state.
    TurnStart,
    /// A policy-vetted tool dispatch returned with this status.
    ToolReturn {
        /// Tool name.
        name: String,
        /// Structural outcome status (never re-parsed from a string).
        status: ToolStatus,
    },
    /// The turn finished. Carries the token/cost/latency attributes.
    TurnEnd {
        /// Resolved per-turn token usage.
        tokens: u64,
        /// Per-turn cost in USD (0.0 until a pricing table lands).
        cost_usd: f64,
        /// Wall-clock turn latency in milliseconds.
        latency_ms: u64,
    },
    /// The turn errored before producing a reply.
    Error {
        /// Human-readable error message (no secrets).
        message: String,
    },
}

/// Accumulated telemetry, exposed for a pull (scrape). All counters are
/// monotonic over the exporter's lifetime.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    /// Configured OTLP endpoint, if any (echoed for operator visibility).
    pub endpoint: Option<String>,
    /// Whether the exporter is collecting (an endpoint is configured).
    pub enabled: bool,
    /// Turns started.
    pub turns: u64,
    /// Tool dispatches returned (any status).
    pub tool_calls: u64,
    /// Tool returns by status: `ok`.
    pub tool_ok: u64,
    /// Tool returns by status: `error`.
    pub tool_error: u64,
    /// Tool returns by status: `blocked`.
    pub tool_blocked: u64,
    /// Tool returns by status: `timeout`.
    pub tool_timeout: u64,
    /// Tool returns by status: `truncated`.
    pub tool_truncated: u64,
    /// Turn-level errors observed.
    pub errors: u64,
    /// Cumulative resolved token usage.
    pub total_tokens: u64,
    /// Cumulative cost in USD.
    pub total_cost_usd: f64,
    /// Cumulative turn latency in milliseconds.
    pub total_latency_ms: u64,
}

/// Pull-based exporter bound to the [`KernelEvent`] stream. Inert (a cheap
/// no-op) until `RUSTYKEYS_OTLP_ENDPOINT` is set; even then it never pushes —
/// telemetry is collected here and read via [`OtlpExporter::snapshot`].
pub struct OtlpExporter {
    endpoint: Option<String>,
    metrics: Mutex<MetricsSnapshot>,
}

impl OtlpExporter {
    /// Build an exporter for the given endpoint. `None` ⇒ inert.
    pub fn new(endpoint: Option<String>) -> Self {
        let metrics = MetricsSnapshot {
            endpoint: endpoint.clone(),
            enabled: endpoint.is_some(),
            ..Default::default()
        };
        Self {
            endpoint,
            metrics: Mutex::new(metrics),
        }
    }

    /// True when an endpoint is configured and the exporter is collecting.
    pub fn is_enabled(&self) -> bool {
        self.endpoint.is_some()
    }

    /// The configured collector endpoint, if any.
    pub fn endpoint(&self) -> Option<&str> {
        self.endpoint.as_deref()
    }

    /// Fold one lifecycle event into the accumulated metrics. A no-op when the
    /// exporter is inert.
    pub fn observe(&self, event: &KernelEvent) {
        if !self.is_enabled() {
            return;
        }
        let mut m = self.metrics.lock().unwrap_or_else(|p| p.into_inner());
        match event {
            KernelEvent::TurnStart => m.turns += 1,
            KernelEvent::ToolReturn { status, .. } => {
                m.tool_calls += 1;
                match status {
                    ToolStatus::Ok => m.tool_ok += 1,
                    ToolStatus::Error => m.tool_error += 1,
                    ToolStatus::Blocked => m.tool_blocked += 1,
                    ToolStatus::Timeout => m.tool_timeout += 1,
                    ToolStatus::Truncated => m.tool_truncated += 1,
                }
            }
            KernelEvent::TurnEnd {
                tokens,
                cost_usd,
                latency_ms,
            } => {
                m.total_tokens += tokens;
                m.total_cost_usd += cost_usd;
                m.total_latency_ms += latency_ms;
            }
            KernelEvent::Error { .. } => m.errors += 1,
        }
    }

    /// Pull the accumulated metrics. This is the scrape surface: a collector or
    /// operator reads it from the host boundary, regardless of tool isolation.
    pub fn snapshot(&self) -> MetricsSnapshot {
        self.metrics
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inert_without_endpoint() {
        let exp = OtlpExporter::new(None);
        assert!(!exp.is_enabled());
        exp.observe(&KernelEvent::TurnStart);
        exp.observe(&KernelEvent::ToolReturn {
            name: "read_file".into(),
            status: ToolStatus::Ok,
        });
        exp.observe(&KernelEvent::TurnEnd {
            tokens: 100,
            cost_usd: 0.0,
            latency_ms: 5,
        });
        let s = exp.snapshot();
        assert!(!s.enabled);
        assert_eq!(s.turns, 0);
        assert_eq!(s.tool_calls, 0);
        assert_eq!(s.total_tokens, 0);
    }

    #[test]
    fn accumulates_when_enabled() {
        let exp = OtlpExporter::new(Some("http://localhost:4317".into()));
        assert!(exp.is_enabled());
        assert_eq!(exp.endpoint(), Some("http://localhost:4317"));

        exp.observe(&KernelEvent::TurnStart);
        exp.observe(&KernelEvent::ToolReturn {
            name: "read_file".into(),
            status: ToolStatus::Ok,
        });
        exp.observe(&KernelEvent::ToolReturn {
            name: "write".into(),
            status: ToolStatus::Blocked,
        });
        exp.observe(&KernelEvent::Error {
            message: "boom".into(),
        });
        exp.observe(&KernelEvent::TurnEnd {
            tokens: 1423,
            cost_usd: 0.0,
            latency_ms: 42,
        });

        let s = exp.snapshot();
        assert!(s.enabled);
        assert_eq!(s.endpoint.as_deref(), Some("http://localhost:4317"));
        assert_eq!(s.turns, 1);
        assert_eq!(s.tool_calls, 2);
        assert_eq!(s.tool_ok, 1);
        assert_eq!(s.tool_blocked, 1);
        assert_eq!(s.errors, 1);
        assert_eq!(s.total_tokens, 1423);
        assert_eq!(s.total_latency_ms, 42);
    }

    #[test]
    fn snapshot_round_trips_json() {
        let exp = OtlpExporter::new(Some("http://c:4317".into()));
        exp.observe(&KernelEvent::TurnStart);
        let s = exp.snapshot();
        let json = serde_json::to_string(&s).unwrap();
        let back: MetricsSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back.turns, 1);
        assert!(back.enabled);
    }
}
