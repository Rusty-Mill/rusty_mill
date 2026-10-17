//! What a finished span is, with no SDK behind it.
//!
//! [`super::layer`] builds these from `tracing` spans and [`super::otlp`]
//! encodes them. This module is the data and the rules that bound it: ids,
//! kinds, status, and the limits that keep one span, or one badly behaved
//! caller, from growing without bound.

use std::{
    fmt,
    time::{SystemTime, UNIX_EPOCH},
};

/// 16 bytes naming a whole trace. Never all zero.
pub(crate) type TraceId = [u8; 16];
/// 8 bytes naming one span. Never all zero.
pub(crate) type SpanId = [u8; 8];

/// Attributes kept per span; the OpenTelemetry default limit.
pub(crate) const MAX_ATTRIBUTES: usize = 128;
/// Events kept per span; the OpenTelemetry default limit.
pub(crate) const MAX_EVENTS: usize = 128;
/// Longest string kept, in bytes. A longer one is cut at a character boundary.
pub(crate) const MAX_STRING_BYTES: usize = 4096;

/// The value of a span attribute.
///
/// OpenTelemetry attribute values are scalars; anything richer should be
/// rendered to a string by the caller.
#[derive(Debug, Clone, PartialEq)]
pub enum AttributeValue {
    /// Text.
    String(String),
    /// A flag.
    Bool(bool),
    /// A signed integer.
    Int(i64),
    /// A floating-point number. A non-finite one is exported as text, since
    /// JSON has no spelling for it.
    Float(f64),
}

impl From<&str> for AttributeValue {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
}

impl From<String> for AttributeValue {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<bool> for AttributeValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<i64> for AttributeValue {
    fn from(value: i64) -> Self {
        Self::Int(value)
    }
}

impl From<f64> for AttributeValue {
    fn from(value: f64) -> Self {
        Self::Float(value)
    }
}

/// An attribute with its key.
pub(crate) type Attribute = (String, AttributeValue);

/// How a span relates to the work around it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Kind {
    #[default]
    Internal,
    Server,
    Client,
    Producer,
    Consumer,
}

impl Kind {
    /// Read the value of an `otel.kind` field, case-insensitively.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        Some(match text.to_ascii_lowercase().as_str() {
            "internal" => Self::Internal,
            "server" => Self::Server,
            "client" => Self::Client,
            "producer" => Self::Producer,
            "consumer" => Self::Consumer,
            _ => return None,
        })
    }

    /// The OTLP enum value.
    pub(crate) fn code(self) -> u32 {
        match self {
            Self::Internal => 1,
            Self::Server => 2,
            Self::Client => 3,
            Self::Producer => 4,
            Self::Consumer => 5,
        }
    }
}

/// The outcome of a span.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum Status {
    #[default]
    Unset,
    Ok,
    Error,
}

impl Status {
    /// Read the value of an `otel.status_code` field, case-insensitively.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        Some(match text.to_ascii_lowercase().as_str() {
            "unset" => Self::Unset,
            "ok" => Self::Ok,
            "error" => Self::Error,
            _ => return None,
        })
    }

    /// The OTLP enum value.
    pub(crate) fn code(&self) -> u32 {
        match self {
            Self::Unset => 0,
            Self::Ok => 1,
            Self::Error => 2,
        }
    }
}

/// Something that happened during a span.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Event {
    pub(crate) time_nanos: u64,
    pub(crate) name: String,
    pub(crate) attributes: Vec<Attribute>,
}

/// A span that has ended and is ready to export.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FinishedSpan {
    pub(crate) trace_id: TraceId,
    pub(crate) span_id: SpanId,
    /// `None` for the root of a trace.
    pub(crate) parent_span_id: Option<SpanId>,
    /// The W3C `tracestate` of the remote parent, passed on unchanged.
    pub(crate) trace_state: Option<String>,
    pub(crate) name: String,
    pub(crate) kind: Kind,
    pub(crate) start_nanos: u64,
    pub(crate) end_nanos: u64,
    pub(crate) attributes: Vec<Attribute>,
    pub(crate) events: Vec<Event>,
    pub(crate) status: Status,
    pub(crate) status_message: Option<String>,
}

/// Nanoseconds since the Unix epoch.
pub(crate) fn now_nanos() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX)
        })
}

