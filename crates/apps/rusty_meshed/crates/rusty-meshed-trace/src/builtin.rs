//! Scenarios that ship with the crate.

use crate::model::Scenario;

/// The TOML text of the shipped *Acquisition Status Dashboard* scenario,
/// verbatim, for tooling that wants the file rather than the model.
pub const ACQUISITION_STATUS_TOML: &str = include_str!("../scenarios/acquisition_status.toml");

/// The shipped *Acquisition Status Dashboard* scenario: ten PAE
/// Fires-flavoured domains at mixed maturity and four leadership outcomes
/// (the dashboard itself, a program risk rollup, cost/schedule variance,
/// readiness status) that stress different domains.
///
/// The maturity levels are illustrative placeholders chosen to exercise
/// every trace state, not an assessment of any real program office.
pub fn acquisition_status() -> Scenario {
    Scenario::from_toml(ACQUISITION_STATUS_TOML)
        .expect("the bundled scenario is validated by this crate's own tests")
}
