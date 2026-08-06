//! Tools for the Rust ADK.
//!
//! A tool is a unit of capability an agent can invoke: a function with a
//! declared schema, a description the model reads to decide when to call it,
//! and a JSON result. This crate defines the [`Tool`] trait, the
//! [`ToolContext`] a tool runs against, and the [`Toolset`] abstraction for
//! tools discovered at run time.
//!
//! # Conventions this crate enforces
//!
//! - **Object results.** A tool's return value is normalized to a JSON object;
//!   scalars are wrapped under a `result` key. Include a `status` field so the
//!   model can distinguish success from failure.
//! - **Argument validation.** Arguments are checked against the declared
//!   schema before the tool body runs, turning a malformed model call into a
//!   clear error instead of a panic inside the tool.
//! - **Confirmation gating.** A tool that requires approval never executes its
//!   body until the user has answered.
//!
//! All three are applied by [`invoke_tool`], which is what agents call.
//!
//! # Example
//!
//! ```
//! use adk_core::Schema;
//! use adk_tools::{FunctionTool, Tool};
//! use serde_json::json;
//!
//! let tool = FunctionTool::new(
//!     "reimburse",
//!     "Reimburses an amount to the user.",
//!     Schema::object().property("amount", Schema::integer().describe("Amount in cents.")),
//!     |args, _ctx| {
//!         Box::pin(async move {
//!             let amount = args.get("amount").and_then(|v| v.as_i64()).unwrap_or(0);
//!             Ok(json!({ "status": "success", "reimbursed": amount }))
//!         })
//!     },
//! )
//! .require_confirmation_when(|args| {
//!     let amount = args.get("amount").and_then(|v| v.as_i64()).unwrap_or(0);
//!     (amount > 1000).then(|| format!("Approve a reimbursement of {amount}?"))
//! });
//!
//! assert_eq!(tool.name(), "reimburse");
//! assert!(tool.declaration().unwrap().parameters.is_some());
//! ```

#![deny(missing_docs)]
#![warn(clippy::all)]

pub mod context;
pub mod function;
pub mod tool;
pub mod toolset;

/// Re-exports the `#[adk_tool]` macro's generated code depends on.
///
/// Routing every generated path through one module means a crate using the
/// macro needs exactly one dependency in scope — this one, or whatever path is
/// named with `#[adk_tool(crate = ...)]`.
#[doc(hidden)]
pub mod __macro_support {
    pub use adk_core::{AdkError, Args, FunctionDeclaration, HasSchema, Result, Schema};
    pub use async_trait::async_trait;
    pub use serde_json;
    pub use std::sync::Arc;

    pub use crate::{Tool, ToolContext};
}

pub use context::ToolContext;
pub use function::{FunctionTool, ToolCallable, ToolFn, ToolFuture};
pub use tool::{invoke_tool, ConfirmationPolicy, ConfirmationPredicate, SharedTool, Tool};
pub use toolset::{resolve_tools, StaticToolset, ToolSource, Toolset};

/// Builds a successful tool result with a `status` field, as ADK recommends.
///
/// ```
/// # use serde_json::json;
/// assert_eq!(
///     adk_tools::success(json!({"temp": 20})),
///     json!({"status": "success", "temp": 20})
/// );
/// ```
pub fn success(value: serde_json::Value) -> serde_json::Value {
    merge_status(value, "success")
}

/// Builds a failed tool result the model can read and recover from.
///
/// Returning this is usually better than returning `Err`: the model sees the
/// message and can retry or explain, whereas an `Err` surfaces as a framework
/// error and ends the tool call.
pub fn error(message: impl Into<String>) -> serde_json::Value {
    serde_json::json!({
        "status": "error",
        "error_message": message.into(),
    })
}

/// Builds a pending result, for a long-running tool that has just started.
pub fn pending(value: serde_json::Value) -> serde_json::Value {
    merge_status(value, "pending")
}

fn merge_status(value: serde_json::Value, status: &str) -> serde_json::Value {
    let mut object = match adk_core::wrap_tool_result(value) {
        serde_json::Value::Object(map) => map,
        // `wrap_tool_result` always yields an object.
        _ => unreachable!("wrap_tool_result returns an object"),
    };
    object
        .entry("status".to_string())
        .or_insert_with(|| serde_json::Value::String(status.to_string()));
    serde_json::Value::Object(object)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn success_adds_a_status_field() {
        assert_eq!(success(json!({"temp": 20}))["status"], "success");
    }

    #[test]
    fn success_wraps_scalars_before_adding_status() {
        assert_eq!(
            success(json!(42)),
            json!({"status": "success", "result": 42})
        );
    }

    #[test]
    fn an_explicit_status_is_not_overwritten() {
        assert_eq!(success(json!({"status": "partial"}))["status"], "partial");
    }

    #[test]
    fn error_carries_the_message() {
        let e = error("city not found");
        assert_eq!(e["status"], "error");
        assert_eq!(e["error_message"], "city not found");
    }
}
