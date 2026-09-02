//! Personnel domain events for the meshed manpower bounded context --
//! the Rust port of `meshed.domains.events.personnel` (DOM-002..005).
//!
//! All events use bitemporal semantics:
//!
//! - `effective_date` -- when the fact became true in the real world
//!   (ISO-8601 string).
//! - `transaction_date` -- when the system recorded the fact (ISO-8601
//!   string).
//!
//! A retroactive correction is identifiable when `effective_date <
//! transaction_date` without any additional flag -- the temporal gap
//! carries the semantic. Both fields are plain `String`, matching the
//! source's own `str` (not `datetime`) choice, made to avoid Avro
//! logical-type serialization issues.

use rusty_json::json;
use rusty_meshed_core::avro::{decode_string, encode_string};
use rusty_meshed_core::{AvroDecodeError, BaseEvent, DomainEvent};

/// Event emitted when a person is assigned to a position within a
/// unit (DOM-002). A delta event capturing the assignment action:
/// `effective_date` records when the assignment took effect
/// operationally, `transaction_date` when it was entered into the
/// system of record.
#[derive(Debug, Clone, PartialEq)]
pub struct PersonnelAssigned {
    pub base: BaseEvent,
    pub person_id: String,
    pub position_id: String,
    pub unit_uic: String,
    pub duty_title: String,
    pub grade: String,
    pub effective_date: String,
    pub transaction_date: String,
}

impl PersonnelAssigned {
    pub const NAMESPACE: &'static str = "meshed.domains.personnel";

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        correlation_id: impl Into<String>,
        person_id: impl Into<String>,
        position_id: impl Into<String>,
        unit_uic: impl Into<String>,
        duty_title: impl Into<String>,
        grade: impl Into<String>,
        effective_date: impl Into<String>,
        transaction_date: impl Into<String>,
    ) -> Self {
        PersonnelAssigned {
            base: BaseEvent::new(correlation_id),
            person_id: person_id.into(),
            position_id: position_id.into(),
            unit_uic: unit_uic.into(),
            duty_title: duty_title.into(),
            grade: grade.into(),
            effective_date: effective_date.into(),
            transaction_date: transaction_date.into(),
        }
    }

    pub fn avro_schema() -> String {
        BaseEvent::avro_record_schema(
            "PersonnelAssigned",
            Self::NAMESPACE,
            json!([
                {"name": "person_id", "type": "string"},
                {"name": "position_id", "type": "string"},
                {"name": "unit_uic", "type": "string"},
                {"name": "duty_title", "type": "string"},
                {"name": "grade", "type": "string"},
                {"name": "effective_date", "type": "string"},
                {"name": "transaction_date", "type": "string"}
            ]),
        )
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.base.encode_into(&mut out);
        encode_string(&self.person_id, &mut out);
        encode_string(&self.position_id, &mut out);
        encode_string(&self.unit_uic, &mut out);
        encode_string(&self.duty_title, &mut out);
        encode_string(&self.grade, &mut out);
        encode_string(&self.effective_date, &mut out);
        encode_string(&self.transaction_date, &mut out);
        out
    }

    pub fn deserialize(bytes: &[u8]) -> Result<Self, AvroDecodeError> {
        let mut pos = 0;
        let base = BaseEvent::decode_from(bytes, &mut pos)?;
        let person_id = decode_string(bytes, &mut pos)?;
        let position_id = decode_string(bytes, &mut pos)?;
        let unit_uic = decode_string(bytes, &mut pos)?;
        let duty_title = decode_string(bytes, &mut pos)?;
        let grade = decode_string(bytes, &mut pos)?;
        let effective_date = decode_string(bytes, &mut pos)?;
        let transaction_date = decode_string(bytes, &mut pos)?;
        Ok(PersonnelAssigned {
            base,
            person_id,
            position_id,
            unit_uic,
            duty_title,
            grade,
            effective_date,
            transaction_date,
        })
    }
}

/// Event emitted when a person is promoted to a new grade (DOM-003).
/// Captures the before/after grade pair so downstream consumers can
/// compute grade changes without a full personnel state read.
#[derive(Debug, Clone, PartialEq)]
pub struct PersonnelPromoted {
    pub base: BaseEvent,
    pub person_id: String,
    pub from_grade: String,
    pub to_grade: String,
    pub effective_date: String,
    pub transaction_date: String,
}

