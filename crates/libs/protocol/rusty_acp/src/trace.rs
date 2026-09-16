//! W3C trace context, carried across the replica boundary.
//!
//! #16 gave every run a span. Every one of them was a *root*: nothing read a
//! `traceparent` on the way in and nothing wrote one on the way out, so the
//! spans a single client call produced were unrelated islands.
//!
//! For a single-process server that is a nuisance. This crate's whole premise
//! is the opposite — identical replicas behind a load balancer, no session
//! affinity — so the ordinary case is a run **created** through replica A,
//! **executing** on replica B, **observed** through C and **cancelled** through
//! A again. The question anyone opens a log for, *what happened to this run and
//! which replica was holding it*, was exactly the one that could not be asked.
//!
//! # Correlation by field, not by span tree
//!
//! A [`TraceContext`] is recorded as `trace_id` on the request span and on the
//! run span, and the two are siblings rather than parent and child.
//!
//! That is not a shortcut, it is forced. An `async` or `stream` run **outlives
//! the request that created it** — that is what those modes are for — so the
//! run cannot be a child of a request span that has already closed. And a
//! cancellation arriving at another replica is a different client call
//! entirely; in OpenTelemetry it would be a span *link*, which is a concept
//! `tracing` has no vocabulary for.
//!
//! A shared field answers the operational question regardless: every line every
//! replica logs about one client call carries the same `trace_id`, which is
//! what makes them greppable as a unit.
//!
//! # Why not `tracing-opentelemetry`
//!
//! It would give real parent-child spans and real links. It is also a large
//! dependency on a fast-moving stack, and taking it would pick the
//! observability ecosystem for everybody who depends on this crate. The same
//! reasoning that made [`metrics`](crate::server) a facade rather than an
//! exporter applies here: emit the identifiers, and leave what collects them to
//! the deployment.
//!
//! The format is a fixed 55 characters and reading it costs nothing:
//!
//! ```text
//! 00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01
//! │  │                                │                └ flags
//! │  │                                └ parent span id
//! │  └ trace id
//! └ version
//! ```

use std::fmt;

/// The header W3C trace context travels in.
pub const TRACEPARENT_HEADER: &str = "traceparent";

/// A trace identifier and the span within it that this request follows.
///
/// Parsed from an incoming [`TRACEPARENT_HEADER`], or minted when there is
/// none. Cheap to copy and carries no allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceContext {
    /// 16 bytes identifying the whole trace. Never all zero.
    trace_id: [u8; 16],
    /// 8 bytes identifying the span this request is a child of. Never all zero.
    span_id: [u8; 8],
    /// Whether the trace is sampled. Propagated unchanged: this crate does not
    /// make sampling decisions, it only refuses to lose one.
    sampled: bool,
}

impl TraceContext {
    /// Mint a fresh context, for a call that arrived without one.
    ///
    /// The bytes come from [`uuid::Uuid::new_v4`] rather than a new `rand`
    /// dependency — `uuid` is already here for every identifier in the
    /// protocol, and what is wanted is 24 bytes no other caller will produce,
    /// which is exactly what it provides.
    pub fn mint() -> Self {
        let trace_id = *uuid::Uuid::new_v4().as_bytes();
        let mut span_id = [0u8; 8];
        span_id.copy_from_slice(&uuid::Uuid::new_v4().as_bytes()[..8]);
        // A v4 UUID cannot be all zeros in practice, but the spec makes an
        // all-zero id invalid and a silently invalid header is worse than an
        // obviously arbitrary one.
        Self {
            trace_id: if trace_id == [0; 16] { [1; 16] } else { trace_id },
            span_id: if span_id == [0; 8] { [1; 8] } else { span_id },
            sampled: true,
        }
    }

    /// Parse a `traceparent` header value, or `None` if it is not one.
    ///
    /// Unparseable input mints nothing and reports nothing: the caller decides
    /// whether to mint, and treating a malformed header as absent is what the
    /// W3C specification asks for. Refusing the request instead would let a
    /// broken upstream proxy take a server down over a field nothing depends
    /// on.
    ///
    /// Later versions are accepted so long as the first four fields parse,
    /// which is also what the specification requires — a version this crate
    /// does not know may append fields but may not change the ones here.
    pub fn parse(value: &str) -> Option<Self> {
        let mut fields = value.split('-');
        let version = fields.next()?;
        let trace_id = fields.next()?;
        let span_id = fields.next()?;
        let flags = fields.next()?;

        // `ff` is forbidden outright; anything else may be a future version.
        if version.len() != 2 || version == "ff" || !version.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return None;
        }
        if version == "00" && fields.next().is_some() {
            return None;
        }

        let trace_id: [u8; 16] = decode(trace_id)?;
        let span_id: [u8; 8] = decode(span_id)?;
        if trace_id == [0; 16] || span_id == [0; 8] {
            return None;
        }
        let flags: [u8; 1] = decode(flags)?;

