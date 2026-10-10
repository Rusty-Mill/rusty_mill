//! `tools/list` and `tools/call`.

use crate::codec::{
    decode_all, encode_all, field, object, opt_array, opt_bool, opt_string, opt_value, string, Obj,
    Wire,
};
use crate::content::ContentBlock;
use crate::page::{decode_list, encode_list, Paging, ResultType};
use crate::Result;
use rusty_json::Value;

/// Method names for tools.
pub mod method {
    /// List the tools a server offers.
    pub const LIST: &str = "tools/list";
    /// Call a tool.
    pub const CALL: &str = "tools/call";
}

/// A tool a server offers.
#[derive(Clone, Debug, PartialEq)]
pub struct Tool {
    /// Programmatic name.
    pub name: String,
    /// Human-readable name.
    pub title: Option<String>,
    /// What it does.
    pub description: Option<String>,
    /// JSON Schema of the arguments, an object.
    pub input_schema: Value,
    /// JSON Schema of `structuredContent`.
    pub output_schema: Option<Value>,
    /// Behaviour hints, raw JSON.
    pub annotations: Option<Value>,
    /// Icons, raw JSON.
    pub icons: Option<Value>,
    /// `_meta`, raw JSON.
    pub meta: Option<Value>,
}

impl Tool {
    /// A tool with only the required members.
    pub fn new(name: impl Into<String>, input_schema: Value) -> Self {
        Self {
            name: name.into(),
            title: None,
            description: None,
            input_schema,
            output_schema: None,
            annotations: None,
            icons: None,
            meta: None,
        }
    }
}

impl Wire for Tool {
    fn to_value(&self) -> Value {
        Obj::new()
            .set("name", self.name.as_str())
            .opt("title", self.title.clone())
            .opt("description", self.description.clone())
            .set("inputSchema", self.input_schema.clone())
            .opt_value("outputSchema", &self.output_schema)
            .opt_value("annotations", &self.annotations)
            .opt_value("icons", &self.icons)
            .opt_value("_meta", &self.meta)
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "Tool";
        object(v, W)?;
        let input_schema = object(field(v, W, "inputSchema")?, "Tool inputSchema")?.clone();
        Ok(Self {
            name: string(v, W, "name")?,
            title: opt_string(v, W, "title")?,
            description: opt_string(v, W, "description")?,
            input_schema,
            output_schema: opt_value(v, "outputSchema"),
            annotations: opt_value(v, "annotations"),
            icons: opt_value(v, "icons"),
            meta: opt_value(v, "_meta"),
        })
    }
}

/// A page of `tools/list`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ListToolsResult {
    /// The tools on this page.
    pub tools: Vec<Tool>,
    /// Cursor, cache hints and `_meta`.
    pub paging: Paging,
}

impl Wire for ListToolsResult {
    fn to_value(&self) -> Value {
        encode_list("tools", &self.tools, &self.paging)
    }

    fn from_value(v: &Value) -> Result<Self> {
        let (tools, paging) = decode_list(v, "ListToolsResult", "tools")?;
        Ok(Self { tools, paging })
    }
}

/// Parameters of `tools/call`.
#[derive(Clone, Debug, PartialEq)]
pub struct CallToolParams {
    /// Which tool.
    pub name: String,
    /// The arguments, a JSON object.
    pub arguments: Option<Value>,
    /// 2026-07-28: answers to an earlier `input_required` result.
    pub input_responses: Option<Value>,
    /// 2026-07-28: the opaque state the server asked to have echoed.
    pub request_state: Option<String>,
    /// Request `_meta`, raw JSON.
    pub meta: Option<Value>,
}

impl CallToolParams {
    /// A call with no arguments.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            arguments: None,
            input_responses: None,
            request_state: None,
            meta: None,
        }
    }
}

impl Wire for CallToolParams {
    fn to_value(&self) -> Value {
        Obj::new()
            .set("name", self.name.as_str())
            .opt_value("arguments", &self.arguments)
            .opt_value("inputResponses", &self.input_responses)
            .opt("requestState", self.request_state.clone())
            .opt_value("_meta", &self.meta)
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "CallToolParams";
        object(v, W)?;
        let arguments = opt_value(v, "arguments");
        if let Some(a) = &arguments {
            object(a, "CallToolParams arguments")?;
        }
        Ok(Self {
            name: string(v, W, "name")?,
            arguments,
            input_responses: opt_value(v, "inputResponses"),
            request_state: opt_string(v, W, "requestState")?,
            meta: opt_value(v, "_meta"),
        })
    }
}

/// The result of a finished `tools/call`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CallToolResult {
    /// `resultType`.
    pub result_type: Option<ResultType>,
    /// What the tool produced.
    pub content: Vec<ContentBlock>,
    /// The same as JSON matching the tool's `outputSchema`.
    pub structured_content: Option<Value>,
    /// Whether the tool failed (a tool failure is a result, not an RPC error).
    pub is_error: Option<bool>,
    /// Result `_meta`, raw JSON.
    pub meta: Option<Value>,
}

impl Wire for CallToolResult {
    fn to_value(&self) -> Value {
        Obj::new()
            .opt("resultType", self.result_type.as_ref().map(|t| t.0.clone()))
            .set("content", encode_all(&self.content))
            .opt_value("structuredContent", &self.structured_content)
            .opt("isError", self.is_error)
            .opt_value("_meta", &self.meta)
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "CallToolResult";
        object(v, W)?;
        Ok(Self {
            result_type: opt_string(v, W, "resultType")?.map(ResultType),
            content: decode_all(opt_array(v, W, "content")?)?,
            structured_content: opt_value(v, "structuredContent"),
            is_error: opt_bool(v, W, "isError")?,
            meta: opt_value(v, "_meta"),
        })
    }
}
