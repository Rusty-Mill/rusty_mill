//! Per-provider endpoints, credentials and wire format.
//!
//! Everything that differs between providers is gathered here so the request
//! path in [`crate::LlmBackend`] reads the same regardless of who is being
//! called.

use agentgateway_config::{AiProvider, AiProviderParams};
use serde_json::Value;

use crate::translate::{self, TranslateError, Usage};

/// A provider configuration this build cannot serve.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// The provider itself is not implemented here.
    #[error(
        "{at}: the `{provider}` provider is not served by this build; \
         `openAI` (or any OpenAI-compatible endpoint via `hostOverride`) and `anthropic` are"
    )]
    Unsupported {
        /// Where in the configuration it came from.
        at: String,
        /// The provider that was asked for.
        provider: &'static str,
    },
    /// The base URL carries a credential.
    ///
    /// An address may legally hold `user:password@`, and that is the problem:
    /// a credential there hides somewhere nobody thinks to look, is sent on
    /// every request, and would be logged with the endpoint. `backendAuth.key`
    /// is where a provider credential belongs.
    #[error(
        "{at}.hostOverride: `{value}` carries userinfo, which does not belong in an upstream \
         address; use `backendAuth.key`"
    )]
    Userinfo {
        /// Where in the configuration it came from.
        at: String,
        /// The offending text.
        value: String,
    },
}

/// A configured model provider.
#[derive(Debug, Clone)]
pub enum Provider {
    /// OpenAI, or anything speaking its API.
    OpenAi(Settings),
    /// Anthropic's Messages API.
    Anthropic(Settings),
}

/// Settings shared by every provider.
#[derive(Debug, Clone)]
pub struct Settings {
    /// Model forced by configuration, if any.
    pub model: Option<String>,
    /// Base URL override, for a compatible endpoint or a proxy.
    pub host: Option<String>,
}

impl Provider {
    /// Resolve a configured provider.
    pub fn new(provider: &AiProvider, at: &str) -> Result<Self, ProviderError> {
        let settings = |params: &AiProviderParams| -> Result<Settings, ProviderError> {
            if let Some(host) = params.host_override.as_deref()
                && host.contains('@')
            {
                return Err(ProviderError::Userinfo {
                    at: at.to_string(),
                    value: host.to_string(),
                });
            }
            Ok(Settings {
                model: params.model.clone(),
                host: params.host_override.clone(),
            })
        };
        let unsupported = |provider| {
            Err(ProviderError::Unsupported {
                at: at.to_string(),
                provider,
            })
        };

        match provider {
            AiProvider::OpenAi(params) => Ok(Provider::OpenAi(settings(params)?)),
            AiProvider::Anthropic(params) => Ok(Provider::Anthropic(settings(params)?)),
            // Gemini, Vertex and Bedrock each need their own request shape and,
            // for the latter two, a signing scheme. Refusing at startup beats
            // accepting the config and failing every request.
            AiProvider::Gemini(_) => unsupported("gemini"),
            AiProvider::Vertex(_) => unsupported("vertex"),
            AiProvider::Bedrock(_) => unsupported("bedrock"),
        }
    }

    /// The provider's name, for logs.
    pub fn name(&self) -> &'static str {
        match self {
            Provider::OpenAi(_) => "openai",
            Provider::Anthropic(_) => "anthropic",
        }
    }

    fn settings(&self) -> &Settings {
        match self {
            Provider::OpenAi(settings) | Provider::Anthropic(settings) => settings,
        }
    }

    /// The model configuration forces, overriding the caller's.
    pub fn forced_model(&self) -> Option<&str> {
        self.settings().model.as_deref()
    }

    /// The URL to POST to.
    pub fn endpoint(&self) -> String {
        let settings = self.settings();
        match self {
            Provider::OpenAi(_) => {
                let base = settings
                    .host
                    .as_deref()
                    .unwrap_or("https://api.openai.com")
                    .trim_end_matches('/');
                format!("{base}/v1/chat/completions")
            }
            Provider::Anthropic(_) => {
                let base = settings
                    .host
                    .as_deref()
                    .unwrap_or("https://api.anthropic.com")
                    .trim_end_matches('/');
                format!("{base}/v1/messages")
            }
        }
    }

    /// Headers carrying the provider credential.
    ///
    /// The two providers spell this differently, and Anthropic additionally
    /// requires a version header on every request — omitting it is rejected,
    /// not defaulted.
    pub fn auth_headers(&self, key: Option<&str>) -> Vec<(&'static str, String)> {
        let mut headers = Vec::new();
        match self {
            Provider::OpenAi(_) => {
                if let Some(key) = key {
                    headers.push(("authorization", format!("Bearer {key}")));
                }
            }
            Provider::Anthropic(_) => {
                if let Some(key) = key {
                    headers.push(("x-api-key", key.to_string()));
                }
                headers.push(("anthropic-version", ANTHROPIC_VERSION.to_string()));
            }
        }
        headers
    }

    /// Convert an OpenAI request into this provider's shape.
    pub fn translate_request(&self, body: &Value) -> Result<Value, TranslateError> {
        match self {
            // Already the right shape; forwarding it whole is what keeps
            // fields this crate does not know about intact.
            Provider::OpenAi(_) => Ok(body.clone()),
            Provider::Anthropic(_) => translate::to_anthropic(body),
        }
    }

    /// Convert a provider response into OpenAI's shape.
    ///
    /// `None` means the response already is OpenAI-shaped and should be
    /// returned byte-for-byte rather than re-serialized.
    pub fn translate_response(&self, body: &Value, created: u64) -> Option<Value> {
        match self {
            Provider::OpenAi(_) => None,
            Provider::Anthropic(_) => Some(translate::from_anthropic(body, created)),
        }
    }

    /// Token usage from a provider response.
    pub fn usage(&self, body: &Value) -> Option<Usage> {
        match self {
            Provider::OpenAi(_) => translate::openai_usage(body),
            Provider::Anthropic(_) => translate::anthropic_usage(body),
        }
    }

    /// Whether a streamed response needs re-framing.
    pub fn translates_stream(&self) -> bool {
        matches!(self, Provider::Anthropic(_))
    }
}

