//! [`BaseEvent`] -- the Rust port of `meshed.sdk.base_event.BaseEvent`
//! (SDK-001..008, DOM-001): the mandatory lineage contract every
//! meshed platform event carries.
//!
//! Lives in this shared crate, not `rusty-meshed-sdk`, for the same
//! reason [`crate::EventType`] does (see that module's doc): both
//! `rusty-meshed-sdk` (`OutputPortSpec<E>`'s `E`) and
//! `rusty-meshed-domains` (every domain event) need it, and they're
//! sibling crates with no dependency edge between them.
//!
//! # Scope of this pass
//!
//! The source's `BaseEvent` is a `pydantic`/`dataclasses_avroschema`
//! base class: subclassing it and adding fields is enough for
//! `avro_schema()`/`serialize()`/`deserialize()` to automatically
//! reflect over *all* fields, base and subclass alike, at runtime.
//! Rust has no runtime reflection, and this workspace has no
//! procedural-macro crate to generate the equivalent from a
//! `#[derive(...)]`, so this pass ports `BaseEvent` itself as a
//! complete, standalone unit -- its own four fields' schema and binary
//! codec, fully round-trip-tested -- without deciding how a concrete
//! domain event (`rusty-meshed-domains`, DOM-002 onward) extends it
//! with its own fields. That's a real design question (a hand-written
//! trait per event? a future derive macro?) worth deciding when a
//! domain event actually needs to answer it, not speculatively here.
//!
//! # Avro encoding
//!
//! No Avro crate exists anywhere in this workspace, so the binary
//! codec below is a minimal, from-scratch implementation of exactly
//! the two Avro primitives `BaseEvent`'s fields need: `string`
//! (zigzag-varint byte-length prefix + UTF-8 bytes) and `array<string>`
//! (Avro's block encoding: a positive item-count varint, that many
//! encoded items, terminated by a zero-count block -- this codec
//! always emits/expects exactly one block, which is valid per the
//! Avro spec, not a simplification of it; decoding also accepts the
//! spec's negative-count-with-byte-size block variant for robustness,
//! even though this codec never emits one itself).

use rusty_err::Error;
use rusty_json::json;

/// The lineage contract every meshed platform event carries (SDK-001,
/// DOM-001).
#[derive(Debug, Clone, PartialEq)]
pub struct BaseEvent {
    /// Globally unique identifier for this event instance -- a fresh
    /// UUID v4 per instance (SDK-002).
    pub event_id: String,
    /// Caller-supplied ID linking causally related events. Required --
    /// unlike the other three fields, [`BaseEvent::new`] takes it as a
    /// parameter rather than defaulting it (SDK-003); there is
    /// deliberately no `Default` impl for `BaseEvent`, for the same
    /// reason.
    pub correlation_id: String,
    /// Upstream `event_id` values that caused this event; empty for a
    /// root event. Each instance gets its own independent `Vec`
    /// (SDK-004).
    pub source_event_ids: Vec<String>,
    /// UTC ISO-8601 timestamp set at construction time (SDK-005).
    pub timestamp: String,
}

impl BaseEvent {
    /// The Avro namespace every `BaseEvent`-derived schema shares
    /// unless a subclass's own `Meta` overrides it (SDK-006).
    pub const NAMESPACE: &'static str = "meshed.base";

    /// Builds a new event with `correlation_id` supplied by the
    /// caller and the other three lineage fields auto-populated:
    /// `event_id` a fresh UUID v4, `source_event_ids` empty,
    /// `timestamp` the current UTC instant.
    pub fn new(correlation_id: impl Into<String>) -> Self {
        BaseEvent {
            event_id: rusty_uuid::Uuid::new_v4().to_string(),
            correlation_id: correlation_id.into(),
            source_event_ids: Vec::new(),
            timestamp: now_iso(),
        }
    }

