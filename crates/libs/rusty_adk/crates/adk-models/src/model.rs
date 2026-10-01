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

/// Folds streamed chunks into the single response they describe.
///
/// The one reducer for streamed model output, used by [`aggregate_stream`]
/// and by agents that forward each chunk as it arrives: text is
/// concatenated across chunks, other parts are kept in order after it, and
/// the latest finish reason and usage win. An error chunk ends the stream:
/// [`StreamAggregator::failed`] turns true and [`StreamAggregator::finish`]
/// returns that error rather than a partial answer.
#[derive(Debug, Default)]
pub struct StreamAggregator {
    text: String,
    other_parts: Vec<adk_core::Part>,
    finish_reason: Option<String>,
    usage: Option<crate::request::UsageMetadata>,
    error: Option<LlmResponse>,
}

impl StreamAggregator {
    /// An empty aggregator.
    pub fn new() -> Self {
        Self::default()
    }

    /// Folds in one chunk. Chunks after an error are ignored.
    pub fn push(&mut self, chunk: &LlmResponse) {
        use adk_core::Part;

        if self.error.is_some() {
            return;
        }
        if chunk.is_error() {
            self.error = Some(chunk.clone());
            return;
        }
        if let Some(content) = &chunk.content {
            for part in &content.parts {
                match part {
                    Part::Text(t) => self.text.push_str(t),
                    other => self.other_parts.push(other.clone()),
                }
            }
        }
        if chunk.finish_reason.is_some() {
            self.finish_reason = chunk.finish_reason.clone();
        }
        if chunk.usage.is_some() {
            self.usage = chunk.usage.clone();
        }
    }

    /// True once an error chunk has been pushed; the caller should stop
    /// reading the stream.
    pub fn failed(&self) -> bool {
        self.error.is_some()
    }

    /// The aggregated, turn-complete response, or the error chunk if one
    /// arrived.
    pub fn finish(self) -> LlmResponse {
        use adk_core::{Content, Part, Role};

        if let Some(error) = self.error {
            return error;
        }
        let mut parts = Vec::new();
        if !self.text.is_empty() {
            parts.push(Part::Text(self.text));
        }
        parts.extend(self.other_parts);
        LlmResponse {
            content: (!parts.is_empty()).then(|| Content::new(Role::Model, parts)),
            finish_reason: self.finish_reason,
            usage: self.usage,
            turn_complete: true,
            ..Default::default()
        }
    }
}

/// Collapses a stream of chunks into the single response they describe.
///
/// See [`StreamAggregator`] for how chunks combine. Useful for treating a
/// streaming model uniformly with a non-streaming one.
pub async fn aggregate_stream(
    mut stream: BoxStream<'_, Result<LlmResponse>>,
) -> Result<LlmResponse> {
    let mut aggregator = StreamAggregator::new();
    while let Some(chunk) = stream.next().await {
        aggregator.push(&chunk?);
        if aggregator.failed() {
            break;
        }
    }
    Ok(aggregator.finish())
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

    fn usage(candidates_tokens: u32) -> crate::request::UsageMetadata {
        crate::request::UsageMetadata {
            candidates_tokens,
            ..Default::default()
        }
    }

    #[test]
    fn text_comes_first_then_other_parts_in_order_and_the_latest_metadata_wins() {
        use adk_core::{Content, FunctionCall, Part, Role};

        let call = |name: &str| {
            LlmResponse::from_content(Content::new(
                Role::Model,
                vec![Part::FunctionCall(FunctionCall::new(
                    name,
                    Default::default(),
                ))],
            ))
        };
        let mut first = LlmResponse::chunk("Hel");
        first.usage = Some(usage(1));
        let mut last = LlmResponse::chunk("lo");
        last.finish_reason = Some("STOP".into());
        last.usage = Some(usage(7));

        let mut aggregator = StreamAggregator::new();
        for chunk in [first, call("a"), call("b"), last] {
            aggregator.push(&chunk);
        }
        let response = aggregator.finish();

        let parts = &response.content.as_ref().unwrap().parts;
        assert!(matches!(&parts[0], Part::Text(t) if t == "Hello"));
        assert!(matches!(&parts[1], Part::FunctionCall(c) if c.name == "a"));
        assert!(matches!(&parts[2], Part::FunctionCall(c) if c.name == "b"));
        assert_eq!(response.finish_reason.as_deref(), Some("STOP"));
        assert_eq!(response.usage.map(|u| u.candidates_tokens), Some(7));
        assert!(response.turn_complete);
    }

    #[test]
    fn an_error_chunk_ends_the_stream_and_is_the_result() {
        let mut aggregator = StreamAggregator::new();
        aggregator.push(&LlmResponse::chunk("partial answer"));
        assert!(!aggregator.failed());
        aggregator.push(&LlmResponse::error("RESOURCE_EXHAUSTED", "quota"));
        assert!(aggregator.failed());
        aggregator.push(&LlmResponse::chunk(" ignored"));

        let response = aggregator.finish();
        assert!(response.is_error());
        assert_eq!(response.error_code.as_deref(), Some("RESOURCE_EXHAUSTED"));
    }

    #[test]
    fn an_empty_stream_is_a_complete_response_without_content() {
        let response = StreamAggregator::new().finish();
        assert!(response.content.is_none());
        assert!(response.turn_complete);
    }
}