impl PersonnelPromoted {
    pub const NAMESPACE: &'static str = "meshed.domains.personnel";

    pub fn new(
        correlation_id: impl Into<String>,
        person_id: impl Into<String>,
        from_grade: impl Into<String>,
        to_grade: impl Into<String>,
        effective_date: impl Into<String>,
        transaction_date: impl Into<String>,
    ) -> Self {
        PersonnelPromoted {
            base: BaseEvent::new(correlation_id),
            person_id: person_id.into(),
            from_grade: from_grade.into(),
            to_grade: to_grade.into(),
            effective_date: effective_date.into(),
            transaction_date: transaction_date.into(),
        }
    }

    pub fn avro_schema() -> String {
        BaseEvent::avro_record_schema(
            "PersonnelPromoted",
            Self::NAMESPACE,
            json!([
                {"name": "person_id", "type": "string"},
                {"name": "from_grade", "type": "string"},
                {"name": "to_grade", "type": "string"},
                {"name": "effective_date", "type": "string"},
                {"name": "transaction_date", "type": "string"}
            ]),
        )
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.base.encode_into(&mut out);
        encode_string(&self.person_id, &mut out);
        encode_string(&self.from_grade, &mut out);
        encode_string(&self.to_grade, &mut out);
        encode_string(&self.effective_date, &mut out);
        encode_string(&self.transaction_date, &mut out);
        out
    }

    pub fn deserialize(bytes: &[u8]) -> Result<Self, AvroDecodeError> {
        let mut pos = 0;
        let base = BaseEvent::decode_from(bytes, &mut pos)?;
        let person_id = decode_string(bytes, &mut pos)?;
        let from_grade = decode_string(bytes, &mut pos)?;
        let to_grade = decode_string(bytes, &mut pos)?;
        let effective_date = decode_string(bytes, &mut pos)?;
        let transaction_date = decode_string(bytes, &mut pos)?;
        Ok(PersonnelPromoted {
            base,
            person_id,
            from_grade,
            to_grade,
            effective_date,
            transaction_date,
        })
    }
}

/// Event emitted when a person separates from the organization
/// (DOM-004). `separation_reason` is a free-form string (e.g. `"ETS"`,
/// `"MEDICAL_DISCHARGE"`, `"RETIREMENT"`) -- consuming teams should
/// treat it as advisory rather than a closed enum until a controlled
/// vocabulary is established.
#[derive(Debug, Clone, PartialEq)]
pub struct PersonnelSeparated {
    pub base: BaseEvent,
    pub person_id: String,
    pub separation_reason: String,
    pub effective_date: String,
    pub transaction_date: String,
}

impl PersonnelSeparated {
    pub const NAMESPACE: &'static str = "meshed.domains.personnel";

    pub fn new(
        correlation_id: impl Into<String>,
        person_id: impl Into<String>,
        separation_reason: impl Into<String>,
        effective_date: impl Into<String>,
        transaction_date: impl Into<String>,
    ) -> Self {
        PersonnelSeparated {
            base: BaseEvent::new(correlation_id),
            person_id: person_id.into(),
            separation_reason: separation_reason.into(),
            effective_date: effective_date.into(),
            transaction_date: transaction_date.into(),
        }
    }

    pub fn avro_schema() -> String {
        BaseEvent::avro_record_schema(
            "PersonnelSeparated",
            Self::NAMESPACE,
            json!([
                {"name": "person_id", "type": "string"},
                {"name": "separation_reason", "type": "string"},
                {"name": "effective_date", "type": "string"},
                {"name": "transaction_date", "type": "string"}
            ]),
        )
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.base.encode_into(&mut out);
        encode_string(&self.person_id, &mut out);
        encode_string(&self.separation_reason, &mut out);
        encode_string(&self.effective_date, &mut out);
        encode_string(&self.transaction_date, &mut out);
        out
    }

