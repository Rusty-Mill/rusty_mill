//! [`PositionManagementProducer`] -- the Rust port of
//! `meshed.domains.products.position_management.PositionManagementProducer`
//! (DOM-020/021).

use crate::events::{
    PositionAuthorizationChanged, PositionFilled, PositionModified, PositionVacated,
};
use rusty_meshed_core::EventType;
use rusty_meshed_sdk::{OutputPortSpec, PortDescriptor};

/// Metadata and output-port declarations for the position-management
/// data product. Uses `DataProductProducerBase::publish` unchanged --
/// no outbox override, unlike [`crate::products::PersonnelLifecycleProducer`]
/// (DOM-021's own note). A caller builds the real producer directly
/// via `DataProductProducerBase::connect(PositionManagementProducer::PRODUCT_NAME,
/// ..., PositionManagementProducer::output_ports(), config)`.
pub struct PositionManagementProducer;

impl PositionManagementProducer {
    pub const PRODUCT_NAME: &'static str = "position-management";
    pub const DOMAIN: &'static str = "manpower";
    pub const VERSION: &'static str = "1.0.0";
    pub const OWNER: &'static str = "manpower-team";
    pub const DESCRIPTION: &'static str =
        "Position management: authorization changes, fills, vacancies, modifications";

    /// The four output ports (DOM-021), all `EventType::Delta`.
    pub fn output_ports() -> Vec<PortDescriptor> {
        vec![
            OutputPortSpec::<PositionAuthorizationChanged>::new(
                "authorization-changes",
                "manpower.position-management.authorization-changes",
                EventType::Delta,
            )
            .describe(),
            OutputPortSpec::<PositionFilled>::new(
                "fills",
                "manpower.position-management.fills",
                EventType::Delta,
            )
            .describe(),
            OutputPortSpec::<PositionVacated>::new(
                "vacancies",
                "manpower.position-management.vacancies",
                EventType::Delta,
            )
            .describe(),
            OutputPortSpec::<PositionModified>::new(
                "modifications",
                "manpower.position-management.modifications",
                EventType::Delta,
            )
            .describe(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_ports_declares_four_delta_ports_under_the_right_topics() {
        let ports = PositionManagementProducer::output_ports();
        assert_eq!(ports.len(), 4);
        assert!(ports
            .iter()
            .all(|p| p.event_classification == EventType::Delta));

        let names: Vec<&str> = ports.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "authorization-changes",
                "fills",
                "vacancies",
                "modifications"
            ]
        );

        assert_eq!(ports[1].topic, "manpower.position-management.fills");
    }

    #[test]
    fn metadata_matches_the_source() {
        assert_eq!(
            PositionManagementProducer::PRODUCT_NAME,
            "position-management"
        );
        assert_eq!(PositionManagementProducer::DOMAIN, "manpower");
        assert_eq!(PositionManagementProducer::VERSION, "1.0.0");
        assert_eq!(PositionManagementProducer::OWNER, "manpower-team");
    }
}
