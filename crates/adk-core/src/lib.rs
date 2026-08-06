//! Core data model for a Rust implementation of the
//! [Agent Development Kit (ADK) 2.0](https://adk.dev/2.0/) architecture.
//!
//! This crate holds the types every other `adk-*` crate speaks in — the
//! [`Event`] that carries everything, the [`State`] it mutates, the
//! [`Session`] it accumulates into, and the [`InvocationContext`] that ties a
//! run together. It has no runtime of its own.
//!
//! # ADK 2.0
//!
//! Version 2.0 moved ADK from a hierarchical agent executor to a graph
//! execution engine, and added [`Event::node_info`] and [`Event::output`] to
//! track graph state and workflow outputs. Both are modelled here; see
//! [`NodeInfo`] for a note on how faithfully the internal layout could be
//! reproduced from the published documentation.
//!
//! # Example
//!
//! ```
//! use adk_core::{Content, Event, State};
//!
//! // State writes are staged, then ride out on an event.
//! let mut state = State::new();
//! state.set("temp:scratch", 1);
//! state.set("user:login_count", 5);
//!
//! let mut event = Event::new("inv-1", "my_agent").with_text("Hello");
//! event.actions.state_delta = state.take_delta();
//!
//! assert!(event.is_final_response());
//! assert_eq!(event.actions.state_delta.len(), 2);
//! ```

#![deny(missing_docs)]
#![warn(clippy::all)]

pub mod content;
pub mod context;
pub mod error;
pub mod event;
pub mod schema;
pub mod services;
pub mod session;
pub mod state;

pub use content::{Blob, Content, FileData, FunctionCall, FunctionResponse, Part, Role};
pub use context::{InvocationContext, RunConfig, StreamingMode};
pub use error::{AdkError, Result};
pub use event::{Args, Event, EventActions, NodeInfo, RequestInput, ToolConfirmation};
pub use schema::{FunctionDeclaration, Schema, SchemaType};
pub use services::{
    ArtifactService, ArtifactVersion, MemoryEntry, MemoryService, Services, SessionService,
};
pub use session::Session;
pub use state::{State, StateScope, APP_PREFIX, TEMP_PREFIX, USER_PREFIX};

use serde_json::{Map, Value};

/// The reserved function name ADK uses to carry a tool-confirmation answer
/// back from the client.
pub const TOOL_CONFIRMATION_FUNCTION_NAME: &str = "adk_request_confirmation";

/// Generates a prefixed unique id, e.g. `evt-9f2c…`.
pub fn new_id(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4())
}

/// The current time in seconds since the Unix epoch.
///
/// Clamps to 0 if the system clock is before the epoch rather than panicking —
/// a nonsensical timestamp on an event is preferable to killing a run.
pub fn now_seconds() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Normalizes a tool return value to ADK's object convention.
///
/// ADK expects tools to return a map. A non-object return is wrapped under a
/// single `result` key, matching what the Python and TypeScript SDKs do, so
/// the model always receives a consistently shaped function response.
pub fn wrap_tool_result(value: Value) -> Value {
    match value {
        Value::Object(_) => value,
        other => {
            let mut map = Map::new();
            map.insert("result".to_string(), other);
            Value::Object(map)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn objects_pass_through_unwrapped() {
        let v = json!({"status": "success", "temp": 20});
        assert_eq!(wrap_tool_result(v.clone()), v);
    }

    #[test]
    fn scalars_are_wrapped_under_result() {
        assert_eq!(wrap_tool_result(json!(42)), json!({"result": 42}));
        assert_eq!(wrap_tool_result(json!("hi")), json!({"result": "hi"}));
        assert_eq!(wrap_tool_result(json!([1, 2])), json!({"result": [1, 2]}));
        assert_eq!(wrap_tool_result(Value::Null), json!({"result": null}));
    }

    #[test]
    fn ids_are_prefixed_and_unique() {
        let a = new_id("evt");
        let b = new_id("evt");
        assert!(a.starts_with("evt-"));
        assert_ne!(a, b);
    }
}
