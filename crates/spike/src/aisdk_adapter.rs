//! Bridging a live aisdk model to the harness — and the spike's empirical
//! verdict on *how*.
//!
//! ## Finding (verified against aisdk 0.5.2)
//!
//! The "kernel drives its own loop over the low-level `LanguageModel` trait"
//! design (PRD 01, Strategy B) is **blocked** as written: `LanguageModel::
//! generate_text` takes a `LanguageModelOptions`, whose `messages` field is
//! `Vec<TaggedMessage>` — and **`TaggedMessage` is `pub(crate)`** (so is the
//! `messages` field, `tools`, `current_step_id`, `stop_reason`). External code
//! cannot name `TaggedMessage`, so it cannot build the messages vector for a
//! multi-turn, tool-aware request. The derived builder does not help: its
//! `.messages(..)` setter still takes the private type.
//!
//! This is the single fact that decides the fork question. It does **not**
//! justify a fork — a *narrow upstream PR* fixes it cleanly:
//!   - make `TaggedMessage` (and a `From<Message>` path) public, **or**
//!   - add a public `LanguageModelOptions` constructor taking `Vec<Message>`.
//!
//! ## What works today without any upstream change (Strategy A)
//!
//! aisdk's *high-level* `LanguageModelRequest` accepts a public `Vec<Message>`
//! and a `ToolList`, and runs the multi-step loop itself. Policy can still be
//! enforced by wrapping each tool's (synchronous) `execute` closure so it calls
//! our async [`ToolDispatch`](crate::tool::ToolDispatch) via a runtime handle,
//! returning the rendered [`ToolOutcome`](crate::outcome::ToolOutcome) string.
//! The trade-off: the kernel no longer owns the loop, and aisdk re-stringifies
//! any `Err` from the closure (so we always return `Ok(outcome.render())` to
//! keep status rendering ours).
//!
//! The helpers below are the pieces a Strategy-A live adapter needs, written
//! against **public** aisdk types only, so they compile and are unit-checkable
//! now. Wiring them to a concrete provider (e.g. local ollama) is the next step.

use aisdk::core::messages::{AssistantMessage, Message};
use aisdk::core::language_model::LanguageModelResponseContentType;
use aisdk::core::tools::{Tool, ToolExecute, ToolList, ToolResultInfo};
use serde_json::Value;

use crate::kernel::ChatMessage;

/// Convert harness history into aisdk's public [`Message`] vector (the input the
/// high-level `LanguageModelRequest::messages` accepts).
pub fn to_messages(history: &[ChatMessage]) -> Vec<Message> {
    history
        .iter()
        .map(|m| match m {
            ChatMessage::User(s) => Message::User(s.clone().into()),
            ChatMessage::Assistant(s) => Message::Assistant(AssistantMessage::new(
                LanguageModelResponseContentType::Text(s.clone()),
                None,
            )),
            ChatMessage::ToolResult { name, content } => {
                let mut info = ToolResultInfo::new(name);
                info.output(Value::String(content.clone()));
                Message::Tool(info)
            }
        })
        .collect()
}

/// Build an aisdk [`ToolList`] that advertises `(name, schema)` to the model.
/// `execute` is left as the default no-op here; a Strategy-A adapter replaces it
/// with a closure that calls the policy-vetting dispatcher.
pub fn schema_only_tool_list(tools: &[(String, Value)]) -> ToolList {
    let tools = tools
        .iter()
        .map(|(name, schema)| Tool {
            name: name.clone(),
            description: String::new(),
            input_schema: serde_json::from_value(schema.clone()).unwrap_or_default(),
            execute: ToolExecute::default(),
        })
        .collect();
    ToolList::new(tools)
}
