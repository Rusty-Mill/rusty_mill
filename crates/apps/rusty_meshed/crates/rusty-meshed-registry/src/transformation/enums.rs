//! Transformation domain enumerations -- the Rust port of
//! `meshed.transformation.enums`.

/// Lifecycle status of a legacy system being migrated to the mesh.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SystemStatus {
    /// Sole source of truth; the mesh data product does not yet see
    /// production traffic for this track.
    Legacy,
    /// Legacy system and mesh data product both carry live data -- the
    /// safe strangler-fig migration state.
    DualWrite,
    /// Legacy system decommissioned after a dual-write period; mesh
    /// data product is now the sole source of truth. Clean cutover.
    Migrated,
    /// Legacy system turned off directly from `Legacy`, skipping
    /// dual-write. Fast, but risky -- consumers lose lineage
    /// continuity and capability regresses.
    Decommissioned,
}

impl SystemStatus {
    /// The wire/storage string value.
    pub fn as_str(self) -> &'static str {
        match self {
            SystemStatus::Legacy => "legacy",
            SystemStatus::DualWrite => "dual_write",
            SystemStatus::Migrated => "migrated",
            SystemStatus::Decommissioned => "decommissioned",
        }
    }

    /// Parses a stored string back into a status.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "legacy" => SystemStatus::Legacy,
            "dual_write" => SystemStatus::DualWrite,
            "migrated" => SystemStatus::Migrated,
            "decommissioned" => SystemStatus::Decommissioned,
            _ => return None,
        })
    }
}

/// The four data mesh principles, scored 0.0-5.0 per track per quarter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CapabilityDimension {
    DomainOwnership,
    DataAsAProduct,
    SelfServePlatform,
    FederatedGovernance,
}

impl CapabilityDimension {
    /// Every member, in the Python enum's declaration order.
    pub const ALL: [CapabilityDimension; 4] = [
        CapabilityDimension::DomainOwnership,
        CapabilityDimension::DataAsAProduct,
        CapabilityDimension::SelfServePlatform,
        CapabilityDimension::FederatedGovernance,
    ];

    /// The wire/storage string value.
    pub fn as_str(self) -> &'static str {
        match self {
            CapabilityDimension::DomainOwnership => "domain_ownership",
            CapabilityDimension::DataAsAProduct => "data_as_a_product",
            CapabilityDimension::SelfServePlatform => "self_serve_platform",
            CapabilityDimension::FederatedGovernance => "federated_governance",
        }
    }

    /// Parses a stored string back into a dimension.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "domain_ownership" => CapabilityDimension::DomainOwnership,
            "data_as_a_product" => CapabilityDimension::DataAsAProduct,
            "self_serve_platform" => CapabilityDimension::SelfServePlatform,
            "federated_governance" => CapabilityDimension::FederatedGovernance,
            _ => return None,
        })
    }
}

/// A transformation decision an operator can queue for the next
/// quarter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DecisionType {
    /// Begin dual-write for a track currently `Legacy`.
    MigrateTrack,
    /// Decommission a track's legacy system -- safe (clean cutover) if
    /// the track is `DualWrite`; risky (capability regression) if the
    /// track is still `Legacy`.
    SunsetLegacy,
    /// Fund the self-serve platform -- lifts `SelfServePlatform` across
    /// every track.
    InvestPlatform,
    /// Fund domain product teams -- lifts `DomainOwnership` and
    /// `DataAsAProduct` across every track.
    InvestProductTeams,
}

impl DecisionType {
    /// The wire/storage string value.
    pub fn as_str(self) -> &'static str {
        match self {
            DecisionType::MigrateTrack => "migrate_track",
            DecisionType::SunsetLegacy => "sunset_legacy",
            DecisionType::InvestPlatform => "invest_platform",
            DecisionType::InvestProductTeams => "invest_product_teams",
        }
    }

    /// Parses a stored string back into a decision type.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "migrate_track" => DecisionType::MigrateTrack,
            "sunset_legacy" => DecisionType::SunsetLegacy,
            "invest_platform" => DecisionType::InvestPlatform,
            "invest_product_teams" => DecisionType::InvestProductTeams,
            _ => return None,
        })
    }

    /// Whether this decision type's `target` is a track slug
    /// (`MigrateTrack`/`SunsetLegacy`) rather than `"platform"`/
    /// `"product_teams"` (the two investment decisions).
    pub fn targets_a_track(self) -> bool {
        matches!(
            self,
            DecisionType::MigrateTrack | DecisionType::SunsetLegacy
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_status_round_trips() {
        for status in [
            SystemStatus::Legacy,
            SystemStatus::DualWrite,
            SystemStatus::Migrated,
            SystemStatus::Decommissioned,
        ] {
            assert_eq!(SystemStatus::parse(status.as_str()), Some(status));
        }
    }

    #[test]
    fn capability_dimension_round_trips() {
        for dimension in CapabilityDimension::ALL {
            assert_eq!(
                CapabilityDimension::parse(dimension.as_str()),
                Some(dimension)
            );
        }
    }

    #[test]
    fn decision_type_round_trips() {
        for decision_type in [
            DecisionType::MigrateTrack,
            DecisionType::SunsetLegacy,
            DecisionType::InvestPlatform,
            DecisionType::InvestProductTeams,
        ] {
            assert_eq!(
                DecisionType::parse(decision_type.as_str()),
                Some(decision_type)
            );
        }
    }

    #[test]
    fn targets_a_track_is_true_only_for_migrate_and_sunset() {
        assert!(DecisionType::MigrateTrack.targets_a_track());
        assert!(DecisionType::SunsetLegacy.targets_a_track());
        assert!(!DecisionType::InvestPlatform.targets_a_track());
        assert!(!DecisionType::InvestProductTeams.targets_a_track());
    }
}
