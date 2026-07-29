use std::sync::Arc;

use skillopt_core::{BackendConfig, ChatBackend, Provider};

use crate::aisf_stage::AisfStageBackend;
use crate::anthropic::AnthropicBackend;
use crate::azure_openai::AzureOpenAiBackend;
use crate::mock::MockBackend;
use crate::openai_compat::OpenAiCompatBackend;

/// Instantiates a [`ChatBackend`] from a [`BackendConfig`], resolving the
/// API key from the environment (a provider-appropriate default variable
/// name unless `api_key_env` overrides it). The `mock` provider needs no
/// key and never touches the network.
pub fn build_backend(cfg: &BackendConfig) -> anyhow::Result<Arc<dyn ChatBackend>> {
    match cfg.provider {
        Provider::Mock => Ok(Arc::new(MockBackend::echo(cfg.model.clone()))),
        Provider::Anthropic => {
            let key_env = cfg
                .api_key_env
                .clone()
                .unwrap_or_else(|| "ANTHROPIC_API_KEY".to_string());
            let api_key = std::env::var(&key_env).map_err(|_| {
                anyhow::anyhow!("missing API key: environment variable {key_env} is not set")
            })?;
            Ok(Arc::new(AnthropicBackend::new(
                api_key,
                cfg.base_url.clone(),
                cfg.model.clone(),
                cfg.temperature,
                cfg.max_tokens,
            )))
        }
        Provider::OpenAiCompatible => {
            // If `api_key_env` is explicitly set, that variable must be
            // present (the user asked for it by name). Otherwise fall back
            // to OPENAI_API_KEY if set, or no auth at all if not - local
            // servers like Ollama don't check it, so requiring a dummy value
            // would just be friction.
            let api_key = match &cfg.api_key_env {
                Some(key_env) => Some(std::env::var(key_env).map_err(|_| {
                    anyhow::anyhow!("missing API key: environment variable {key_env} is not set")
                })?),
                None => std::env::var("OPENAI_API_KEY").ok(),
            };
            Ok(Arc::new(OpenAiCompatBackend::new(
                api_key,
                cfg.base_url.clone(),
                cfg.model.clone(),
                cfg.temperature,
                cfg.max_tokens,
            )))
        }
        Provider::AzureOpenAi => {
            let key_env = cfg
                .api_key_env
                .clone()
                .unwrap_or_else(|| "AZURE_OPENAI_API_KEY".to_string());
            let api_key = std::env::var(&key_env).map_err(|_| {
                anyhow::anyhow!("missing API key: environment variable {key_env} is not set")
            })?;
            let endpoint = cfg.base_url.clone().ok_or_else(|| {
                anyhow::anyhow!(
                    "azure_openai requires base_url (the resource endpoint, \
                     e.g. https://my-resource.openai.azure.com)"
                )
            })?;
            Ok(Arc::new(AzureOpenAiBackend::new(
                api_key,
                endpoint,
                cfg.model.clone(),
                cfg.api_version.clone(),
                cfg.temperature,
                cfg.max_tokens,
            )))
        }
        Provider::AisfStage => {
            let binary_path = cfg.aisf_binary_path.clone().ok_or_else(|| {
                anyhow::anyhow!(
                    "aisf_stage requires aisf_binary_path (path to AISF's built \
                     `software-factory` binary)"
                )
            })?;
            // `model` is reinterpreted as the AISF stage name for this
            // provider -- see BackendConfig::model's doc comment.
            Ok(Arc::new(AisfStageBackend::new(
                binary_path,
                cfg.model.clone(),
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skillopt_core::BackendConfig;

    fn cfg(api_key_env: Option<&str>) -> BackendConfig {
        BackendConfig {
            provider: Provider::OpenAiCompatible,
            model: "llama3".into(),
            base_url: Some("http://localhost:11434/v1".into()),
            api_key_env: api_key_env.map(|s| s.to_string()),
            temperature: None,
            max_tokens: 64,
            api_version: None,
            aisf_binary_path: None,
        }
    }

    // Both cases share one test (rather than two #[test] fns) because they
    // mutate the process-global OPENAI_API_KEY env var, and Rust runs tests
    // in the same process concurrently by default.
    #[test]
    fn openai_compatible_key_resolution() {
        std::env::remove_var("OPENAI_API_KEY");

        // No api_key_env and no OPENAI_API_KEY set: succeeds with no key,
        // matching a local server like Ollama that doesn't check auth.
        assert!(build_backend(&cfg(None)).is_ok());

        // OPENAI_API_KEY set with no explicit api_key_env: picked up.
        std::env::set_var("OPENAI_API_KEY", "sk-fallback");
        assert!(build_backend(&cfg(None)).is_ok());
        std::env::remove_var("OPENAI_API_KEY");

        // Explicit api_key_env pointing at an unset variable: errors rather
        // than silently proceeding with no auth, since the user named it.
        std::env::remove_var("SOME_OTHER_KEY");
        assert!(build_backend(&cfg(Some("SOME_OTHER_KEY"))).is_err());

        // Explicit api_key_env pointing at a set variable: succeeds.
        std::env::set_var("SOME_OTHER_KEY", "sk-explicit");
        assert!(build_backend(&cfg(Some("SOME_OTHER_KEY"))).is_ok());
        std::env::remove_var("SOME_OTHER_KEY");
    }

    #[test]
    fn azure_openai_requires_base_url_and_key() {
        std::env::remove_var("AZURE_OPENAI_API_KEY");

        let base = BackendConfig {
            provider: Provider::AzureOpenAi,
            model: "gpt-4o-deployment".into(),
            base_url: None,
            api_key_env: None,
            temperature: None,
            max_tokens: 64,
            api_version: None,
            aisf_binary_path: None,
        };

        // No base_url: errors, regardless of key.
        std::env::set_var("AZURE_OPENAI_API_KEY", "sk-azure");
        assert!(build_backend(&base).is_err());

        // base_url set but no key: errors.
        std::env::remove_var("AZURE_OPENAI_API_KEY");
        let with_url = BackendConfig {
            base_url: Some("https://my-resource.openai.azure.com".into()),
            ..base
        };
        assert!(build_backend(&with_url).is_err());

        // Both set: succeeds.
        std::env::set_var("AZURE_OPENAI_API_KEY", "sk-azure");
        assert!(build_backend(&with_url).is_ok());
        std::env::remove_var("AZURE_OPENAI_API_KEY");
    }

    #[test]
    fn aisf_stage_requires_binary_path() {
        let without_path = BackendConfig {
            provider: Provider::AisfStage,
            model: "triage".into(),
            base_url: None,
            api_key_env: None,
            temperature: None,
            max_tokens: 64,
            api_version: None,
            aisf_binary_path: None,
        };
        assert!(build_backend(&without_path).is_err());

        let with_path = BackendConfig {
            aisf_binary_path: Some("/path/to/software-factory".into()),
            ..without_path
        };
        // No key/network requirement -- constructing the backend never
        // touches the binary, so this succeeds even though the path isn't
        // real; only an actual `chat()` call would try to spawn it.
        assert!(build_backend(&with_path).is_ok());
    }
}