/// Cut `text` to at most [`MAX_STRING_BYTES`], at a character boundary.
pub(crate) fn clip(mut text: String) -> String {
    if text.len() > MAX_STRING_BYTES {
        let mut end = MAX_STRING_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

/// Clip the strings in `value`.
pub(crate) fn clip_value(value: AttributeValue) -> AttributeValue {
    match value {
        AttributeValue::String(text) => AttributeValue::String(clip(text)),
        other => other,
    }
}

/// Add `(key, value)`, replacing an earlier value for `key`, unless the list is
/// full and the key is new.
pub(crate) fn put(attributes: &mut Vec<Attribute>, key: &str, value: AttributeValue) {
    let value = clip_value(value);
    if let Some(existing) = attributes.iter_mut().find(|(k, _)| k == key) {
        existing.1 = value;
    } else if attributes.len() < MAX_ATTRIBUTES {
        attributes.push((key.to_owned(), value));
    }
}

/// Lowercase hex of `bytes`.
pub(crate) fn hex(bytes: &[u8]) -> String {
    use fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

/// Parse `text` as exactly `N` bytes of hex, rejecting all zeros.
pub(crate) fn parse_id<const N: usize>(text: &str) -> Option<[u8; N]> {
    if text.len() != N * 2 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut id = [0u8; N];
    for (byte, pair) in id.iter_mut().zip(text.as_bytes().chunks(2)) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    (id != [0u8; N]).then_some(id)
}

/// Whether a root span on `trace_id` is recorded, at `ratio` of traces.
///
/// The trace id decides, as in the OpenTelemetry `TraceIdRatioBased` sampler,
/// so every service that sees the same trace id and ratio agrees.
pub(crate) fn sampled_at(ratio: f64, trace_id: &TraceId) -> bool {
    if ratio >= 1.0 {
        return true;
    }
    if ratio <= 0.0 || ratio.is_nan() {
        return false;
    }
    let mut low = [0u8; 8];
    low.copy_from_slice(&trace_id[8..]);
    let draw = u64::from_be_bytes(low) >> 1;
    // 2^63 as the scale: `draw` has 63 significant bits.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let threshold = (ratio * (1u64 << 63) as f64) as u64;
    draw < threshold
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_through_hex() {
        let id: TraceId = [0xab; 16];
        assert_eq!(hex(&id), "ab".repeat(16));
        assert_eq!(parse_id::<16>(&hex(&id)), Some(id));
    }

    #[test]
    fn parse_id_rejects_wrong_length_non_hex_and_all_zero() {
        assert_eq!(parse_id::<8>("0000000000000000"), None);
        assert_eq!(parse_id::<8>("00f067aa0ba902"), None);
        assert_eq!(parse_id::<8>("00f067aa0ba902b7ff"), None);
        assert_eq!(parse_id::<8>("00f067aa0ba902bz"), None);
        assert_eq!(
            parse_id::<8>("00f067aa0ba902b7").map(|id| id[7]),
            Some(0xb7)
        );
        // A sign or space is not a hex digit, though `from_str_radix` allows `+`.
        assert_eq!(parse_id::<2>("+1+1"), None);
    }

    #[test]
    fn a_long_string_is_cut_on_a_character_boundary() {
        let text = "é".repeat(MAX_STRING_BYTES); // two bytes each
        let clipped = clip(text);
        assert!(clipped.len() <= MAX_STRING_BYTES);
        assert!(clipped.chars().all(|c| c == 'é'));
        assert_eq!(clip("short".to_owned()), "short");
    }

    #[test]
    fn attributes_replace_by_key_and_stop_at_the_limit() {
        let mut attributes = Vec::new();
        put(&mut attributes, "a", 1.into());
        put(&mut attributes, "a", 2.into());
        assert_eq!(attributes, [("a".to_owned(), AttributeValue::Int(2))]);

        for n in 0..(MAX_ATTRIBUTES * 2) {
            put(&mut attributes, &format!("k{n}"), true.into());
        }
        assert_eq!(attributes.len(), MAX_ATTRIBUTES);
        // An existing key can still be updated when the list is full.
        put(&mut attributes, "a", 3.into());
        assert_eq!(attributes[0].1, AttributeValue::Int(3));
    }

    #[test]
    fn kinds_and_statuses_parse_case_insensitively() {
        assert_eq!(Kind::parse("SERVER"), Some(Kind::Server));
        assert_eq!(Kind::parse("nonsense"), None);
        assert_eq!(Kind::Server.code(), 2);
        assert_eq!(Status::parse("Error"), Some(Status::Error));
        assert_eq!(Status::parse("fine"), None);
        assert_eq!(Status::Ok.code(), 1);
    }

    #[test]
    fn the_ratio_sampler_is_decided_by_the_trace_id() {
        let low: TraceId = [0; 16];
        let mut high: TraceId = [0; 16];
        high[8..].copy_from_slice(&u64::MAX.to_be_bytes());

        assert!(sampled_at(1.0, &high));
        assert!(!sampled_at(0.0, &low));
        assert!(!sampled_at(f64::NAN, &low));
        assert!(
            sampled_at(0.5, &low),
            "a low draw is under any positive ratio"
        );
        assert!(
            !sampled_at(0.5, &high),
            "the top draw is over a ratio below 1"
        );
    }

    #[test]
    fn the_ratio_is_roughly_the_fraction_sampled() {
        let mut kept = 0;
        for n in 0u64..10_000 {
            let mut id: TraceId = [0; 16];
            // Spread the draws evenly over the 64-bit range.
            id[8..].copy_from_slice(&n.wrapping_mul(1_844_674_407_370_955).to_be_bytes());
            kept += usize::from(sampled_at(0.25, &id));
        }
        assert!(
            (2_300..2_700).contains(&kept),
            "kept {kept} of 10000 at 0.25"
        );
    }
}
