//! `FakeLanguageModel` — a scripted aisdk [`LanguageModel`] for offline,
//! deterministic tests (testing-strategy.md). It implements the *real* aisdk
//! trait, so [`crate::run_turn`] exercises aisdk's genuine tool-calling loop
//! (and our policy-vetting bridge) with no live provider.

use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use aisdk::Result as AiResult;
use aisdk::core::capabilities::{TextInputSupport, ToolCallSupport};
use aisdk::core::language_model::{
    LanguageModel, LanguageModelOptions, LanguageModelResponse, LanguageModelResponseContentType,
    LanguageModelStreamChunk,
};
use aisdk::core::tools::ToolCallInfo;
use async_trait::async_trait;
use futures::Stream;
use serde_json::Value;

/// One unit of scripted model output.
#[derive(Debug, Clone)]
pub enum Scripted {
    /// Emit assistant text (ends the turn if it is the last content).
    Text(String),
    /// Request a tool call with `name`/`args`.
    ToolCall {
        /// Tool name.
        name: String,
        /// JSON arguments.
        args: Value,
    },
}

/// A model whose per-call responses are a fixed script.
#[derive(Debug, Clone)]
pub struct FakeLanguageModel {
    script: Arc<Mutex<VecDeque<Vec<Scripted>>>>,
}

impl FakeLanguageModel {
    /// Build from an ordered list of per-call response batches.
    pub fn new(turns: Vec<Vec<Scripted>>) -> Self {
        Self { script: Arc::new(Mutex::new(turns.into_iter().collect())) }
    }
}

impl TextInputSupport for FakeLanguageModel {}
impl ToolCallSupport for FakeLanguageModel {}

#[async_trait]
impl LanguageModel for FakeLanguageModel {
    fn name(&self) -> String {
        "fake".to_string()
    }

    async fn generate_text(
        &mut self,
        _options: LanguageModelOptions,
    ) -> AiResult<LanguageModelResponse> {
        let batch = self
            .script
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .pop_front()
            .unwrap_or_default();

        let contents = batch
            .into_iter()
            .map(|s| match s {
                Scripted::Text(t) => LanguageModelResponseContentType::Text(t),
                Scripted::ToolCall { name, args } => {
                    let mut info = ToolCallInfo::new(name);
                    info.input(args);
                    LanguageModelResponseContentType::ToolCall(info)
                }
            })
            .collect();

        Ok(LanguageModelResponse { contents, usage: None })
    }

    async fn stream_text(
        &mut self,
        _options: LanguageModelOptions,
    ) -> AiResult<Pin<Box<dyn Stream<Item = AiResult<Vec<LanguageModelStreamChunk>>> + Send>>> {
        Ok(Box::pin(futures::stream::empty()))
    }
}
