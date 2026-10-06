//! Embedding provider trait for vector representations of text.

use async_trait::async_trait;

use crate::error::AiError;

/// Trait for text embedding providers.
///
/// Implementors convert text into dense vector representations suitable
/// for semantic similarity search and retrieval-augmented generation.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Generate embeddings for a batch of texts.
    ///
    /// Returns one vector per input text. All vectors share the same
    /// dimensionality as reported by [`EmbeddingProvider::dimension`].
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, AiError>;

    /// Return the dimensionality of the embedding vectors produced.
    fn dimension(&self) -> usize;

    /// The dimension this provider is *known* to return, or `None`.
    ///
    /// Unlike [`dimension`](Self::dimension), which some providers fill with
    /// a legacy constant, `Some(d)` is a verified promise: indexing rejects
    /// any reply vector whose length is not `d`, and an unchanged file may be
    /// skipped only when its stored dimension equals `d`. `None` (the
    /// default) means the dimension is not known, so every reply is checked
    /// for uniformity only and an unchanged file is always re-embedded.
    ///
    /// A provider returns `Some` only for a dimension it has verified, never
    /// one inferred from a model name. This does not detect a model change
    /// that keeps the same dimension.
    fn expected_dimension(&self) -> Option<usize> {
        None
    }
}
