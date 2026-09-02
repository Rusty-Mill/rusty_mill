//! The live aisdk embedding adapter (Phase 5). Wraps an aisdk `EmbeddingModel`
//! as a `feed::Embedder`, so semantic recall works against any
//! OpenAI-compatible embedding endpoint. This is the single place an aisdk
//! embedding type is named; `feed` stays model-agnostic behind the trait.

use aisdk::core::embedding_model::EmbeddingModel;
use aisdk::core::EmbeddingModelRequest;
use async_trait::async_trait;
use rk_feed::{Embedder, ToolError};

/// Adapts an aisdk [`EmbeddingModel`] to the [`Embedder`] seam.
pub struct AiSdkEmbedder<M: EmbeddingModel> {
    model: M,
}

impl<M: EmbeddingModel> AiSdkEmbedder<M> {
    /// Wrap `model`.
    pub fn new(model: M) -> Self {
        Self { model }
    }
}

#[async_trait]
impl<M: EmbeddingModel> Embedder for AiSdkEmbedder<M> {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, ToolError> {
        let request = EmbeddingModelRequest::builder()
            .model(self.model.clone())
            .input(vec![text.to_string()])
            .build();
        let vectors = request
            .embed()
            .await
            .map_err(|e| ToolError::Other(e.to_string()))?;
        vectors
            .into_iter()
            .next()
            .ok_or_else(|| ToolError::Other("empty embedding response".to_string()))
    }
}
