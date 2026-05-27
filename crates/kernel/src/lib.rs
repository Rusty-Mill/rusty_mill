#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
//! `kernel` — the aisdk `LanguageModelRequest` loop, dispatching through the
//! abstract `constrain::ToolDispatch` seam (ARCHITECTURE §4-5, §54).
//!
//! ## Strategy A (spike 01)
//!
//! aisdk's high-level loop owns multi-step tool calling, but `handle_tool_call`
//! is `pub(crate)` — there is no interception point, and the low-level options
//! type is not externally constructible (`pub(crate)` `TaggedMessage`). So we
//! enforce policy from *inside* each tool's (synchronous) `execute` closure: it
//! bridges to our async [`ToolDispatch`] via the current runtime handle and
//! always returns `Ok(outcome.render())`, keeping the structural
//! [`rk_observe::ToolOutcome`] ours rather than aisdk's re-stringified error.

#[cfg(feature = "fake")]
pub mod fake;

use std::sync::Arc;

use aisdk::core::capabilities::{TextInputSupport, ToolCallSupport};
use aisdk::core::language_model::LanguageModel;
use aisdk::core::tools::{Tool, ToolExecute};
use aisdk::core::LanguageModelRequest;
use rk_constrain::ToolDispatch;

/// Kernel errors (ADR-0023).
#[derive(Debug, thiserror::Error)]
pub enum KernelError {
    /// The underlying model/provider failed.
    #[error("model error: {0}")]
    Model(String),
}

/// Build an aisdk [`Tool`] that advertises `(name, schema)` and, when invoked by
/// aisdk's loop, bridges to our policy-vetted async dispatcher.
fn bridge_tool(name: String, schema: serde_json::Value, dispatch: Arc<dyn ToolDispatch>) -> Tool {
    let exec_name = name.clone();
    Tool {
        name,
        description: String::new(),
        input_schema: serde_json::from_value(schema).unwrap_or_default(),
        execute: ToolExecute::new(Box::new(move |args| {
            // aisdk runs this closure inside a spawned task on the multi-thread
            // runtime, so block_in_place + the current handle is sound.
            let dispatch = dispatch.clone();
            let name = exec_name.clone();
            let outcome = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current()
                    .block_on(async move { dispatch.dispatch(&name, args).await })
            });
            // Always Ok: status is carried structurally in our ToolOutcome; we
            // hand aisdk the rendered string so it is not re-prefixed as "Error".
            Ok(outcome.render())
        })),
    }
}

/// Run one user turn to completion. aisdk's loop drives multi-step tool calling;
/// each tool call is vetted + executed via `dispatch`. Returns the final text.
pub async fn run_turn<M>(
    model: M,
    system: &str,
    user_prompt: &str,
    dispatch: Arc<dyn ToolDispatch>,
) -> Result<String, KernelError>
where
    M: LanguageModel + TextInputSupport + ToolCallSupport,
{
    let mut builder = LanguageModelRequest::builder()
        .model(model)
        .system(system.to_string())
        .prompt(user_prompt.to_string());

    for (name, schema) in dispatch.schemas() {
        builder = builder.with_tool(bridge_tool(name, schema, dispatch.clone()));
    }

    let mut request = builder.build();
    let response = request
        .generate_text()
        .await
        .map_err(|e| KernelError::Model(e.to_string()))?;

    Ok(response.text().unwrap_or_default())
}