/// The Messages API version this gateway speaks.
const ANTHROPIC_VERSION: &str = "2023-06-01";

#[cfg(test)]
mod tests {
    use super::*;
    use agentgateway_config::AiProviderParams;

    fn params(model: Option<&str>, host: Option<&str>) -> AiProviderParams {
        AiProviderParams {
            model: model.map(str::to_string),
            host_override: host.map(str::to_string),
        }
    }

    #[test]
    fn default_endpoints_are_the_real_ones() {
        let openai = Provider::new(&AiProvider::OpenAi(params(None, None)), "t").expect("ok");
        assert_eq!(
            openai.endpoint(),
            "https://api.openai.com/v1/chat/completions"
        );

        let anthropic = Provider::new(&AiProvider::Anthropic(params(None, None)), "t").expect("ok");
        assert_eq!(
            anthropic.endpoint(),
            "https://api.anthropic.com/v1/messages"
        );
    }

    #[test]
    fn a_host_override_points_at_a_compatible_endpoint() {
        // This is how a self-hosted or proxied deployment is reached, so a
        // trailing slash must not produce a double one.
        let provider = Provider::new(
            &AiProvider::OpenAi(params(None, Some("http://localhost:8000/"))),
            "t",
        )
        .expect("ok");
        assert_eq!(
            provider.endpoint(),
            "http://localhost:8000/v1/chat/completions"
        );
    }

    #[test]
    fn openai_uses_a_bearer_token_and_anthropic_a_key_header() {
        let openai = Provider::new(&AiProvider::OpenAi(params(None, None)), "t").expect("ok");
        assert_eq!(
            openai.auth_headers(Some("sk-1")),
            vec![("authorization", "Bearer sk-1".to_string())]
        );

        let anthropic = Provider::new(&AiProvider::Anthropic(params(None, None)), "t").expect("ok");
        let headers = anthropic.auth_headers(Some("sk-ant"));
        assert!(headers.contains(&("x-api-key", "sk-ant".to_string())));
        assert!(
            headers.iter().any(|(name, _)| *name == "anthropic-version"),
            "Anthropic rejects a request without a version header"
        );
    }

    #[test]
    fn the_version_header_is_sent_even_without_a_key() {
        // Otherwise a keyless misconfiguration produces a confusing "missing
        // version" error instead of the authentication one it really is.
        let anthropic = Provider::new(&AiProvider::Anthropic(params(None, None)), "t").expect("ok");
        let headers = anthropic.auth_headers(None);
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].0, "anthropic-version");
    }

    #[test]
    fn an_openai_request_passes_through_untouched() {
        // The point: fields this crate has never heard of must survive.
        let body = serde_json::json!({
            "model": "gpt-4o",
            "messages": [],
            "tools": [{"type": "function", "function": {"name": "f"}}],
            "response_format": {"type": "json_object"},
        });
        let provider = Provider::new(&AiProvider::OpenAi(params(None, None)), "t").expect("ok");
        assert_eq!(
            provider.translate_request(&body).expect("should translate"),
            body
        );
    }

    #[test]
    fn an_openai_response_is_not_reserialized() {
        let provider = Provider::new(&AiProvider::OpenAi(params(None, None)), "t").expect("ok");
        assert!(
            provider
                .translate_response(&serde_json::json!({"a": 1}), 0)
                .is_none(),
            "None means hand back the original bytes"
        );
        assert!(!provider.translates_stream());
    }

    #[test]
    fn unimplemented_providers_fail_at_startup() {
        for provider in [
            AiProvider::Gemini(params(None, None)),
            AiProvider::Vertex(params(None, None)),
            AiProvider::Bedrock(params(None, None)),
        ] {
            let err = Provider::new(&provider, "binds[0]")
                .expect_err("should not be served by this build");
            assert!(err.to_string().contains("binds[0]"), "got: {err}");
        }
    }

    #[test]
    fn a_forced_model_is_reported() {
        let provider = Provider::new(
            &AiProvider::Anthropic(params(Some("claude-sonnet-4"), None)),
            "t",
        )
        .expect("ok");
        assert_eq!(provider.forced_model(), Some("claude-sonnet-4"));
    }
}
