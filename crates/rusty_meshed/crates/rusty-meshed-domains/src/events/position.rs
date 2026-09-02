//! Position domain events for the meshed manpower bounded context --
//! the Rust port of `meshed.domains.events.position` (DOM-006..009).
//! Same bitemporal `effective_date`/`transaction_date` semantics as
//! `crate::events::personnel` -- see that module's doc.

use rusty_json::json;
use rusty_meshed_core::avro::{decode_string, encode_string};
use rusty_meshed_core::{AvroDecodeError, BaseEvent, DomainEvent};

/// Event emitted when a position's authorization status is modified
/// (DOM-006). `authorization_status` carries values such as
/// `"AUTHORIZED"` or `"DEAUTHORIZED"`. Changes to grade or duty title
/// while authorization remains the same should use
/// [`PositionModified`] instead.
#[derive(Debug, Clone, PartialEq)]
pub struct PositionAuthorizationChanged {
    pub base: BaseEvent,
    pub position_id: String,
    pub unit_uic: String,
    pub authorized_grade: String,
    pub duty_title: String,
    pub authorization_status: String,
    pub effective_date: String,
    pub transaction_date: String,
}

impl PositionAuthorizationChanged {
    pub const NAMESPACE: &'static str = "meshed.domains.position";

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        correlation_id: impl Into<String>,
        position_id: impl Into<String>,
        unit_uic: impl Into<String>,
        authorized_grade: impl Into<String>,
        duty_title: impl Into<String>,
        authorization_status: impl Into<String>,
        effective_date: impl Into<String>,
        transaction_date: impl Into<String>,
    ) -> Self {
        PositionAuthorizationChanged {
            base: BaseEvent::new(correlation_id),
            position_id: position_id.into(),
            unit_uic: unit_uic.into(),
            authorized_grade: authorized_grade.into(),
            duty_title: duty_title.into(),
            authorization_status: authorization_status.into(),
            effective_date: effective_date.into(),
            transaction_date: transaction_date.into(),
        }
    }

    pub fn avro_schema() -> String {
        BaseEvent::avro_record_schema(
            "PositionAuthorizationChanged",
            Self::NAMESPACE,
            json!([
                {"name": "position_id", "type": "string"},
                {"name": "unit_uic", "type": "string"},
                {"name": "authorized_grade", "type": "string"},
                {"name": "duty_title", "type": "string"},
                {"name": "authorization_status", "type": "string"},
                {"name": "effective_date", "type": "string"},
                {"name": "transaction_date", "type": "string"}
            ]),
        )
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.base.encode_into(&mut out);
        encode_string(&self.position_id, &mut out);
        encode_string(&self.unit_uic, &mut out);
        encode_string(&self.authorized_grade, &mut out);
        encode_string(&self.duty_title, &mut out);
        encode_string(&self.authorization_status, &mut out);
        encode_string(&self.effective_date, &mut out);
        encode_string(&self.transaction_date, &mut out);
        out
    }

    pub fn deserialize(bytes: &[u8]) -> Result<Self, AvroDecodeError> {
        let mut pos = 0;
        let base = BaseEvent::decode_from(bytes, &mut pos)?;
        let position_id = decode_string(bytes, &mut pos)?;
        let unit_uic = decode_string(bytes, &mut pos)?;
        let authorized_grade = decode_string(bytes, &mut pos)?;
        let duty_title = decode_string(bytes, &mut pos)?;
        let authorization_status = decode_string(bytes, &mut pos)?;
        let effective_date = decode_string(bytes, &mut pos)?;
        let transaction_date = decode_string(bytes, &mut pos)?;
        Ok(PositionAuthorizationChanged {
            base,
            position_id,
            unit_uic,
            authorized_grade,
            duty_title,
            authorization_status,
            effective_date,
            transaction_date,
        })
    }
}

/// Event emitted when a person fills a previously vacant position
/// (DOM-007). Complements [`PositionVacated`] -- together they form
/// the complete fill/vacate lifecycle for a position.
#[derive(Debug, Clone, PartialEq)]
pub struct PositionFilled {
    pub base: BaseEvent,
    pub position_id: String,
    pub person_id: String,
    pub unit_uic: String,
    pub effective_date: String,
    pub transaction_date: String,
}

impl PositionFilled {
    pub const NAMESPACE: &'static str = "meshed.domains.position";

    pub fn new(
        correlation_id: impl Into<String>,
        position_id: impl Into<String>,
        person_id: impl Into<String>,
        unit_uic: impl Into<String>,
        effective_date: impl Into<String>,
        transaction_date: impl Into<String>,
    ) -> Self {
        PositionFilled {
            base: BaseEvent::new(correlation_id),
            position_id: position_id.into(),
            person_id: person_id.into(),
            unit_uic: unit_uic.into(),
            effective_date: effective_date.into(),
            transaction_date: transaction_date.into(),
        }
    }