    /// The Avro record schema for `BaseEvent`'s own four fields, as a
    /// JSON-parseable string (SDK-007).
    pub fn avro_schema() -> String {
        let schema = json!({
            "type": "record",
            "name": "BaseEvent",
            "namespace": Self::NAMESPACE,
            "fields": [
                {"name": "event_id", "type": "string"},
                {"name": "correlation_id", "type": "string"},
                {"name": "source_event_ids", "type": {"type": "array", "items": "string"}},
                {"name": "timestamp", "type": "string"}
            ]
        });
        rusty_json::to_string(&schema)
            .expect("a schema built from string literals always serializes")
    }

    /// Encodes this event as Avro binary, field order matching
    /// [`avro_schema`](Self::avro_schema) (SDK-008).
    pub fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::new();
        encode_string(&self.event_id, &mut out);
        encode_string(&self.correlation_id, &mut out);
        encode_string_array(&self.source_event_ids, &mut out);
        encode_string(&self.timestamp, &mut out);
        out
    }

    /// Decodes Avro binary produced by [`serialize`](Self::serialize),
    /// preserving all fields (SDK-008).
    pub fn deserialize(bytes: &[u8]) -> Result<Self, AvroDecodeError> {
        let mut pos = 0;
        let event_id = decode_string(bytes, &mut pos)?;
        let correlation_id = decode_string(bytes, &mut pos)?;
        let source_event_ids = decode_string_array(bytes, &mut pos)?;
        let timestamp = decode_string(bytes, &mut pos)?;
        Ok(BaseEvent {
            event_id,
            correlation_id,
            source_event_ids,
            timestamp,
        })
    }
}

/// Errors from [`BaseEvent::deserialize`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AvroDecodeError {
    #[error("unexpected end of input decoding an Avro long")]
    UnexpectedEofInLong,
    #[error("unexpected end of input decoding an Avro string/bytes payload")]
    UnexpectedEofInPayload,
    #[error("Avro string/bytes length must be non-negative, got {0}")]
    NegativeLength(i64),
    #[error("invalid UTF-8 in an Avro string field")]
    InvalidUtf8,
}

// ---------------------------------------------------------------------
// Avro binary primitives (`long` zigzag-varint, `string`, `array<string>`)
// ---------------------------------------------------------------------

fn encode_long(value: i64, out: &mut Vec<u8>) {
    let mut n = ((value << 1) ^ (value >> 63)) as u64;
    loop {
        let byte = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            out.push(byte);
            break;
        }
        out.push(byte | 0x80);
    }
}

