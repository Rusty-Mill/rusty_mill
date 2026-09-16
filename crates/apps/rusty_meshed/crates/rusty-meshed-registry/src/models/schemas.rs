//! Create/Public/Update request-and-response shapes -- the Rust port
//! of `meshed.registry.schemas` (REG-026..033, REG-139). See the
//! parent module doc for why most of these are thin wrappers or plain
//! aliases rather than separate structs: there's no ORM lazy-loading
//! for a distinct "public" type to guard against.

use super::enums::MaturityTier;
use super::{DataContract, InputPort, OutputPort, PortAccessGrant};
use rusty_err::Error;
use rusty_meshed_core::EventType;
use rusty_meshed_governance::{GovernanceEngine, GovernedProduct};
use std::sync::OnceLock;

/// Request body for registering a data product (REG-026's sibling --
/// the Create side). `maturity_tier` defaults to
/// [`MaturityTier::Mvp`] (REG-016) and `tags` to `"[]"` (REG-017) when
/// not overridden via the `with_*` builders.
#[derive(Debug, Clone, PartialEq)]
pub struct DataProductCreate {
    pub name: String,
    pub owner: String,
    pub version: String,
    pub domain: String,
    pub description: String,
    pub maturity_tier: MaturityTier,
    pub tags: String,
}

impl DataProductCreate {
    pub fn new(
        name: impl Into<String>,
        owner: impl Into<String>,
        version: impl Into<String>,
        domain: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        DataProductCreate {
            name: name.into(),
            owner: owner.into(),
            version: version.into(),
            domain: domain.into(),
            description: description.into(),
            maturity_tier: MaturityTier::default(),
            tags: "[]".to_string(),
        }
    }

    pub fn with_maturity_tier(mut self, maturity_tier: MaturityTier) -> Self {
        self.maturity_tier = maturity_tier;
        self
    }

    pub fn with_tags(mut self, tags: impl Into<String>) -> Self {
        self.tags = tags.into();
        self
    }
}

impl GovernedProduct for DataProductCreate {
    fn description(&self) -> Option<&str> {
        Some(self.description.as_str())
    }
    fn version(&self) -> Option<&str> {
        Some(self.version.as_str())
    }
    fn domain(&self) -> Option<&str> {
        Some(self.domain.as_str())
    }
}

/// The module-level `_DEFAULT_ENGINE` singleton (REG-043): the same
/// governance engine instance gates both `POST /data-products` and
/// `POST /governance/evaluate`, built once on first use.
pub fn default_governance_engine() -> &'static GovernanceEngine<DataProductCreate> {
    static ENGINE: OnceLock<GovernanceEngine<DataProductCreate>> = OnceLock::new();
    ENGINE.get_or_init(rusty_meshed_governance::default_engine)
}

/// Partial update body for a data product (REG-026). Every field is
/// `None` by default; only fields explicitly set are meant to be
/// applied by the caller (`exclude_unset=True` semantics -- a
/// `None` here means "leave unchanged", not "clear this field").
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DataProductUpdate {
    pub name: Option<String>,
    pub owner: Option<String>,
    pub version: Option<String>,
    pub domain: Option<String>,
    pub description: Option<String>,
    pub maturity_tier: Option<MaturityTier>,
    pub tags: Option<String>,
}

/// Request body for registering an input port (REG-032):
/// `description` is optional, defaulting to `None`.
#[derive(Debug, Clone, PartialEq)]
pub struct InputPortCreate {
    pub topic_name: String,
    pub description: Option<String>,
}

