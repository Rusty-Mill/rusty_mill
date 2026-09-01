//! Readiness domain events for the meshed manpower bounded context --
//! the Rust port of `meshed.domains.events.readiness` (DOM-010). Same
//! bitemporal `effective_date`/`transaction_date` semantics as
//! `crate::events::personnel` -- see that module's doc.

use rusty_json::json;
use rusty_meshed_core::avro::{decode_double, decode_string, encode_double, encode_string};
use rusty_meshed_core::{AvroDecodeError, BaseEvent, DomainEvent};

/// Event emitted when a unit's readiness is formally assessed
/// (DOM-010). This is a measurement event -- it records a point-in-time
/// readiness percentage for a unit. `assessed_at` is the wall-clock
/// time the assessment was conducted (ISO-8601 string);
/// `effective_date` is the operational date the readiness figure is
/// attributed to; `transaction_date` is when it was entered into the
/// system of record.
///
/// `readiness_pct` is expressed as a value between `0.0` and `100.0`
/// (not enforced -- same as the source). Unlike every other domain
/// event in this crate (all `DELTA`), a producer classifies this
/// port's events `EventType::Measurement` -- that classification lives
/// on the output port declaration (`rusty-meshed-sdk::OutputPortSpec`),
/// not on the event struct itself, matching the source (nothing about
/// `UnitReadinessAssessed`'s own class declares its `EventType`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnitReadinessAssessed {
    pub base: BaseEvent,
    pub unit_uic: String,
    pub readiness_pct: f64,
    pub assessed_at: String,
    pub effective_date: String,
    pub transaction_date: String,
}

impl UnitReadinessAssessed {
    pub const NAMESPACE: &'static str = "meshed.domains.readiness";

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        correlation_id: impl Into<String>,
        unit_uic: impl Into<String>,
        readiness_pct: f64,
        assessed_at: impl Into<String>,
        effective_date: impl Into<String>,
        transaction_date: impl Into<String>,
    ) -> Self {
        UnitReadinessAssessed {
            base: BaseEvent::new(correlation_id),
            unit_uic: unit_uic.into(),
            readiness_pct,
            assessed_at: assessed_at.into(),
            effective_date: effective_date.into(),
            transaction_date: transaction_date.into(),
        }
    }

    pub fn avro_schema() -> String {
        BaseEvent::avro_record_schema(
            "UnitReadinessAssessed",
            Self::NAMESPACE,
            json!([
                {"name": "unit_uic", "type": "string"},
                {"name": "readiness_pct", "type": "double"},
                {"name": "assessed_at", "type": "string"},
                {"name": "effective_date", "type": "string"},
                {"name": "transaction_date", "type": "string"}
            ]),
        )
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.base.encode_into(&mut out);
        encode_string(&self.unit_uic, &mut out);
        encode_double(self.readiness_pct, &mut out);
        encode_string(&self.assessed_at, &mut out);
        encode_string(&self.effective_date, &mut out);
        encode_string(&self.transaction_date, &mut out);
        out
    }

    pub fn deserialize(bytes: &[u8]) -> Result<Self, AvroDecodeError> {
        let mut pos = 0;
        let base = BaseEvent::decode_from(bytes, &mut pos)?;
        let unit_uic = decode_string(bytes, &mut pos)?;
        let readiness_pct = decode_double(bytes, &mut pos)?;
        let assessed_at = decode_string(bytes, &mut pos)?;
        let effective_date = decode_string(bytes, &mut pos)?;
        let transaction_date = decode_string(bytes, &mut pos)?;
        Ok(UnitReadinessAssessed {
            base,
            unit_uic,
            readiness_pct,
            assessed_at,
            effective_date,
            transaction_date,
        })
    }
}

impl DomainEvent for UnitReadinessAssessed {
    const EVENT_NAME: &'static str = "UnitReadinessAssessed";

    fn base(&self) -> &BaseEvent {
        &self.base
    }

    fn avro_schema() -> String {
        Self::avro_schema()
    }

    fn serialize(&self) -> Vec<u8> {
        self.serialize()
    }

    fn deserialize(bytes: &[u8]) -> Result<Self, AvroDecodeError> {
        Self::deserialize(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_then_deserialize_round_trips_including_the_float_field() {
        let event = UnitReadinessAssessed::new(
            "req-1",
            "UIC-1",
            75.5,
            "2026-01-01T00:00:00Z",
            "2026-01-01",
            "2026-01-02",
        );
        let bytes = event.serialize();
        assert_eq!(UnitReadinessAssessed::deserialize(&bytes).unwrap(), event);
    }

    #[test]
    fn avro_schema_types_readiness_pct_as_double() {
        let schema = UnitReadinessAssessed::avro_schema();
        let parsed: rusty_json::Value = rusty_json::from_str(&schema).unwrap();
        assert_eq!(
            parsed.get("namespace").unwrap().as_str(),
            Some("meshed.domains.readiness")
        );
        let fields = parsed.get("fields").unwrap().as_array().unwrap();
        let readiness_field = fields
            .iter()
            .find(|f| f.get("name").unwrap().as_str() == Some("readiness_pct"))
            .unwrap();
        assert_eq!(
            readiness_field.get("type").unwrap().as_str(),
            Some("double")
        );
    }
}
