use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use skillopt_core::{ChatBackend, Message};

/// Deterministic, network-free backend. Cycles through a fixed list of
/// responses (repeating the last one once exhausted), or falls back to
/// echoing the final user message if no responses were configured. Useful
/// for `--dry-run`, examples, and integration tests that shouldn't require
/// an API key.
pub struct MockBackend {
    name: String,
    responses: Vec<String>,
    calls: AtomicUsize,
}

impl MockBackend {
    pub fn new(name: impl Into<String>, responses: Vec<String>) -> Self {
        Self {
            name: name.into(),
            responses,
            calls: AtomicUsize::new(0),
        }
    }

    /// A mock that just echoes the last user/assistant message content back,
    /// useful as a no-op executor stand-in.
    pub fn echo(name: impl Into<String>) -> Self {
        Self::new(name, Vec::new())
    }
}

#[async_trait]
impl ChatBackend for MockBackend {
    fn name(&self) -> &str {
        &self.name
    }

    async fn chat(&self, messages: &[Message]) -> anyhow::Result<String> {
        let call_idx = self.calls.fetch_add(1, Ordering::SeqCst);
        if self.responses.is_empty() {
            let last = messages
                .last()
                .map(|m| m.content.clone())
                .unwrap_or_default();
            return Ok(last);
        }
        let idx = call_idx.min(self.responses.len() - 1);
        Ok(self.responses[idx].clone())
    }
}