    pub fn avro_schema() -> String {
        BaseEvent::avro_record_schema(
            "PositionFilled",
            Self::NAMESPACE,
            json!([
                {"name": "position_id", "type": "string"},
                {"name": "person_id", "type": "string"},
                {"name": "unit_uic", "type": "string"},
                {"name": "effective_date", "type": "string"},
                {"name": "transaction_date", "type": "string"}
            ]),
        )
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.base.encode_into(&mut out);
        encode_string(&self.position_id, &mut out);
        encode_string(&self.person_id, &mut out);
        encode_string(&self.unit_uic, &mut out);
        encode_string(&self.effective_date, &mut out);
        encode_string(&self.transaction_date, &mut out);
        out
    }

    pub fn deserialize(bytes: &[u8]) -> Result<Self, AvroDecodeError> {
        let mut pos = 0;
        let base = BaseEvent::decode_from(bytes, &mut pos)?;
        let position_id = decode_string(bytes, &mut pos)?;
        let person_id = decode_string(bytes, &mut pos)?;
        let unit_uic = decode_string(bytes, &mut pos)?;
        let effective_date = decode_string(bytes, &mut pos)?;
        let transaction_date = decode_string(bytes, &mut pos)?;
        Ok(PositionFilled {
            base,
            position_id,
            person_id,
            unit_uic,
            effective_date,
            transaction_date,
        })
    }
}

/// Event emitted when a person vacates a position (DOM-008).
/// `vacancy_reason` captures why the position became vacant (e.g.
/// `"REASSIGNMENT"`, `"SEPARATION"`, `"TDY_DEPARTURE"`).
#[derive(Debug, Clone, PartialEq)]
pub struct PositionVacated {
    pub base: BaseEvent,
    pub position_id: String,
    pub person_id: String,
    pub unit_uic: String,
    pub vacancy_reason: String,
    pub effective_date: String,
    pub transaction_date: String,
}

impl PositionVacated {
    pub const NAMESPACE: &'static str = "meshed.domains.position";

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        correlation_id: impl Into<String>,
        position_id: impl Into<String>,
        person_id: impl Into<String>,
        unit_uic: impl Into<String>,
        vacancy_reason: impl Into<String>,
        effective_date: impl Into<String>,
        transaction_date: impl Into<String>,
    ) -> Self {
        PositionVacated {
            base: BaseEvent::new(correlation_id),
            position_id: position_id.into(),
            person_id: person_id.into(),
            unit_uic: unit_uic.into(),
            vacancy_reason: vacancy_reason.into(),
            effective_date: effective_date.into(),
            transaction_date: transaction_date.into(),
        }
    }

    pub fn avro_schema() -> String {
        BaseEvent::avro_record_schema(
            "PositionVacated",
            Self::NAMESPACE,
            json!([
                {"name": "position_id", "type": "string"},
                {"name": "person_id", "type": "string"},
                {"name": "unit_uic", "type": "string"},
                {"name": "vacancy_reason", "type": "string"},
                {"name": "effective_date", "type": "string"},
                {"name": "transaction_date", "type": "string"}
            ]),
        )
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.base.encode_into(&mut out);
        encode_string(&self.position_id, &mut out);
        encode_string(&self.person_id, &mut out);
        encode_string(&self.unit_uic, &mut out);
        encode_string(&self.vacancy_reason, &mut out);
        encode_string(&self.effective_date, &mut out);
        encode_string(&self.transaction_date, &mut out);
        out
    }

    pub fn deserialize(bytes: &[u8]) -> Result<Self, AvroDecodeError> {
        let mut pos = 0;
        let base = BaseEvent::decode_from(bytes, &mut pos)?;
        let position_id = decode_string(bytes, &mut pos)?;
        let person_id = decode_string(bytes, &mut pos)?;
        let unit_uic = decode_string(bytes, &mut pos)?;
        let vacancy_reason = decode_string(bytes, &mut pos)?;
        let effective_date = decode_string(bytes, &mut pos)?;
        let transaction_date = decode_string(bytes, &mut pos)?;
        Ok(PositionVacated {
            base,
            position_id,
            person_id,
            unit_uic,
            vacancy_reason,
            effective_date,
            transaction_date,
        })
    }
}

/// Event emitted when a specific field on a position record changes
/// (DOM-009). Carries a generic field-level diff (`field_changed`,
/// `old_value`, `new_value`) to avoid schema churn every time the
/// modifiable field set grows. Consumers interested in a specific
/// field should filter on `field_changed`.
#[derive(Debug, Clone, PartialEq)]
pub struct PositionModified {
    pub base: BaseEvent,
    pub position_id: String,
    pub unit_uic: String,
    pub field_changed: String,
    pub old_value: String,
    pub new_value: String,
    pub effective_date: String,
    pub transaction_date: String,
}