    pub fn deserialize(bytes: &[u8]) -> Result<Self, AvroDecodeError> {
        let mut pos = 0;
        let base = BaseEvent::decode_from(bytes, &mut pos)?;
        let person_id = decode_string(bytes, &mut pos)?;
        let separation_reason = decode_string(bytes, &mut pos)?;
        let effective_date = decode_string(bytes, &mut pos)?;
        let transaction_date = decode_string(bytes, &mut pos)?;
        Ok(PersonnelSeparated {
            base,
            person_id,
            separation_reason,
            effective_date,
            transaction_date,
        })
    }
}

/// Event emitted when a person's duty/availability status changes
/// (DOM-005). Examples of status transitions: `PRESENT_FOR_DUTY ->
/// TDY`, `PRESENT_FOR_DUTY -> LEAVE`, `TDY -> PRESENT_FOR_DUTY`.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusChanged {
    pub base: BaseEvent,
    pub person_id: String,
    pub previous_status: String,
    pub new_status: String,
    pub effective_date: String,
    pub transaction_date: String,
}

impl StatusChanged {
    pub const NAMESPACE: &'static str = "meshed.domains.personnel";

    pub fn new(
        correlation_id: impl Into<String>,
        person_id: impl Into<String>,
        previous_status: impl Into<String>,
        new_status: impl Into<String>,
        effective_date: impl Into<String>,
        transaction_date: impl Into<String>,
    ) -> Self {
        StatusChanged {
            base: BaseEvent::new(correlation_id),
            person_id: person_id.into(),
            previous_status: previous_status.into(),
            new_status: new_status.into(),
            effective_date: effective_date.into(),
            transaction_date: transaction_date.into(),
        }
    }

    pub fn avro_schema() -> String {
        BaseEvent::avro_record_schema(
            "StatusChanged",
            Self::NAMESPACE,
            json!([
                {"name": "person_id", "type": "string"},
                {"name": "previous_status", "type": "string"},
                {"name": "new_status", "type": "string"},
                {"name": "effective_date", "type": "string"},
                {"name": "transaction_date", "type": "string"}
            ]),
        )
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.base.encode_into(&mut out);
        encode_string(&self.person_id, &mut out);
        encode_string(&self.previous_status, &mut out);
        encode_string(&self.new_status, &mut out);
        encode_string(&self.effective_date, &mut out);
        encode_string(&self.transaction_date, &mut out);
        out
    }

    pub fn deserialize(bytes: &[u8]) -> Result<Self, AvroDecodeError> {
        let mut pos = 0;
        let base = BaseEvent::decode_from(bytes, &mut pos)?;
        let person_id = decode_string(bytes, &mut pos)?;
        let previous_status = decode_string(bytes, &mut pos)?;
        let new_status = decode_string(bytes, &mut pos)?;
        let effective_date = decode_string(bytes, &mut pos)?;
        let transaction_date = decode_string(bytes, &mut pos)?;
        Ok(StatusChanged {
            base,
            person_id,
            previous_status,
            new_status,
            effective_date,
            transaction_date,
        })
    }
}

