//! A JSON-object wrapper for passthrough API responses.
//!
//! MCP requires a tool's structured output -- and the `outputSchema` it's
//! validated against -- to be a JSON object at the top level. A bare
//! `serde_json::Value` doesn't satisfy that: Proxmox/OPNsense endpoints like
//! `list_nodes`/`list_services` return a JSON *array*, and `schemars`'
//! generated schema for an unconstrained `Value` isn't `type: object` either
//! way. Every passthrough tool wraps its value in this single-field struct
//! instead of returning `Json<serde_json::Value>` directly, so both the
//! schema and the structured content itself stay a valid MCP object.

use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;

/// Wraps an arbitrary JSON value as a tool's structured content.
#[derive(Debug, Serialize, JsonSchema)]
pub struct JsonResult {
    /// The upstream API's response, passed through as-is.
    pub result: Value,
}

impl From<Value> for JsonResult {
    fn from(result: Value) -> Self {
        Self { result }
    }
}
