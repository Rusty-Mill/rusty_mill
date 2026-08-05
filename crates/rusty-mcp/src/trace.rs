//! W3C trace context propagation over MCP `_meta` (SEP-414).
//!
//! The 2026-07-28 spec reserves three bare `_meta` keys — `traceparent`,
//! `tracestate` and `baggage` — as an explicit exception to the reverse-DNS
//! prefix rule, so MCP interoperates with existing OpenTelemetry tooling.
//!
//! `rmcp` carries those strings; this module gives them meaning: strict
//! parsing, a `tracing` span whose fields carry the ids, and serialization
//! back out for onward calls.
//!
//! ```no_run
//! # use rmcp::service::{RequestContext, RoleServer};
//! use rusty_mcp::trace::TraceContext;
//!
//! # fn example(ctx: &RequestContext<RoleServer>) {
//! let span = TraceContext::from_request(ctx)
//!     .map(|tc| tc.span("tools/call"))
//!     .unwrap_or_else(|| tracing::info_span!("tools/call"));
//! let _guard = span.enter();
//! tracing::info!("handling the call");
//! # }
//! ```
//!
//! Every log line inside that span carries `trace_id` and `parent_span_id`, so
//! a request can be followed across the client, this server, and whatever it
//! calls next.
//!
//! # Invalid input restarts the trace
//!
//! A malformed `traceparent` is treated as **absent**, not as an error: W3C
//! requires the receiver to start a new trace rather than propagate something
//! it could not parse. Returning an error instead would let a broken upstream
//! take a server down, and propagating the raw bytes would corrupt every trace
//! downstream.
//!
//! # Baggage is untrusted
//!
//! Baggage crosses service boundaries unauthenticated, so anyone who can reach
//! the client can put values in it. Use it for diagnostics; never for
//! authorization decisions. [`Baggage`] caps entries and length per W3C so an
//! oversized header cannot be used to exhaust memory.

use std::collections::BTreeMap;

use rmcp::{
    model::RequestParamsMeta,
    service::{RequestContext, RoleServer},
};

/// Maximum baggage entries (W3C Baggage §3.2.1).
const MAX_BAGGAGE_ENTRIES: usize = 180;

/// Maximum total baggage length in bytes (W3C Baggage §3.2.1).
const MAX_BAGGAGE_BYTES: usize = 8192;

/// A parsed W3C `traceparent`, plus the `tracestate` and `baggage` beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceContext {
    /// 16-byte trace id, lowercase hex. Never all zeros.
    trace_id: String,
    /// 8-byte span id of the caller, lowercase hex. Never all zeros.
    parent_span_id: String,
    /// Trace flags byte; bit 0 is "sampled".
    trace_flags: u8,
    /// Vendor state, passed through unmodified.
    tracestate: Option<String>,
    /// Parsed baggage.
    baggage: Baggage,
}

impl TraceContext {
    /// Parse the trace context out of a request's `_meta`.
    ///
    /// `None` when there is no `traceparent`, or when it is malformed — the
    /// caller should start a fresh trace in both cases.
    pub fn from_request(context: &RequestContext<RoleServer>) -> Option<Self> {
        Self::from_parts(
            context.meta.get_traceparent()?,
            context.meta.get_tracestate(),
            context.meta.get_baggage(),
        )
    }

    /// Parse from anything carrying `_meta`, such as a request params struct.
    pub fn from_meta<M: RequestParamsMeta>(carrier: &M) -> Option<Self> {
        Self::from_parts(
            carrier.traceparent()?,
            carrier.tracestate(),
            carrier.baggage(),
        )
    }

    /// Parse the three header values directly.
    pub fn from_parts(
        traceparent: &str,
        tracestate: Option<&str>,
        baggage: Option<&str>,
    ) -> Option<Self> {
        let (trace_id, parent_span_id, trace_flags) = parse_traceparent(traceparent)?;

        Some(Self {
            trace_id,
            parent_span_id,
            trace_flags,
            // `tracestate` travels with a valid `traceparent` and is opaque to
            // us; pass it along untouched.
            tracestate: tracestate.map(str::to_string),
            baggage: baggage.map(Baggage::parse).unwrap_or_default(),
        })
    }

