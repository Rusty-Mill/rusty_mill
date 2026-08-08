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
         `openAI` (or any OpenAI-compatible endpoint via `hostOverride`), `anthropic` and \
         `gemini` are"
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
    /// Google's Gemini API.
    Gemini(Settings),
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
            AiProvider::Gemini(params) => Ok(Provider::Gemini(settings(params)?)),
            // Vertex and Bedrock speak Gemini-like and Anthropic-like shapes
            // respectively, but sign with cloud credentials rather than an API
            // key -- a different kind of work from a translation. Refusing at
            // startup beats accepting the config and failing every request.
            AiProvider::Vertex(_) => unsupported("vertex"),
            AiProvider::Bedrock(_) => unsupported("bedrock"),
        }
    }

    /// The provider's name, for logs.
    pub fn name(&self) -> &'static str {
        match self {
            Provider::OpenAi(_) => "openai",
            Provider::Anthropic(_) => "anthropic",
            Provider::Gemini(_) => "gemini",
        }
    }

    fn settings(&self) -> &Settings {
        match self {
            Provider::OpenAi(settings)
            | Provider::Anthropic(settings)
            | Provider::Gemini(settings) => settings,
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
            // The base only: Gemini names the model and the method in the path,
            // so the rest of the URL is a function of the request and is built
            // per call. See `request_url`.
            Provider::Gemini(_) => {
                let base = settings
                    .host
                    .as_deref()
                    .unwrap_or("https://generativelanguage.googleapis.com")
                    .trim_end_matches('/');
                format!("{base}/v1beta")
            }
        }
    }

    /// The URL one request goes to.
    ///
    /// For OpenAI and Anthropic this is the endpoint resolved at startup: the
    /// model is a body field and a stream is the same URL with `stream: true`,
    /// so nothing about the address depends on the request.
    ///
    /// Gemini names both in the path — `models/{model}:generateContent`, or
    /// `:streamGenerateContent?alt=sse` — so its URL cannot be known until the
    /// caller's model has been resolved. `model` has been through
    /// [`translate::gemini::model_path`] by then, which is what keeps a
    /// client-controlled string from choosing the method.
    pub fn request_url(&self, endpoint: &str, model_path: &str, streaming: bool) -> String {
        match self {
            Provider::OpenAi(_) | Provider::Anthropic(_) => endpoint.to_string(),
            Provider::Gemini(_) => {
                let method = match streaming {
                    true => "streamGenerateContent?alt=sse",
                    false => "generateContent",
                };
                format!("{endpoint}/{model_path}:{method}")
            }
        }
    }

    /// The moderation URL a `promptGuard` rule may borrow, when this provider
    /// has one to lend.
    ///
    /// Only OpenAI: the endpoint is OpenAI's, and a key issued for another
    /// provider is not a credential for it. `hostOverride` is carried along,
    /// because a borrowed key should not travel further than the host it was
    /// configured for. See [`crate::guard::moderation`].
    pub fn moderation_endpoint(&self) -> Option<String> {
        match self {
            Provider::OpenAi(settings) => {
                let base = settings
                    .host
                    .as_deref()
                    .unwrap_or("https://api.openai.com")
                    .trim_end_matches('/');
                Some(format!("{base}/v1/moderations"))
            }
            Provider::Anthropic(_) | Provider::Gemini(_) => None,
        }
    }

    /// Headers carrying the provider credential.
    ///
    /// Each provider spells this differently. Anthropic additionally requires
    /// a version header on every request — omitting it is rejected, not
    /// defaulted.
    ///
    /// Gemini also accepts its key as a `?key=` query parameter, and this uses
    /// the header instead: a credential in a URL is written to every access log
    /// between here and Google, and to this gateway's own if a request is ever
    /// traced.
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
            Provider::Gemini(_) => {
                if let Some(key) = key {
                    headers.push(("x-goog-api-key", key.to_string()));
                }
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
            Provider::Gemini(_) => translate::gemini::to_gemini(body),
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
            Provider::Gemini(_) => Some(translate::gemini::from_gemini(body, created)),
        }
    }

    /// Token usage from a provider response.
    pub fn usage(&self, body: &Value) -> Option<Usage> {
        match self {
            Provider::OpenAi(_) => translate::openai_usage(body),
            Provider::Anthropic(_) => translate::anthropic_usage(body),
            Provider::Gemini(_) => translate::gemini::usage(body),
        }
    }

    /// Whether this provider's URL names the model.
    ///
    /// True only for Gemini, and it is why the address is built per request
    /// rather than resolved once at startup.
    pub fn needs_model_in_path(&self) -> bool {
        matches!(self, Provider::Gemini(_))
    }

    /// Whether a streamed response needs re-framing.
    pub fn translates_stream(&self) -> bool {
        matches!(self, Provider::Anthropic(_))
    }

    /// Whether this provider takes cache breakpoints in the request.
    ///
    /// Anthropic does, as `cache_control` on a content block. OpenAI caches
    /// long prefixes by itself and takes no configuration for it, so marking
    /// one there would be a field nobody reads. Gemini caches through an API of
    /// its own — content is uploaded, given a handle and referenced by
    /// `cachedContent` — which is not a breakpoint in a request and cannot be
    /// driven from this policy.
    pub fn caches_explicitly(&self) -> bool {
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
        // Vertex and Bedrock sign with cloud credentials rather than an API
        // key, which is a different kind of work from a translation.
        for provider in [
            AiProvider::Vertex(params(None, None)),
            AiProvider::Bedrock(params(None, None)),
        ] {
            let err = Provider::new(&provider, "binds[0]")
                .expect_err("should not be served by this build");
            assert!(err.to_string().contains("binds[0]"), "got: {err}");
        }
    }

    #[test]
    fn gemini_builds_its_url_from_the_request() {
        // The model and the method are both in the path, so unlike the other
        // two providers the address is not knowable until the caller has asked
        // for something.
        let gemini = Provider::new(&AiProvider::Gemini(params(None, None)), "t").expect("ok");
        assert_eq!(
            gemini.endpoint(),
            "https://generativelanguage.googleapis.com/v1beta",
            "the base only"
        );
        assert!(gemini.needs_model_in_path());
        assert_eq!(
            gemini.request_url(&gemini.endpoint(), "models/gemini-2.5-flash", false),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent"
        );
        assert_eq!(
            gemini.request_url(&gemini.endpoint(), "models/gemini-2.5-flash", true),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse"
        );

        let hosted = Provider::new(
            &AiProvider::Gemini(params(None, Some("https://proxy.internal/"))),
            "t",
        )
        .expect("ok");
        assert_eq!(hosted.endpoint(), "https://proxy.internal/v1beta");
    }

    #[test]
    fn a_request_url_is_the_endpoint_itself_for_the_other_providers() {
        for provider in [
            AiProvider::OpenAi(params(None, None)),
            AiProvider::Anthropic(params(None, None)),
        ] {
            let provider = Provider::new(&provider, "t").expect("ok");
            assert!(!provider.needs_model_in_path());
            assert_eq!(
                provider.request_url("https://example.test/v1/x", "models/ignored", true),
                "https://example.test/v1/x",
                "the model is a body field and a stream is the same URL"
            );
        }
    }

    #[test]
    fn geminis_key_goes_in_a_header_rather_than_the_query_string() {
        // A credential in a URL is written to every access log between here
        // and Google.
        let gemini = Provider::new(&AiProvider::Gemini(params(None, None)), "t").expect("ok");
        assert_eq!(
            gemini.auth_headers(Some("k")),
            vec![("x-goog-api-key", "k".to_string())]
        );
        assert!(gemini.auth_headers(None).is_empty());
    }

    #[test]
    fn gemini_lends_no_moderation_endpoint() {
        // Its key is not an OpenAI credential; see `guard::moderation`.
        let gemini = Provider::new(&AiProvider::Gemini(params(None, None)), "t").expect("ok");
        assert!(gemini.moderation_endpoint().is_none());
        assert!(!gemini.caches_explicitly());
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
