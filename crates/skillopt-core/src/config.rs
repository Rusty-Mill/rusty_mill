use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Which `ChatBackend` implementation to construct. `skillopt-model` owns
/// the actual construction logic; core only needs to name the choice so it
/// round-trips through YAML.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Anthropic,
    // `snake_case` alone would derive "open_ai_compatible" ("Ai" is treated
    // as its own word); every doc/config in this repo uses "openai_compatible".
    #[serde(rename = "openai_compatible")]
    OpenAiCompatible,
    // Same pitfall: `snake_case` alone derives "azure_open_ai", not the
    // "azure_openai" every doc/config in this repo uses.
    #[serde(rename = "azure_openai")]
    AzureOpenAi,
    // `snake_case` alone would already derive "aisf_stage" here (no
    // internal-capital pitfall the way "OpenAi"/"AzureOpenAi" have), but
    // every other provider in this file spells its wire string out
    // explicitly rather than relying on the derive being right, and this
    // one shouldn't be the exception that silently breaks if `rename_all`'s
    // behavior ever changes.
    #[serde(rename = "aisf_stage")]
    AisfStage,
    Mock,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendConfig {
    pub provider: Provider,
    /// For every provider except `aisf_stage`, the model name. `aisf_stage`
    /// reinterprets this as the AISF pipeline stage to drive (e.g.
    /// `"triage"`) -- the same kind of per-provider reinterpretation
    /// `azure_openai` already does (`model` = deployment name there).
    pub model: String,
    #[serde(default)]
    pub base_url: Option<String>,
    /// Name of the environment variable holding the API key. Defaults to a
    /// provider-appropriate name (e.g. `ANTHROPIC_API_KEY`) when unset.
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    /// Azure OpenAI's required `api-version` query param. Ignored by every
    /// other provider. Defaults to a recent stable GA version if unset.
    #[serde(default)]
    pub api_version: Option<String>,
    /// `aisf_stage`'s path to the built AISF `software-factory` binary.
    /// Ignored by every other provider.
    #[serde(default)]
    pub aisf_binary_path: Option<PathBuf>,
}

fn default_max_tokens() -> u32 {
    1024
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrainConfig {
    #[serde(default = "default_epochs")]
    pub epochs: usize,
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
    #[serde(default = "default_val_batch_size")]
    pub val_batch_size: usize,
    /// Upper bound on the number of `EditOp`s the optimizer may propose in
    /// a single step (the text-space analogue of a learning-rate cap).
    #[serde(default = "default_max_ops_per_edit")]
    pub max_ops_per_edit: usize,
    /// How many recent rejected-edit rationales to keep and show the
    /// optimizer, so it doesn't retry the same rejected change.
    #[serde(default = "default_rejection_buffer_size")]
    pub rejection_buffer_size: usize,
    /// Minimum validation-score improvement over the current best required
    /// for the gate to accept a candidate edit.
    #[serde(default = "default_min_improvement")]
    pub min_improvement: f64,
    #[serde(default = "default_seed")]
    pub seed: u64,
}

fn default_epochs() -> usize {
    3
}
fn default_batch_size() -> usize {
    4
}
fn default_val_batch_size() -> usize {
    8
}
fn default_max_ops_per_edit() -> usize {
    3
}
fn default_rejection_buffer_size() -> usize {
    5
}
fn default_min_improvement() -> f64 {
    1e-6
}
fn default_seed() -> u64 {
    0
}

impl Default for TrainConfig {
    fn default() -> Self {
        Self {
            epochs: default_epochs(),
            batch_size: default_batch_size(),
            val_batch_size: default_val_batch_size(),
            max_ops_per_edit: default_max_ops_per_edit(),
            rejection_buffer_size: default_rejection_buffer_size(),
            min_improvement: default_min_improvement(),
            seed: default_seed(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvConfig {
    pub name: String,
    #[serde(default)]
    pub params: serde_yaml::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunConfig {
    pub skill_path: PathBuf,
    pub output_dir: PathBuf,
    pub executor: BackendConfig,
    pub optimizer: BackendConfig,
    pub reflector: BackendConfig,
    #[serde(default)]
    pub train: TrainConfig,
    pub env: EnvConfig,
}

impl RunConfig {
    pub fn from_yaml_str(s: &str) -> anyhow::Result<Self> {
        Ok(serde_yaml::from_str(s)?)
    }

    pub fn from_file(path: &std::path::Path) -> anyhow::Result<Self> {
        let s = std::fs::read_to_string(path)?;
        Self::from_yaml_str(&s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_config_with_defaults() {
        let yaml = r#"
skill_path: skills/initial.md
output_dir: out
executor:
  provider: mock
  model: mock-executor
optimizer:
  provider: mock
  model: mock-optimizer
reflector:
  provider: mock
  model: mock-reflector
env:
  name: synthetic_arithmetic
"#;
        let cfg = RunConfig::from_yaml_str(yaml).unwrap();
        assert_eq!(cfg.train.epochs, 3);
        assert_eq!(cfg.train.batch_size, 4);
        assert_eq!(cfg.executor.provider, Provider::Mock);
        assert_eq!(cfg.env.name, "synthetic_arithmetic");
    }

    #[test]
    fn provider_yaml_names_match_documented_strings() {
        // Every doc/example config in this repo writes these exact strings;
        // a derive-only rename_all would silently produce
        // "open_ai_compatible" instead and break every one of them.
        assert_eq!(
            serde_yaml::from_str::<Provider>("anthropic").unwrap(),
            Provider::Anthropic
        );
        assert_eq!(
            serde_yaml::from_str::<Provider>("openai_compatible").unwrap(),
            Provider::OpenAiCompatible
        );
        assert_eq!(
            serde_yaml::from_str::<Provider>("azure_openai").unwrap(),
            Provider::AzureOpenAi
        );
        assert_eq!(
            serde_yaml::from_str::<Provider>("aisf_stage").unwrap(),
            Provider::AisfStage
        );
        assert_eq!(
            serde_yaml::from_str::<Provider>("mock").unwrap(),
            Provider::Mock
        );
    }
}