impl InputPortCreate {
    pub fn new(topic_name: impl Into<String>) -> Self {
        InputPortCreate {
            topic_name: topic_name.into(),
            description: None,
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

/// Response schema for input port endpoints -- identical in shape to
/// the persisted row, so this is a plain alias (see the parent module
/// doc).
pub type InputPortPublic = InputPort;

/// Raised by [`OutputPortCreate::new`] when `event_type` isn't one of
/// [`EventType`]'s three wire values (REG-033).
#[derive(Debug, Error)]
pub enum OutputPortValidationError {
    #[error("'{0}' is not a valid EventType member")]
    InvalidEventType(String),
}

/// Request body for registering an output port (REG-033):
/// `event_type` is required and validated eagerly against
/// [`EventType`]'s members, matching the API layer's 422-on-invalid
/// behavior.
#[derive(Debug, Clone, PartialEq)]
pub struct OutputPortCreate {
    pub topic_name: String,
    pub schema_subject: String,
    pub event_type: EventType,
    pub description: Option<String>,
}

impl OutputPortCreate {
    pub fn new(
        topic_name: impl Into<String>,
        schema_subject: impl Into<String>,
        event_type: &str,
    ) -> Result<Self, OutputPortValidationError> {
        let event_type = EventType::parse(event_type)
            .ok_or_else(|| OutputPortValidationError::InvalidEventType(event_type.to_string()))?;
        Ok(OutputPortCreate {
            topic_name: topic_name.into(),
            schema_subject: schema_subject.into(),
            event_type,
            description: None,
        })
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

/// Response schema for output port endpoints -- a plain alias, as with
/// [`InputPortPublic`].
pub type OutputPortPublic = OutputPort;

/// Raised by [`DataContractCreate::new`] (REG-027..030): the Rust
/// equivalent of the source's pydantic 422s (a field-level
/// `Field(ge=0.0, le=100.0)` constraint plus an `@model_validator`
/// checking the other three).
#[derive(Debug, Error)]
pub enum DataContractValidationError {
    #[error("slo_completeness_pct must be between 0.0 and 100.0, got {0}")]
    CompletenessOutOfRange(f64),
    #[error("schema_ref must not be empty or blank")]
    EmptySchemaRef,
    #[error("owner must not be empty or blank")]
    EmptyOwner,
    #[error("quality_assertions must contain at least one assertion")]
    EmptyQualityAssertions,
}

/// Request body for registering a data contract. Validates eagerly at
/// construction, in the same order the source's field constraint then
/// `model_validator(mode="after")` run: completeness range, then
/// blank `schema_ref`, then blank `owner`, then empty
/// `quality_assertions`. `slo_freshness_seconds` is a plain `i64` with
/// no non-negativity constraint (REG-139), matching the source.
#[derive(Debug, Clone, PartialEq)]
pub struct DataContractCreate {
    pub schema_ref: String,
    pub owner: String,
    pub slo_freshness_seconds: i64,
    pub slo_completeness_pct: f64,
    pub quality_assertions: Vec<String>,
}

impl DataContractCreate {
    pub fn new(
        schema_ref: impl Into<String>,
        owner: impl Into<String>,
        slo_freshness_seconds: i64,
        slo_completeness_pct: f64,
        quality_assertions: Vec<String>,
    ) -> Result<Self, DataContractValidationError> {
        let schema_ref = schema_ref.into();
        let owner = owner.into();

        if !(0.0..=100.0).contains(&slo_completeness_pct) {
            return Err(DataContractValidationError::CompletenessOutOfRange(
                slo_completeness_pct,
            ));
        }
        if schema_ref.trim().is_empty() {
            return Err(DataContractValidationError::EmptySchemaRef);
        }
        if owner.trim().is_empty() {
            return Err(DataContractValidationError::EmptyOwner);
        }
        if quality_assertions.is_empty() {
            return Err(DataContractValidationError::EmptyQualityAssertions);
        }

        Ok(DataContractCreate {
            schema_ref,
            owner,
            slo_freshness_seconds,
            slo_completeness_pct,
            quality_assertions,
        })
    }
}

/// Response schema for data contract endpoints. `quality_assertions`
/// is always a decoded `Vec<String>` here -- see
/// [`DataContractPublic::from_row`] -- unlike [`DataProduct`]'s `tags`,
/// which stays a raw JSON string in its public view (REG-138).
///
/// [`DataProduct`]: super::DataProduct
#[derive(Debug, Clone, PartialEq)]
pub struct DataContractPublic {
    pub id: i64,
    pub output_port_id: i64,
    pub schema_ref: String,
    pub owner: String,
    pub slo_freshness_seconds: i64,
    pub slo_completeness_pct: f64,
    pub quality_assertions: Vec<String>,
}

impl DataContractPublic {
    /// Builds the public view from a persisted [`DataContract`] row,
    /// decoding `quality_assertions` from its JSON-encoded storage
    /// form back into a `Vec<String>` (REG-031).
    pub fn from_row(row: &DataContract) -> Self {
        let quality_assertions =
            rusty_json::from_str::<Vec<String>>(&row.quality_assertions).unwrap_or_default();
        DataContractPublic {
            id: row.id,
            output_port_id: row.output_port_id,
            schema_ref: row.schema_ref.clone(),
            owner: row.owner.clone(),
            slo_freshness_seconds: row.slo_freshness_seconds,
            slo_completeness_pct: row.slo_completeness_pct,
            quality_assertions,
        }
    }
}

/// Request body for creating a port access grant -- the Rust port of
/// `meshed.governance.rbac.PortAccessGrantCreate` (GOV-013). No
/// validation beyond field presence: the source's schema has none
/// either (unlike `DataContractCreate`), and the router itself is
/// what enforces the port-exists (404) and duplicate-grant (409)
/// rules (GOV-014, GOV-015).
#[derive(Debug, Clone, PartialEq)]
pub struct PortAccessGrantCreate {
    pub output_port_id: i64,
    pub consumer_group_id: String,
    pub granted_by: String,
}

impl PortAccessGrantCreate {
    pub fn new(
        output_port_id: i64,
        consumer_group_id: impl Into<String>,
        granted_by: impl Into<String>,
    ) -> Self {
        PortAccessGrantCreate {
            output_port_id,
            consumer_group_id: consumer_group_id.into(),
            granted_by: granted_by.into(),
        }
    }
}

/// Response schema for access-grant endpoints -- a plain alias, as
/// with [`InputPortPublic`]/[`OutputPortPublic`] (see the parent
/// module doc).
pub type PortAccessGrantPublic = PortAccessGrant;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_product_create_defaults_maturity_tier_and_tags() {
        let create =
            DataProductCreate::new("orders", "team-a", "1.0.0", "commerce", "Order events");
        assert_eq!(create.maturity_tier, MaturityTier::Mvp);
        assert_eq!(create.tags, "[]");
    }

    #[test]
    fn data_product_create_builders_override_the_defaults() {
        let create =
            DataProductCreate::new("orders", "team-a", "1.0.0", "commerce", "Order events")
                .with_maturity_tier(MaturityTier::Mature)
                .with_tags(r#"["finance","audit"]"#);
        assert_eq!(create.maturity_tier, MaturityTier::Mature);
        assert_eq!(create.tags, r#"["finance","audit"]"#);
    }

    #[test]
    fn data_product_create_exposes_governed_product_fields() {
        let create =
            DataProductCreate::new("orders", "team-a", "1.0.0", "commerce", "Order events");
        assert_eq!(create.description(), Some("Order events"));
        assert_eq!(create.version(), Some("1.0.0"));
        assert_eq!(create.domain(), Some("commerce"));
    }

    #[test]
    fn default_governance_engine_runs_the_three_built_in_policies() {
        let create = DataProductCreate::new("orders", "team-a", "bad-version", "Commerce", "short");
        let violations = default_governance_engine().evaluate(&create);
        assert_eq!(violations.len(), 3);
    }

    #[test]
    fn default_governance_engine_is_the_same_instance_across_calls() {
        let first: *const _ = default_governance_engine();
        let second: *const _ = default_governance_engine();
        assert_eq!(first, second);
    }

    #[test]
    fn data_product_update_defaults_to_all_none() {
        let update = DataProductUpdate::default();
        assert_eq!(
            update,
            DataProductUpdate {
                name: None,
                owner: None,
                version: None,
                domain: None,
                description: None,
                maturity_tier: None,
                tags: None,
            }
        );
    }

    #[test]
    fn input_port_create_description_defaults_to_none() {
        let create = InputPortCreate::new("upstream.topic");
        assert_eq!(create.description, None);
        let create = create.with_description("some notes");
        assert_eq!(create.description, Some("some notes".to_string()));
    }

    #[test]
    fn output_port_create_accepts_a_valid_event_type() {
        let create =
            OutputPortCreate::new("downstream.topic", "downstream.topic-value", "state").unwrap();
        assert_eq!(create.event_type, EventType::State);
    }

    #[test]
    fn output_port_create_rejects_an_invalid_event_type() {
        let err = OutputPortCreate::new("downstream.topic", "downstream.topic-value", "not-real")
            .unwrap_err();
        match err {
            OutputPortValidationError::InvalidEventType(value) => assert_eq!(value, "not-real"),
        }
    }

    fn valid_contract_args() -> (&'static str, &'static str, i64, f64, Vec<String>) {
        (
            "downstream.topic-value:1",
            "team-a",
            60,
            99.5,
            vec!["no nulls in order_id".to_string()],
        )
    }

    #[test]
    fn data_contract_create_accepts_valid_input() {
        let (schema_ref, owner, freshness, completeness, assertions) = valid_contract_args();
        let create = DataContractCreate::new(
            schema_ref,
            owner,
            freshness,
            completeness,
            assertions.clone(),
        )
        .unwrap();
        assert_eq!(create.quality_assertions, assertions);
    }

    #[test]
    fn data_contract_create_rejects_completeness_out_of_range() {
        let (schema_ref, owner, freshness, _, assertions) = valid_contract_args();
        let err =
            DataContractCreate::new(schema_ref, owner, freshness, 150.0, assertions).unwrap_err();
        assert!(matches!(
            err,
            DataContractValidationError::CompletenessOutOfRange(150.0)
        ));
    }

    #[test]
    fn data_contract_create_rejects_blank_schema_ref() {
        let (_, owner, freshness, completeness, assertions) = valid_contract_args();
        let err =
            DataContractCreate::new("   ", owner, freshness, completeness, assertions).unwrap_err();
        assert!(matches!(err, DataContractValidationError::EmptySchemaRef));
    }

    #[test]
    fn data_contract_create_rejects_blank_owner() {
        let (schema_ref, _, freshness, completeness, assertions) = valid_contract_args();
        let err = DataContractCreate::new(schema_ref, "  ", freshness, completeness, assertions)
            .unwrap_err();
        assert!(matches!(err, DataContractValidationError::EmptyOwner));
    }

    #[test]
    fn data_contract_create_rejects_empty_quality_assertions() {
        let (schema_ref, owner, freshness, completeness, _) = valid_contract_args();
        let err = DataContractCreate::new(schema_ref, owner, freshness, completeness, vec![])
            .unwrap_err();
        assert!(matches!(
            err,
            DataContractValidationError::EmptyQualityAssertions
        ));
    }

    #[test]
    fn data_contract_create_allows_a_negative_freshness_seconds_no_constraint() {
        let (schema_ref, owner, _, completeness, assertions) = valid_contract_args();
        let create =
            DataContractCreate::new(schema_ref, owner, -5, completeness, assertions).unwrap();
        assert_eq!(create.slo_freshness_seconds, -5);
    }

    #[test]
    fn data_contract_public_decodes_quality_assertions_from_json() {
        let row = DataContract {
            id: 1,
            output_port_id: 2,
            schema_ref: "downstream.topic-value:1".to_string(),
            owner: "team-a".to_string(),
            slo_freshness_seconds: 60,
            slo_completeness_pct: 99.5,
            quality_assertions: r#"["no nulls in order_id","amount > 0"]"#.to_string(),
        };
        let public = DataContractPublic::from_row(&row);
        assert_eq!(
            public.quality_assertions,
            vec!["no nulls in order_id".to_string(), "amount > 0".to_string()]
        );
    }

    #[test]
    fn port_access_grant_create_builds_from_its_three_fields() {
        let create = PortAccessGrantCreate::new(2, "billing-service", "admin@example.com");
        assert_eq!(create.output_port_id, 2);
        assert_eq!(create.consumer_group_id, "billing-service");
        assert_eq!(create.granted_by, "admin@example.com");
    }
}