    /// The trace id, lowercase hex.
    pub fn trace_id(&self) -> &str {
        &self.trace_id
    }

    /// The caller's span id — the parent for work done here.
    pub fn parent_span_id(&self) -> &str {
        &self.parent_span_id
    }

    /// Whether the caller sampled this trace.
    ///
    /// A server that samples its own telemetry should honour this rather than
    /// deciding independently, or a trace ends up half-recorded.
    pub fn is_sampled(&self) -> bool {
        self.trace_flags & 0x01 != 0
    }

    /// Raw trace flags byte.
    pub fn trace_flags(&self) -> u8 {
        self.trace_flags
    }

    /// Vendor `tracestate`, if any.
    pub fn tracestate(&self) -> Option<&str> {
        self.tracestate.as_deref()
    }

    /// Parsed baggage.
    pub fn baggage(&self) -> &Baggage {
        &self.baggage
    }

    /// Render as a `traceparent` header value.
    pub fn to_traceparent(&self) -> String {
        format!(
            "00-{}-{}-{:02x}",
            self.trace_id, self.parent_span_id, self.trace_flags
        )
    }

    /// A context describing a child span of this one, for an onward call.
    ///
    /// `span_id` must be 16 lowercase hex digits and not all zeros — generate
    /// it with whatever your tracing stack uses.
    pub fn child(&self, span_id: &str) -> Option<Self> {
        valid_hex(span_id, 16).then(|| Self {
            trace_id: self.trace_id.clone(),
            parent_span_id: span_id.to_ascii_lowercase(),
            trace_flags: self.trace_flags,
            tracestate: self.tracestate.clone(),
            baggage: self.baggage.clone(),
        })
    }

    /// Write this context into a `_meta` carrier, for an outbound request.
    pub fn apply_to<M: RequestParamsMeta>(&self, carrier: &mut M) {
        carrier.set_traceparent(&self.to_traceparent());
        if let Some(tracestate) = &self.tracestate {
            carrier.set_tracestate(tracestate);
        }
        if !self.baggage.is_empty() {
            carrier.set_baggage(&self.baggage.to_header_value());
        }
    }

    /// An `INFO` span named `name`, carrying the trace ids as fields.
    ///
    /// Fields rather than a real parent link: `tracing` alone cannot adopt a
    /// remote parent. This is enough to correlate logs, and a
    /// `tracing-opentelemetry` layer can build the real link from the same ids.
    pub fn span(&self, name: &'static str) -> tracing::Span {
        tracing::info_span!(
            "mcp.request",
            otel.name = name,
            trace_id = %self.trace_id,
            parent_span_id = %self.parent_span_id,
            sampled = self.is_sampled(),
        )
    }
}

/// Parse a W3C `traceparent`, returning `(trace_id, parent_id, flags)`.
///
/// Rejects the all-zero trace and span ids, and version `ff`. Unknown future
/// versions are accepted by reading only the first four fields, which is what
/// W3C requires for forward compatibility.
fn parse_traceparent(value: &str) -> Option<(String, String, u8)> {
    let value = value.trim();
    let mut fields = value.split('-');

    let version = fields.next()?;
    let trace_id = fields.next()?;
    let parent_id = fields.next()?;
    let flags = fields.next()?;

    if !valid_hex(version, 2) || version.eq_ignore_ascii_case("ff") {
        return None;
    }

    // Version 00 is exactly four fields; later versions may append more.
    let is_v0 = version == "00";
    if is_v0 && fields.next().is_some() {
        return None;
    }

    if !valid_hex(trace_id, 32) || !valid_hex(parent_id, 16) || !valid_hex(flags, 2) {
        return None;
    }

    // All-zero ids are explicitly invalid: they mean "no trace", and treating
    // them as real would merge unrelated requests into one trace.
    if trace_id.bytes().all(|b| b == b'0') || parent_id.bytes().all(|b| b == b'0') {
        return None;
    }

    Some((
        trace_id.to_ascii_lowercase(),
        parent_id.to_ascii_lowercase(),
        u8::from_str_radix(flags, 16).ok()?,
    ))
}

/// Exactly `len` hex digits.
fn valid_hex(value: &str, len: usize) -> bool {
    value.len() == len && value.bytes().all(|b| b.is_ascii_hexdigit())
}