        Some(Self { trace_id, span_id, sampled: flags[0] & 0x01 != 0 })
    }

    /// The trace id, as the 32 lowercase hex characters that identify it in
    /// every other system that will see it.
    ///
    /// This is the value recorded on spans, and the one to search for.
    pub fn trace_id(&self) -> String {
        hex(&self.trace_id)
    }

    /// The span id this request names as its parent.
    pub fn span_id(&self) -> String {
        hex(&self.span_id)
    }

    /// Whether the trace is sampled.
    pub fn is_sampled(&self) -> bool {
        self.sampled
    }

    /// A context in the same trace, naming `span_id` as the new parent.
    ///
    /// What an outbound call sends: the trace is the same journey, the parent
    /// is this hop. Not currently used to build a span tree — see the module
    /// docs on why this crate correlates by field — but the header has to be
    /// well-formed for the systems that do build one from it.
    pub fn child(&self) -> Self {
        let mut span_id = [0u8; 8];
        span_id.copy_from_slice(&uuid::Uuid::new_v4().as_bytes()[..8]);
        Self { span_id: if span_id == [0; 8] { [1; 8] } else { span_id }, ..*self }
    }
}

impl fmt::Display for TraceContext {
    /// The `traceparent` header value: always version `00`, which is the only
    /// version this crate claims to write.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "00-{}-{}-{:02x}",
            hex(&self.trace_id),
            hex(&self.span_id),
            u8::from(self.sampled)
        )
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Decode exactly `N` bytes of lowercase hex, or `None`.
///
/// Uppercase is rejected deliberately: the specification says the fields are
/// lowercase, and a receiver that quietly accepts both is how two systems end
/// up disagreeing about whether two ids are the same string.
fn decode<const N: usize>(text: &str) -> Option<[u8; N]> {
    if text.len() != N * 2 || !text.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()) {
        return None;
    }
    let mut out = [0u8; N];
    for (index, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(text.get(index * 2..index * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

    #[test]
    fn a_header_round_trips() {
        let context = TraceContext::parse(SAMPLE).expect("a valid traceparent");
        assert_eq!(context.trace_id(), "4bf92f3577b34da6a3ce929d0e0e4736");
        assert_eq!(context.span_id(), "00f067aa0ba902b7");
        assert!(context.is_sampled());
        assert_eq!(context.to_string(), SAMPLE);
    }

    /// The whole point of propagating: a child keeps the trace and takes a new
    /// place in it. A `child` that changed the trace id would silently split
    /// every call into two traces.
    #[test]
    fn a_child_keeps_the_trace_and_changes_the_span() {
        let context = TraceContext::parse(SAMPLE).unwrap();
        let child = context.child();
        assert_eq!(child.trace_id(), context.trace_id());
        assert_ne!(child.span_id(), context.span_id());
        assert_eq!(child.is_sampled(), context.is_sampled());
    }

    #[test]
    fn an_unsampled_flag_survives() {
        let unsampled = SAMPLE.replace("-01", "-00");
        let context = TraceContext::parse(&unsampled).unwrap();
        assert!(!context.is_sampled());
        assert_eq!(context.to_string(), unsampled);
    }

    /// Rejected rather than repaired. Every one of these is a header some
    /// upstream got wrong, and inventing a trace id from a broken one is how a
    /// trace silently forks.
    #[test]
    fn malformed_headers_are_not_contexts() {
        for bad in [
            "",
            "00",
            "00-4bf92f3577b34da6a3ce929d0e0e4736",
            // all-zero ids are invalid per the specification
            "00-00000000000000000000000000000000-00f067aa0ba902b7-01",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01",
            // wrong lengths
            "00-4bf92f3577b34da6a3ce929d0e0e473-00f067aa0ba902b7-01",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b-01",
            // uppercase is not lowercase hex
            "00-4BF92F3577B34DA6A3CE929D0E0E4736-00f067aa0ba902b7-01",
            // not hex at all
            "00-zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz-00f067aa0ba902b7-01",
            // version ff is forbidden
            "ff-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
            // version 00 takes exactly four fields
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01-extra",
        ] {
            assert!(
                TraceContext::parse(bad).is_none(),
                "accepted a malformed traceparent: {bad:?}"
            );
        }
    }

    /// A future version may add fields; the four this crate reads keep their
    /// meaning. Rejecting it would make this crate the reason an upgraded
    /// upstream stopped being traceable.
    #[test]
    fn a_later_version_is_still_read() {
        let future = "01-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01-something";
        let context = TraceContext::parse(future).expect("a later version should still parse");
        assert_eq!(context.trace_id(), "4bf92f3577b34da6a3ce929d0e0e4736");
    }

    #[test]
    fn a_minted_context_is_valid_and_distinct() {
        let one = TraceContext::mint();
        let two = TraceContext::mint();
        assert_ne!(one.trace_id(), two.trace_id());
        assert_eq!(TraceContext::parse(&one.to_string()), Some(one));
    }
}
