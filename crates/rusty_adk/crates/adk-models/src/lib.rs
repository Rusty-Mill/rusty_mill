//! Model abstraction and provider connectors for the Rust ADK.
//!
//! An agent is configured with a model *name*; a [`Model`] implementation turns
//! that into real requests. [`MockModel`] replays a scripted sequence, which is
//! what the test suite and the offline examples run against; the `gemini` and
//! `anthropic` features add live connectors.
//!
//! # Example
//!
//! ```
//! # tokio_test::block_on(async {
//! use adk_models::{LlmRequest, MockModel, Model};
//!
//! let model = MockModel::new().push_text("Paris");
//! let response = model.generate_content(LlmRequest::new("mock")).await.unwrap();
//!
//! assert_eq!(response.text_content(), "Paris");
//! assert_eq!(model.call_count(), 1);
//! # });
//! ```

#![deny(missing_docs)]
#![warn(clippy::all)]

pub mod mock;
pub mod model;
pub mod request;

#[cfg(feature = "anthropic")]
pub mod anthropic;
#[cfg(feature = "gemini")]
pub mod gemini;

pub use mock::MockModel;
pub use model::{aggregate_stream, Model, ModelRegistry, SharedModel};
pub use request::{GenerateContentConfig, LlmRequest, LlmResponse, UsageMetadata};

#[cfg(feature = "anthropic")]
pub use anthropic::AnthropicModel;
#[cfg(feature = "gemini")]
pub use gemini::GeminiModel;

/// Builds a registry wired to whichever live connectors are compiled in.
///
/// Models are matched by name prefix, so an agent configured with
/// `gemini-flash-latest` or `claude-opus-5` resolves without the caller
/// naming a connector. Returns an empty registry when no provider feature is
/// enabled, and skips any provider whose API key is absent.
pub fn default_registry() -> ModelRegistry {
    #[allow(unused_mut)]
    let mut registry = ModelRegistry::new();

    #[cfg(feature = "gemini")]
    if let Ok(model) = GeminiModel::from_env("gemini-flash-latest") {
        registry.register_prefix("gemini", std::sync::Arc::new(model) as SharedModel);
    }

    #[cfg(feature = "anthropic")]
    if let Ok(model) = AnthropicModel::from_env("claude-opus-5") {
        registry.register_prefix("claude", std::sync::Arc::new(model) as SharedModel);
    }

    registry
}