/// W3C Baggage: cross-cutting key/value pairs travelling with a trace.
///
/// Treat every value as untrusted input from whoever reached the client.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Baggage {
    entries: BTreeMap<String, String>,
}

impl Baggage {
    /// An empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse a `baggage` header value.
    ///
    /// Malformed members are skipped rather than failing the whole set — one
    /// bad entry from an upstream should not discard the rest. Values are
    /// percent-decoded; properties after `;` are dropped, since nothing here
    /// consumes them.
    pub fn parse(value: &str) -> Self {
        let mut entries = BTreeMap::new();

        if value.len() > MAX_BAGGAGE_BYTES {
            tracing::debug!(len = value.len(), "ignoring oversized baggage header");
            return Self { entries };
        }

        for member in value.split(',') {
            if entries.len() >= MAX_BAGGAGE_ENTRIES {
                tracing::debug!("baggage entry limit reached, ignoring the rest");
                break;
            }

            // Properties (`;k=v`) are metadata about the member; drop them.
            let member = member.split(';').next().unwrap_or("").trim();
            if member.is_empty() {
                continue;
            }

            let Some((key, raw)) = member.split_once('=') else {
                continue;
            };

            let key = key.trim();
            if key.is_empty() {
                continue;
            }

            entries.insert(
                key.to_string(),
                percent_encoding::percent_decode_str(raw.trim())
                    .decode_utf8_lossy()
                    .into_owned(),
            );
        }

        Self { entries }
    }

