//! A telemetry span per MCP request, and what a guardrail can write on it.
//!
//! An `mcpGuardrails` processor answers with a bag of values in
//! `McpRequestResult.metadata` — the classification it made, the rule it
//! matched, the tenant it resolved. Upstream stashes that for downstream CEL
//! filters. This gateway has none, so it puts the bag on the request's span
//! instead: a decision a guardrail took in-band becomes visible in the trace
//! afterwards, rather than only in the processor's own logs.
//!
//! # Why the span is created here
//!
//! There was not one before. `tracing` fields have to be declared when a span
//! is opened, so arbitrary processor-supplied keys cannot be recorded as
//! fields at all; they go on as OpenTelemetry attributes, which need an
//! OpenTelemetry span to go on. Opening one per request is what makes that
//! possible, and it is worth having on its own — a `tools/call` that took four
//! seconds is not visible anywhere otherwise.
//!
//! # The span is not entered
//!
//! It is created and held for the length of the call, never `enter()`ed. The
//! OpenTelemetry layer times a span from creation to close rather than from
//! enter to exit, so the duration is right either way — and entering would be
//! actively wrong here. `Span::enter` returns a guard tied to the thread, and
//! holding one across an `.await` in a server that multiplexes tasks onto a
//! thread pool attributes another request's events to this span.
//!
//! The trade is that log lines inside a handler are not nested under the span.
//! Correct timing and correct attribution beat nesting; `#[instrument]` or
//! `Instrument` would give all three, at the cost of restructuring every
//! handler around an inner function.
//!
//! # When no collector is configured
//!
//! Setting an attribute is a no-op unless the OpenTelemetry layer is
//! installed, which it is only when `config.tracing` names a collector. The
//! span itself still exists, so log lines inside it are still correlated.

use rmcp::service::{RequestContext, RoleServer};
use tracing::Span;

use crate::guardrails::Annotations;

/// Prefix for attributes a processor asked for.
///
/// Namespaced so a guardrail cannot collide with the gateway's own attributes
/// or with anything OpenTelemetry reserves, and named for the policy that
/// produced it so an operator reading a trace knows where it came from.
const PREFIX: &str = "mcpGuardrails.";

/// Open a span for one MCP request.
///
/// When the client propagated a W3C trace context in `_meta`, the span carries
/// its ids so this request joins the caller's trace rather than starting a new
/// one. A malformed context is treated as absent, which is what W3C requires.
pub fn request(method: &'static str, context: &RequestContext<RoleServer>) -> Span {
    match rusty_mcp::trace::TraceContext::from_request(context) {
        Some(trace) => trace.span(method),
        None => tracing::info_span!("mcp.request", otel.name = method),
    }
}

/// Put a processor's annotations on `span` as OpenTelemetry attributes.
///
/// Scalars go on natively so a trace viewer can filter on them. Anything
/// nested is rendered as JSON: OpenTelemetry attribute values are scalars and
/// homogeneous arrays, and flattening an arbitrary object into dotted keys
/// would invent structure the processor did not ask for.
pub fn annotate(span: &Span, annotations: &Annotations) {
    if annotations.is_empty() {
        return;
    }

    use opentelemetry::Value;
    use tracing_opentelemetry::OpenTelemetrySpanExt as _;

    for (key, value) in annotations.iter() {
        let value = match value {
            serde_json::Value::String(text) => Value::String(text.clone().into()),
            serde_json::Value::Bool(flag) => Value::Bool(*flag),
            serde_json::Value::Number(number) => match number.as_i64() {
                Some(int) => Value::I64(int),
                None => match number.as_f64() {
                    Some(float) => Value::F64(float),
                    None => Value::String(number.to_string().into()),
                },
            },
            // Null has no attribute form; the empty string is the closest
            // honest rendering and keeps the key visible.
            serde_json::Value::Null => Value::String("".into()),
            nested => Value::String(nested.to_string().into()),
        };
        span.set_attribute(format!("{PREFIX}{key}"), value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attributes_are_namespaced_by_the_policy_that_produced_them() {
        // The prefix is the contract an operator reads a trace against, so it
        // is pinned rather than left to whatever the format string says.
        assert_eq!(PREFIX, "mcpGuardrails.");
        assert_eq!(
            format!("{PREFIX}classification"),
            "mcpGuardrails.classification"
        );
    }

    #[test]
    fn annotating_nothing_is_a_no_op() {
        // Called on every request that runs a request-phase processor, so the
        // empty case has to cost nothing and must not panic outside a span.
        annotate(&Span::none(), &Annotations::default());
    }
}