impl PositionModified {
    pub const NAMESPACE: &'static str = "meshed.domains.position";

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        correlation_id: impl Into<String>,
        position_id: impl Into<String>,
        unit_uic: impl Into<String>,
        field_changed: impl Into<String>,
        old_value: impl Into<String>,
        new_value: impl Into<String>,
        effective_date: impl Into<String>,
        transaction_date: impl Into<String>,
    ) -> Self {
        PositionModified {
            base: BaseEvent::new(correlation_id),
            position_id: position_id.into(),
            unit_uic: unit_uic.into(),
            field_changed: field_changed.into(),
            old_value: old_value.into(),
            new_value: new_value.into(),
            effective_date: effective_date.into(),
            transaction_date: transaction_date.into(),
        }
    }

    pub fn avro_schema() -> String {
        BaseEvent::avro_record_schema(
            "PositionModified",
            Self::NAMESPACE,
            json!([
                {"name": "position_id", "type": "string"},
                {"name": "unit_uic", "type": "string"},
                {"name": "field_changed", "type": "string"},
                {"name": "old_value", "type": "string"},
                {"name": "new_value", "type": "string"},
                {"name": "effective_date", "type": "string"},
                {"name": "transaction_date", "type": "string"}
            ]),
        )
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.base.encode_into(&mut out);
        encode_string(&self.position_id, &mut out);
        encode_string(&self.unit_uic, &mut out);
        encode_string(&self.field_changed, &mut out);
        encode_string(&self.old_value, &mut out);
        encode_string(&self.new_value, &mut out);
        encode_string(&self.effective_date, &mut out);
        encode_string(&self.transaction_date, &mut out);
        out
    }

    pub fn deserialize(bytes: &[u8]) -> Result<Self, AvroDecodeError> {
        let mut pos = 0;
        let base = BaseEvent::decode_from(bytes, &mut pos)?;
        let position_id = decode_string(bytes, &mut pos)?;
        let unit_uic = decode_string(bytes, &mut pos)?;
        let field_changed = decode_string(bytes, &mut pos)?;
        let old_value = decode_string(bytes, &mut pos)?;
        let new_value = decode_string(bytes, &mut pos)?;
        let effective_date = decode_string(bytes, &mut pos)?;
        let transaction_date = decode_string(bytes, &mut pos)?;
        Ok(PositionModified {
            base,
            position_id,
            unit_uic,
            field_changed,
            old_value,
            new_value,
            effective_date,
            transaction_date,
        })
    }
}

impl DomainEvent for PositionAuthorizationChanged {
    const EVENT_NAME: &'static str = "PositionAuthorizationChanged";

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

    fn to_json(&self) -> rusty_json::Value {
        let mut fields = self.base.to_json_fields();
        fields.insert(
            "position_id".to_string(),
            rusty_json::Value::from(self.position_id.as_str()),
        );
        fields.insert(
            "unit_uic".to_string(),
            rusty_json::Value::from(self.unit_uic.as_str()),
        );
        fields.insert(
            "authorized_grade".to_string(),
            rusty_json::Value::from(self.authorized_grade.as_str()),
        );
        fields.insert(
            "duty_title".to_string(),
            rusty_json::Value::from(self.duty_title.as_str()),
        );
        fields.insert(
            "authorization_status".to_string(),
            rusty_json::Value::from(self.authorization_status.as_str()),
        );
        fields.insert(
            "effective_date".to_string(),
            rusty_json::Value::from(self.effective_date.as_str()),
        );
        fields.insert(
            "transaction_date".to_string(),
            rusty_json::Value::from(self.transaction_date.as_str()),
        );
        rusty_json::Value::Object(fields)
    }
}

impl DomainEvent for PositionFilled {
    const EVENT_NAME: &'static str = "PositionFilled";

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

    fn to_json(&self) -> rusty_json::Value {
        let mut fields = self.base.to_json_fields();
        fields.insert(
            "position_id".to_string(),
            rusty_json::Value::from(self.position_id.as_str()),
        );
        fields.insert(
            "person_id".to_string(),
            rusty_json::Value::from(self.person_id.as_str()),
        );
        fields.insert(
            "unit_uic".to_string(),
            rusty_json::Value::from(self.unit_uic.as_str()),
        );
        fields.insert(
            "effective_date".to_string(),
            rusty_json::Value::from(self.effective_date.as_str()),
        );
        fields.insert(
            "transaction_date".to_string(),
            rusty_json::Value::from(self.transaction_date.as_str()),
        );
        rusty_json::Value::Object(fields)
    }
}