fn decode_long(bytes: &[u8], pos: &mut usize) -> Result<i64, AvroDecodeError> {
    let mut result: u64 = 0;
    let mut shift = 0u32;
    loop {
        let byte = *bytes
            .get(*pos)
            .ok_or(AvroDecodeError::UnexpectedEofInLong)?;
        *pos += 1;
        result |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    Ok(((result >> 1) as i64) ^ -((result & 1) as i64))
}

fn encode_string(s: &str, out: &mut Vec<u8>) {
    encode_long(s.len() as i64, out);
    out.extend_from_slice(s.as_bytes());
}

fn decode_string(bytes: &[u8], pos: &mut usize) -> Result<String, AvroDecodeError> {
    let len = decode_long(bytes, pos)?;
    if len < 0 {
        return Err(AvroDecodeError::NegativeLength(len));
    }
    let end = *pos + len as usize;
    let slice = bytes
        .get(*pos..end)
        .ok_or(AvroDecodeError::UnexpectedEofInPayload)?;
    let s = std::str::from_utf8(slice)
        .map_err(|_| AvroDecodeError::InvalidUtf8)?
        .to_string();
    *pos = end;
    Ok(s)
}

fn encode_string_array(items: &[String], out: &mut Vec<u8>) {
    if !items.is_empty() {
        encode_long(items.len() as i64, out);
        for item in items {
            encode_string(item, out);
        }
    }
    encode_long(0, out);
}

fn decode_string_array(bytes: &[u8], pos: &mut usize) -> Result<Vec<String>, AvroDecodeError> {
    let mut result = Vec::new();
    loop {
        let count = decode_long(bytes, pos)?;
        if count == 0 {
            break;
        }
        let item_count = if count < 0 {
            // Negative block count: followed by the block's total
            // byte size (which we don't need to interpret item-by-item
            // decoding, so it's just consumed and discarded).
            decode_long(bytes, pos)?;
            -count
        } else {
            count
        };
        for _ in 0..item_count {
            result.push(decode_string(bytes, pos)?);
        }
    }
    Ok(result)
}

/// A minimal RFC 3339 UTC "now" formatter -- same hand-rolled
/// civil-from-days algorithm duplicated elsewhere in this crate family
/// (see `rusty-meshed-observability::metrics::now_iso`'s doc for why
/// there's no shared clock type to build on instead).
fn now_iso() -> String {
    let since_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let total_secs = since_epoch.as_secs();
    let mut days = (total_secs / 86_400) as i64;
    let secs_of_day = total_secs % 86_400;
    let (hour, minute, second) = (
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60,
    );

    days += 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = (days - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if month <= 2 { year + 1 } else { year };

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_requires_only_correlation_id_and_auto_populates_the_rest() {
        let event = BaseEvent::new("req-abc-123");
        assert_eq!(event.correlation_id, "req-abc-123");
        assert!(!event.event_id.is_empty());
        assert_eq!(event.event_id.len(), 36, "UUID v4 string form");
        assert!(event.source_event_ids.is_empty());
        assert!(!event.timestamp.is_empty());
    }

    #[test]
    fn each_instance_gets_an_independent_event_id_and_source_event_ids() {
        let a = BaseEvent::new("req-1");
        let mut b = BaseEvent::new("req-1");
        assert_ne!(a.event_id, b.event_id);

        b.source_event_ids.push("upstream-1".to_string());
        assert!(a.source_event_ids.is_empty(), "must not alias b's Vec");
    }

    #[test]
    fn avro_schema_is_json_parseable_with_the_four_lineage_fields() {
        let schema = BaseEvent::avro_schema();
        let parsed: rusty_json::Value = rusty_json::from_str(&schema).unwrap();
        assert_eq!(
            parsed.get("namespace").unwrap().as_str(),
            Some("meshed.base")
        );
        let fields = parsed.get("fields").unwrap().as_array().unwrap();
        let names: Vec<&str> = fields
            .iter()
            .map(|f| f.get("name").unwrap().as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            vec![
                "event_id",
                "correlation_id",
                "source_event_ids",
                "timestamp"
            ]
        );
    }

    #[test]
    fn serialize_then_deserialize_round_trips_all_fields() {
        let mut event = BaseEvent::new("req-abc-123");
        event.source_event_ids = vec!["e-1".to_string(), "e-2".to_string()];

        let bytes = event.serialize();
        let decoded = BaseEvent::deserialize(&bytes).unwrap();
        assert_eq!(decoded, event);
    }

    #[test]
    fn serialize_then_deserialize_round_trips_empty_source_event_ids() {
        let event = BaseEvent::new("req-abc-123");
        let bytes = event.serialize();
        let decoded = BaseEvent::deserialize(&bytes).unwrap();
        assert_eq!(decoded, event);
        assert!(decoded.source_event_ids.is_empty());
    }

    #[test]
    fn deserialize_rejects_truncated_input() {
        let event = BaseEvent::new("req-abc-123");
        let bytes = event.serialize();
        let truncated = &bytes[..bytes.len() - 1];
        assert!(BaseEvent::deserialize(truncated).is_err());
    }

    #[test]
    fn long_round_trips_negative_zero_and_positive_values() {
        for value in [i64::MIN, -1_000_000, -1, 0, 1, 1_000_000, i64::MAX] {
            let mut buf = Vec::new();
            encode_long(value, &mut buf);
            let mut pos = 0;
            assert_eq!(decode_long(&buf, &mut pos).unwrap(), value);
            assert_eq!(pos, buf.len());
        }
    }
}
