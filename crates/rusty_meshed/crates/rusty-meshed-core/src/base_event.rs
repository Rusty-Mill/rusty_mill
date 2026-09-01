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
//! # Extending `BaseEvent` with subclass fields
//!
//! The source's `BaseEvent` is a `pydantic`/`dataclasses_avroschema`
//! base class: subclassing it and adding fields is enough for
//! `avro_schema()`/`serialize()`/`deserialize()` to automatically
//! reflect over *all* fields, base and subclass alike, at runtime.
//! Rust has no runtime reflection, and this workspace has no
//! procedural-macro crate to generate the equivalent from a
//! `#[derive(...)]`.
//!
//! `rusty-meshed-domains`' nine domain events (DOM-002..010) answer
//! this by composition, not inheritance: each event struct embeds a
//! `base: BaseEvent` field plus its own typed fields, and hand-builds
//! its own `avro_schema()`/`serialize()`/`deserialize()` using
//! [`BaseEvent::encode_into`]/[`BaseEvent::decode_from`] (position-
//! flexible, unlike [`serialize`](BaseEvent::serialize)/
//! [`deserialize`](BaseEvent::deserialize), which are self-contained
//! for `BaseEvent` used standalone) to fold the four lineage fields in
//! at the front, followed by their own fields encoded via
//! [`crate::avro`]'s primitives directly. See
//! `rusty-meshed-domains::events`'s own module doc for why this crate
//! doesn't try to generalize that pattern into a shared trait: with
//! only nine concrete cases and no polymorphic dispatch over them yet,
//! a trait would be premature abstraction over data that doesn't need
//! one yet.

use crate::avro::{
    decode_string, decode_string_array, encode_string, encode_string_array, AvroDecodeError,
};
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

    /// The four lineage fields' Avro schema entries, in wire order --
    /// what a domain event's own `avro_schema()` prepends to its own
    /// fields' entries.
    pub fn avro_schema_fields() -> rusty_json::Value {
        json!([
            {"name": "event_id", "type": "string"},
            {"name": "correlation_id", "type": "string"},
            {"name": "source_event_ids", "type": {"type": "array", "items": "string"}},
            {"name": "timestamp", "type": "string"}
        ])
    }

    /// The Avro record schema for `BaseEvent`'s own four fields, as a
    /// JSON-parseable string (SDK-007).
    pub fn avro_schema() -> String {
        Self::avro_record_schema("BaseEvent", Self::NAMESPACE, json!([]))
    }

    /// Builds a full Avro record schema string for a domain event: the
    /// four lineage fields (in [`avro_schema_fields`](Self::avro_schema_fields)'s
    /// order) followed by `own_fields`, matching the source's own field
    /// ordering (`BaseEvent`'s fields first, then subclass fields in
    /// declaration order) -- what every `rusty-meshed-domains` event's
    /// `avro_schema()` is built from, so the four-lineage-field prefix
    /// only needs to be assembled correctly once.
    pub fn avro_record_schema(
        name: &str,
        namespace: &str,
        own_fields: rusty_json::Value,
    ) -> String {
        let mut fields = Self::avro_schema_fields();
        if let (Some(base_fields), Some(extra)) = (fields.as_array_mut(), own_fields.as_array()) {
            base_fields.extend(extra.iter().cloned());
        }
        let schema = json!({
            "type": "record",
            "name": name,
            "namespace": namespace,
            "fields": fields
        });
        rusty_json::to_string(&schema)
            .expect("a schema built from string literals always serializes")
    }

    /// Appends this event's four lineage fields to `out`, in the same
    /// order [`avro_schema_fields`](Self::avro_schema_fields) declares
    /// them -- the position-flexible primitive a domain event's own
    /// `serialize()` calls before encoding its own fields.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        encode_string(&self.event_id, out);
        encode_string(&self.correlation_id, out);
        encode_string_array(&self.source_event_ids, out);
        encode_string(&self.timestamp, out);
    }

    /// Decodes this event's four lineage fields starting at `*pos`,
    /// advancing it past the bytes consumed -- the position-flexible
    /// primitive a domain event's own `deserialize()` calls before
    /// decoding its own fields.
    pub fn decode_from(bytes: &[u8], pos: &mut usize) -> Result<Self, AvroDecodeError> {
        let event_id = decode_string(bytes, pos)?;
        let correlation_id = decode_string(bytes, pos)?;
        let source_event_ids = decode_string_array(bytes, pos)?;
        let timestamp = decode_string(bytes, pos)?;
        Ok(BaseEvent {
            event_id,
            correlation_id,
            source_event_ids,
            timestamp,
        })
    }

    /// Encodes this event as Avro binary (SDK-008). A thin wrapper
    /// over [`encode_into`](Self::encode_into) for `BaseEvent` used
    /// standalone, not embedded in a domain event.
    pub fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.encode_into(&mut out);
        out
    }

    /// Decodes Avro binary produced by [`serialize`](Self::serialize),
    /// preserving all fields (SDK-008). A thin wrapper over
    /// [`decode_from`](Self::decode_from) requiring the entire input
    /// be one `BaseEvent`.
    pub fn deserialize(bytes: &[u8]) -> Result<Self, AvroDecodeError> {
        let mut pos = 0;
        Self::decode_from(bytes, &mut pos)
    }
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
}