impl DomainEvent for PositionVacated {
    const EVENT_NAME: &'static str = "PositionVacated";

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

    fn to_json(&self) -> rusty_json::Value {
        let mut fields = self.base.to_json_fields();
        fields.insert(
            "position_id".to_string(),
            rusty_json::Value::from(self.position_id.as_str()),
        );
        fields.insert(
            "person_id".to_string(),
            rusty_json::Value::from(self.person_id.as_str()),
        );
        fields.insert(
            "unit_uic".to_string(),
            rusty_json::Value::from(self.unit_uic.as_str()),
        );
        fields.insert(
            "vacancy_reason".to_string(),
            rusty_json::Value::from(self.vacancy_reason.as_str()),
        );
        fields.insert(
            "effective_date".to_string(),
            rusty_json::Value::from(self.effective_date.as_str()),
        );
        fields.insert(
            "transaction_date".to_string(),
            rusty_json::Value::from(self.transaction_date.as_str()),
        );
        rusty_json::Value::Object(fields)
    }
}

impl DomainEvent for PositionModified {
    const EVENT_NAME: &'static str = "PositionModified";

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

    fn to_json(&self) -> rusty_json::Value {
        let mut fields = self.base.to_json_fields();
        fields.insert(
            "position_id".to_string(),
            rusty_json::Value::from(self.position_id.as_str()),
        );
        fields.insert(
            "unit_uic".to_string(),
            rusty_json::Value::from(self.unit_uic.as_str()),
        );
        fields.insert(
            "field_changed".to_string(),
            rusty_json::Value::from(self.field_changed.as_str()),
        );
        fields.insert(
            "old_value".to_string(),
            rusty_json::Value::from(self.old_value.as_str()),
        );
        fields.insert(
            "new_value".to_string(),
            rusty_json::Value::from(self.new_value.as_str()),
        );
        fields.insert(
            "effective_date".to_string(),
            rusty_json::Value::from(self.effective_date.as_str()),
        );
        fields.insert(
            "transaction_date".to_string(),
            rusty_json::Value::from(self.transaction_date.as_str()),
        );
        rusty_json::Value::Object(fields)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_authorization_changed_serialize_then_deserialize_round_trips() {
        let event = PositionAuthorizationChanged::new(
            "req-1",
            "pos-1",
            "UIC-1",
            "E5",
            "Squad Leader",
            "AUTHORIZED",
            "2026-01-01",
            "2026-01-02",
        );
        let bytes = event.serialize();
        assert_eq!(
            PositionAuthorizationChanged::deserialize(&bytes).unwrap(),
            event
        );
        assert_eq!(
            PositionAuthorizationChanged::NAMESPACE,
            "meshed.domains.position"
        );
    }

    #[test]
    fn position_filled_serialize_then_deserialize_round_trips() {
        let event =
            PositionFilled::new("req-1", "pos-1", "p-1", "UIC-1", "2026-01-01", "2026-01-02");
        let bytes = event.serialize();
        assert_eq!(PositionFilled::deserialize(&bytes).unwrap(), event);
    }

    #[test]
    fn position_vacated_serialize_then_deserialize_round_trips() {
        let event = PositionVacated::new(
            "req-1",
            "pos-1",
            "p-1",
            "UIC-1",
            "REASSIGNMENT",
            "2026-01-01",
            "2026-01-02",
        );
        let bytes = event.serialize();
        assert_eq!(PositionVacated::deserialize(&bytes).unwrap(), event);
    }

    #[test]
    fn position_modified_serialize_then_deserialize_round_trips() {
        let event = PositionModified::new(
            "req-1",
            "pos-1",
            "UIC-1",
            "duty_title",
            "Rifleman",
            "Squad Leader",
            "2026-01-01",
            "2026-01-02",
        );
        let bytes = event.serialize();
        assert_eq!(PositionModified::deserialize(&bytes).unwrap(), event);
    }

    #[test]
    fn position_filled_avro_schema_includes_lineage_and_own_fields() {
        let schema = PositionFilled::avro_schema();
        let parsed: rusty_json::Value = rusty_json::from_str(&schema).unwrap();
        assert_eq!(
            parsed.get("namespace").unwrap().as_str(),
            Some("meshed.domains.position")
        );
        let names: Vec<&str> = parsed
            .get("fields")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f.get("name").unwrap().as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            vec![
                "event_id",
                "correlation_id",
                "source_event_ids",
                "timestamp",
                "position_id",
                "person_id",
                "unit_uic",
                "effective_date",
                "transaction_date",
            ]
        );
    }
}