impl DomainEvent for PersonnelAssigned {
    const EVENT_NAME: &'static str = "PersonnelAssigned";

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
            "person_id".to_string(),
            rusty_json::Value::from(self.person_id.as_str()),
        );
        fields.insert(
            "position_id".to_string(),
            rusty_json::Value::from(self.position_id.as_str()),
        );
        fields.insert(
            "unit_uic".to_string(),
            rusty_json::Value::from(self.unit_uic.as_str()),
        );
        fields.insert(
            "duty_title".to_string(),
            rusty_json::Value::from(self.duty_title.as_str()),
        );
        fields.insert(
            "grade".to_string(),
            rusty_json::Value::from(self.grade.as_str()),
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

impl DomainEvent for PersonnelPromoted {
    const EVENT_NAME: &'static str = "PersonnelPromoted";

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
            "person_id".to_string(),
            rusty_json::Value::from(self.person_id.as_str()),
        );
        fields.insert(
            "from_grade".to_string(),
            rusty_json::Value::from(self.from_grade.as_str()),
        );
        fields.insert(
            "to_grade".to_string(),
            rusty_json::Value::from(self.to_grade.as_str()),
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

impl DomainEvent for PersonnelSeparated {
    const EVENT_NAME: &'static str = "PersonnelSeparated";

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
            "person_id".to_string(),
            rusty_json::Value::from(self.person_id.as_str()),
        );
        fields.insert(
            "separation_reason".to_string(),
            rusty_json::Value::from(self.separation_reason.as_str()),
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

impl DomainEvent for StatusChanged {
    const EVENT_NAME: &'static str = "StatusChanged";

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
            "person_id".to_string(),
            rusty_json::Value::from(self.person_id.as_str()),
        );
        fields.insert(
            "previous_status".to_string(),
            rusty_json::Value::from(self.previous_status.as_str()),
        );
        fields.insert(
            "new_status".to_string(),
            rusty_json::Value::from(self.new_status.as_str()),
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
    fn personnel_assigned_avro_schema_includes_lineage_and_own_fields() {
        let schema = PersonnelAssigned::avro_schema();
        let parsed: rusty_json::Value = rusty_json::from_str(&schema).unwrap();
        assert_eq!(
            parsed.get("namespace").unwrap().as_str(),
            Some("meshed.domains.personnel")
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
                "person_id",
                "position_id",
                "unit_uic",
                "duty_title",
                "grade",
                "effective_date",
                "transaction_date",
            ]
        );
    }

    #[test]
    fn personnel_assigned_to_json_includes_lineage_and_own_fields() {
        let mut event = PersonnelAssigned::new(
            "req-1",
            "p-1",
            "pos-1",
            "UIC-1",
            "Rifleman",
            "E4",
            "2026-01-01",
            "2026-01-02",
        );
        event.base.source_event_ids = vec!["upstream-1".to_string()];
        let json = event.to_json();
        let object = json.as_object().unwrap();
        assert_eq!(
            object.get("event_id").unwrap().as_str(),
            Some(event.base.event_id.as_str())
        );
        assert_eq!(
            object.get("correlation_id").unwrap().as_str(),
            Some("req-1")
        );
        assert_eq!(
            object
                .get("source_event_ids")
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(object.get("person_id").unwrap().as_str(), Some("p-1"));
        assert_eq!(object.get("grade").unwrap().as_str(), Some("E4"));
        assert_eq!(
            object.get("transaction_date").unwrap().as_str(),
            Some("2026-01-02")
        );
    }

    #[test]
    fn personnel_assigned_serialize_then_deserialize_round_trips() {
        let event = PersonnelAssigned::new(
            "req-1",
            "p-1",
            "pos-1",
            "UIC-1",
            "Rifleman",
            "E4",
            "2026-01-01",
            "2026-01-02",
        );
        let bytes = event.serialize();
        assert_eq!(PersonnelAssigned::deserialize(&bytes).unwrap(), event);
    }

    #[test]
    fn personnel_promoted_serialize_then_deserialize_round_trips() {
        let event = PersonnelPromoted::new("req-1", "p-1", "E4", "E5", "2026-01-01", "2026-01-02");
        let bytes = event.serialize();
        assert_eq!(PersonnelPromoted::deserialize(&bytes).unwrap(), event);
        assert_eq!(PersonnelPromoted::NAMESPACE, "meshed.domains.personnel");
    }

    #[test]
    fn personnel_separated_serialize_then_deserialize_round_trips() {
        let event = PersonnelSeparated::new("req-1", "p-1", "ETS", "2026-01-01", "2026-01-02");
        let bytes = event.serialize();
        assert_eq!(PersonnelSeparated::deserialize(&bytes).unwrap(), event);
    }

    #[test]
    fn status_changed_serialize_then_deserialize_round_trips() {
        let event = StatusChanged::new(
            "req-1",
            "p-1",
            "PRESENT_FOR_DUTY",
            "TDY",
            "2026-01-01",
            "2026-01-02",
        );
        let bytes = event.serialize();
        assert_eq!(StatusChanged::deserialize(&bytes).unwrap(), event);
    }

    #[test]
    fn correlation_id_and_source_event_ids_flow_through_the_embedded_base_event() {
        let mut event = PersonnelSeparated::new("req-1", "p-1", "ETS", "2026-01-01", "2026-01-02");
        event.base.source_event_ids = vec!["e-0".to_string()];
        let bytes = event.serialize();
        let decoded = PersonnelSeparated::deserialize(&bytes).unwrap();
        assert_eq!(decoded.base.correlation_id, "req-1");
        assert_eq!(decoded.base.source_event_ids, vec!["e-0".to_string()]);
    }
}
