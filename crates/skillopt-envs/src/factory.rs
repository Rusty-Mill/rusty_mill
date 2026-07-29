use std::sync::Arc;

use skillopt_core::{EnvConfig, Environment};

use crate::aisf_triage::{AisfTriageEnv, AisfTriageParams};
use crate::aisf_validation::{AisfValidationEnv, AisfValidationParams};
use crate::synthetic_arithmetic::{SyntheticArithmeticEnv, SyntheticArithmeticParams};

/// Instantiates an [`Environment`] by name. `synthetic_arithmetic`,
/// `aisf_triage`, and `aisf_validation` are the built-in benchmarks
/// today; new ones register here the same way SkillOpt registers new
/// benchmarks under `skillopt/envs/<name>/`.
pub fn build_env(cfg: &EnvConfig) -> anyhow::Result<Arc<dyn Environment>> {
    match cfg.name.as_str() {
        "synthetic_arithmetic" => {
            let params: SyntheticArithmeticParams = if cfg.params.is_null() {
                SyntheticArithmeticParams::default()
            } else {
                serde_yaml::from_value(cfg.params.clone()).map_err(|e| {
                    anyhow::anyhow!("invalid params for synthetic_arithmetic env: {e}")
                })?
            };
            Ok(Arc::new(SyntheticArithmeticEnv::new(params)))
        }
        "aisf_triage" => {
            let params: AisfTriageParams = serde_yaml::from_value(cfg.params.clone())
                .map_err(|e| anyhow::anyhow!("invalid params for aisf_triage env: {e}"))?;
            Ok(Arc::new(AisfTriageEnv::new(params)?))
        }
        "aisf_validation" => {
            let params: AisfValidationParams = serde_yaml::from_value(cfg.params.clone())
                .map_err(|e| anyhow::anyhow!("invalid params for aisf_validation env: {e}"))?;
            Ok(Arc::new(AisfValidationEnv::new(params)?))
        }
        other => anyhow::bail!(
            "unknown environment: {other:?} (known: synthetic_arithmetic, aisf_triage, \
             aisf_validation)"
        ),
    }
}