    /// Look a value up.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(String::as_str)
    }

    /// Insert a value.
    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.entries.insert(key.into(), value.into());
    }

    /// Whether there are no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// How many entries there are.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Iterate over the entries.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Render as a `baggage` header value, percent-encoding values.
    pub fn to_header_value(&self) -> String {
        self.entries
            .iter()
            .map(|(key, value)| {
                let encoded = percent_encoding::utf8_percent_encode(
                    value,
                    percent_encoding::NON_ALPHANUMERIC,
                );
                format!("{key}={encoded}")
            })
            .collect::<Vec<_>>()
            .join(",")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = "00-0af7651916cd43dd8448eb211c80319c-00f067aa0ba902b7-01";

    #[test]
    fn parses_the_spec_example() {
        let tc = TraceContext::from_parts(VALID, None, None).expect("valid");

        assert_eq!(tc.trace_id(), "0af7651916cd43dd8448eb211c80319c");
        assert_eq!(tc.parent_span_id(), "00f067aa0ba902b7");
        assert!(tc.is_sampled());
    }

    #[test]
    fn round_trips_through_the_header_form() {
        let tc = TraceContext::from_parts(VALID, None, None).expect("valid");
        assert_eq!(tc.to_traceparent(), VALID);
    }

    #[test]
    fn normalizes_uppercase_hex() {
        let tc = TraceContext::from_parts(
            "00-0AF7651916CD43DD8448EB211C80319C-00F067AA0BA902B7-01",
            None,
            None,
        )
        .expect("valid");
        assert_eq!(tc.trace_id(), "0af7651916cd43dd8448eb211c80319c");
    }

    #[test]
    fn reads_the_sampled_flag() {
        let unsampled = VALID.replace("-01", "-00");
        assert!(
            !TraceContext::from_parts(&unsampled, None, None)
                .expect("valid")
                .is_sampled()
        );
    }

    #[test]
    fn rejects_all_zero_ids() {
        // These mean "no trace"; accepting them would merge unrelated
        // requests into a single bogus trace.
        assert!(
            TraceContext::from_parts(
                "00-00000000000000000000000000000000-00f067aa0ba902b7-01",
                None,
                None
            )
            .is_none()
        );
        assert!(
            TraceContext::from_parts(
                "00-0af7651916cd43dd8448eb211c80319c-0000000000000000-01",
                None,
                None
            )
            .is_none()
        );
    }

    #[test]
    fn rejects_malformed_traceparents() {
        for bad in [
            "",
            "not-a-traceparent",
            // Wrong field lengths.
            "00-0af7651916cd43dd-00f067aa0ba902b7-01",
            "00-0af7651916cd43dd8448eb211c80319c-00f067aa-01",
            // Non-hex.
            "00-0af7651916cd43dd8448eb211c80319g-00f067aa0ba902b7-01",
            // Version ff is reserved and invalid.
            "ff-0af7651916cd43dd8448eb211c80319c-00f067aa0ba902b7-01",
            // Version 00 must have exactly four fields.
            "00-0af7651916cd43dd8448eb211c80319c-00f067aa0ba902b7-01-extra",
            // Missing fields.
            "00-0af7651916cd43dd8448eb211c80319c-00f067aa0ba902b7",
        ] {
            assert!(
                TraceContext::from_parts(bad, None, None).is_none(),
                "should have rejected `{bad}`"
            );
        }
    }

    #[test]
    fn accepts_unknown_future_versions() {
        // W3C forward compatibility: read the first four fields, ignore extras.
        let tc = TraceContext::from_parts(
            "01-0af7651916cd43dd8448eb211c80319c-00f067aa0ba902b7-01-something",
            None,
            None,
        )
        .expect("a future version should still parse");
        assert_eq!(tc.trace_id(), "0af7651916cd43dd8448eb211c80319c");
    }

    #[test]
    fn carries_tracestate_through_unmodified() {
        let tc =
            TraceContext::from_parts(VALID, Some("vendor=opaque,other=1"), None).expect("valid");
        assert_eq!(tc.tracestate(), Some("vendor=opaque,other=1"));
    }

    #[test]
    fn builds_a_child_context() {
        let tc = TraceContext::from_parts(VALID, None, None).expect("valid");
        let child = tc.child("1111111111111111").expect("valid span id");

        // Same trace, new parent.
        assert_eq!(child.trace_id(), tc.trace_id());
        assert_eq!(child.parent_span_id(), "1111111111111111");
    }

    #[test]
    fn rejects_a_bad_child_span_id() {
        let tc = TraceContext::from_parts(VALID, None, None).expect("valid");
        assert!(tc.child("nope").is_none());
        assert!(tc.child("111").is_none());
    }

    #[test]
    fn parses_baggage() {
        let baggage = Baggage::parse("userId=alice,serverNode=DF%2028,isProduction=false");

        assert_eq!(baggage.get("userId"), Some("alice"));
        // Percent-decoded.
        assert_eq!(baggage.get("serverNode"), Some("DF 28"));
        assert_eq!(baggage.get("isProduction"), Some("false"));
    }

    #[test]
    fn drops_baggage_properties() {
        let baggage = Baggage::parse("key1=value1;property1;property2,key2=value2");
        assert_eq!(baggage.get("key1"), Some("value1"));
        assert_eq!(baggage.get("key2"), Some("value2"));
    }

    #[test]
    fn skips_malformed_baggage_members_but_keeps_the_rest() {
        let baggage = Baggage::parse("good=1,nonsense,=2,also-good=3");
        assert_eq!(baggage.get("good"), Some("1"));
        assert_eq!(baggage.get("also-good"), Some("3"));
        assert_eq!(baggage.len(), 2);
    }

    #[test]
    fn ignores_oversized_baggage() {
        // A cap is the difference between a diagnostic aid and a memory
        // exhaustion vector, since baggage is attacker-controlled.
        let huge = format!("k={}", "a".repeat(MAX_BAGGAGE_BYTES));
        assert!(Baggage::parse(&huge).is_empty());
    }

    #[test]
    fn caps_the_number_of_baggage_entries() {
        let many = (0..MAX_BAGGAGE_ENTRIES + 50)
            .map(|i| format!("k{i}=v"))
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(Baggage::parse(&many).len(), MAX_BAGGAGE_ENTRIES);
    }

    #[test]
    fn baggage_round_trips() {
        let mut baggage = Baggage::new();
        baggage.insert("userId", "alice");
        baggage.insert("node", "DF 28");

        let reparsed = Baggage::parse(&baggage.to_header_value());
        assert_eq!(reparsed.get("userId"), Some("alice"));
        assert_eq!(reparsed.get("node"), Some("DF 28"));
    }

    #[test]
    fn empty_baggage_renders_empty() {
        assert!(Baggage::new().to_header_value().is_empty());
        assert!(Baggage::parse("").is_empty());
    }
}
