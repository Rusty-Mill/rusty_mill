//! The [`Model`] trait and the registry that resolves a model by name.

use adk_core::{AdkError, Result};
use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::request::{LlmRequest, LlmResponse};

/// A language model an agent can call.
#[async_trait]
pub trait Model: Send + Sync {
    /// The provider model identifier this instance targets.
    fn name(&self) -> &str;

    /// Generates a single complete response.
    async fn generate_content(&self, request: LlmRequest) -> Result<LlmResponse>;

    /// Generates a response as a stream of chunks.
    ///
    /// The default adapts [`Model::generate_content`] into a one-element
    /// stream, so a provider without streaming support still satisfies a
    /// streaming run — it simply delivers everything at the end.
    ///
    /// Implementations that do stream must emit partial chunks with
    /// [`LlmResponse::partial`] set, then a final aggregated response with
    /// [`LlmResponse::turn_complete`] set. The agent relies on that final
    /// event to know what to commit.
    fn generate_content_stream<'a>(
        &'a self,
        request: LlmRequest,
    ) -> BoxStream<'a, Result<LlmResponse>> {
        Box::pin(stream::once(
            async move { self.generate_content(request).await },
        ))
    }

    /// Whether this model supports streaming natively.
    fn supports_streaming(&self) -> bool {
        false
    }
}

/// A model behind shared ownership, as agents hold them.
pub type SharedModel = Arc<dyn Model>;

/// Resolves model identifiers to connectors.
///
/// Lets an agent be configured with a model *name* — the way every ADK SDK
/// configures one — while the concrete connector is chosen at wiring time.
#[derive(Default)]
pub struct ModelRegistry {
    exact: BTreeMap<String, SharedModel>,
    prefixes: Vec<(String, SharedModel)>,
}

impl ModelRegistry {
    /// Builds an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a connector under an exact model name.
    pub fn register(&mut self, name: impl Into<String>, model: SharedModel) -> &mut Self {
        self.exact.insert(name.into(), model);
        self
    }

    /// Registers a connector for every model name starting with `prefix`.
    ///
    /// Longer prefixes are matched first, so `gemini-2.5` wins over `gemini-`.
    pub fn register_prefix(&mut self, prefix: impl Into<String>, model: SharedModel) -> &mut Self {
        self.prefixes.push((prefix.into(), model));
        self.prefixes
            .sort_by_key(|(p, _)| std::cmp::Reverse(p.len()));
        self
    }

    /// Resolves a model name, preferring an exact match.
    pub fn resolve(&self, name: &str) -> Result<SharedModel> {
        if let Some(model) = self.exact.get(name) {
            return Ok(Arc::clone(model));
        }
        for (prefix, model) in &self.prefixes {
            if name.starts_with(prefix.as_str()) {
                return Ok(Arc::clone(model));
            }
        }
        Err(AdkError::Config(format!(
            "no model registered for '{name}'"
        )))
    }
}

/// Collapses a stream of chunks into the single response they describe.
///
/// Concatenates text across chunks and keeps the last non-text parts, usage,
/// and finish reason. Useful for treating a streaming model uniformly with a
/// non-streaming one.
pub async fn aggregate_stream(
    mut stream: BoxStream<'_, Result<LlmResponse>>,
) -> Result<LlmResponse> {
    use adk_core::{Content, Part, Role};

    let mut text = String::new();
    let mut other_parts: Vec<Part> = Vec::new();
    let mut final_response = LlmResponse {
        turn_complete: true,
        ..Default::default()
    };

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if chunk.is_error() {
            return Ok(chunk);
        }
        if let Some(content) = &chunk.content {
            for part in &content.parts {
                match part {
                    Part::Text(t) => text.push_str(t),
                    other => other_parts.push(other.clone()),
                }
            }
        }
        if chunk.finish_reason.is_some() {
            final_response.finish_reason = chunk.finish_reason.clone();
        }
        if chunk.usage.is_some() {
            final_response.usage = chunk.usage.clone();
        }
    }

    let mut parts = Vec::new();
    if !text.is_empty() {
        parts.push(Part::Text(text));
    }
    parts.extend(other_parts);
    if !parts.is_empty() {
        final_response.content = Some(Content::new(Role::Model, parts));
    }
    Ok(final_response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockModel;

    fn registry() -> ModelRegistry {
        let mut r = ModelRegistry::new();
        r.register_prefix(
            "gemini-",
            Arc::new(MockModel::echo("gemini")) as SharedModel,
        );
        r.register_prefix(
            "gemini-2.5",
            Arc::new(MockModel::echo("gemini-2.5")) as SharedModel,
        );
        r.register(
            "custom-model",
            Arc::new(MockModel::echo("exact")) as SharedModel,
        );
        r
    }

    #[test]
    fn exact_names_win() {
        assert_eq!(registry().resolve("custom-model").unwrap().name(), "exact");
    }

    #[test]
    fn longer_prefixes_win() {
        let r = registry();
        assert_eq!(r.resolve("gemini-2.5-pro").unwrap().name(), "gemini-2.5");
        assert_eq!(r.resolve("gemini-flash-latest").unwrap().name(), "gemini");
    }

    #[test]
    fn unknown_models_are_a_config_error() {
        assert!(registry().resolve("gpt-9").is_err());
    }

    #[tokio::test]
    async fn aggregation_concatenates_chunk_text() {
        let model = MockModel::new().push_stream(["The ", "capital ", "is Paris."]);
        let stream = model.generate_content_stream(LlmRequest::new("mock"));
        let aggregated = aggregate_stream(stream).await.unwrap();
        assert_eq!(aggregated.text_content(), "The capital is Paris.");
        assert!(aggregated.turn_complete);
    }
}
