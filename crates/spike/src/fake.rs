//! Scripted `ChatModel` for offline, deterministic tests (testing-strategy.md).
//!
//! Each call to [`ChatModel::respond`] pops the next scripted batch of steps, so
//! a test can drive a full multi-step tool loop with no live provider.

use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::Value;

use crate::error::ModelError;
use crate::kernel::{ChatMessage, ChatModel, ModelStep};

/// A model whose responses are a fixed script.
pub struct FakeChatModel {
    script: Mutex<std::collections::VecDeque<Vec<ModelStep>>>,
}

impl FakeChatModel {
    /// Build from an ordered list of per-call step batches.
    pub fn new(turns: Vec<Vec<ModelStep>>) -> Self {
        Self { script: Mutex::new(turns.into_iter().collect()) }
    }
}

#[async_trait]
impl ChatModel for FakeChatModel {
    async fn respond(
        &self,
        _system: &str,
        _history: &[ChatMessage],
        _tools: &[(String, Value)],
    ) -> Result<Vec<ModelStep>, ModelError> {
        let mut script = self.script.lock().expect("script mutex poisoned");
        script
            .pop_front()
            .ok_or_else(|| ModelError::Provider("fake model script exhausted".into()))
    }
}
