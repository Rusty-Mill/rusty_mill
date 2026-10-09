//! The core MCP types: `initialize` and the tools methods.
//!
//! Encoding emits only the members the reference encoders emit; decoding
//! ignores members it does not know, so a newer peer still parses.

use crate::util::{expect_object, field, opt_array, opt_bool, opt_string, opt_value, string, Obj};
use crate::{Error, Result};
use rusty_json::Value;

/// A spec revision string, for example `2025-06-18`.
pub type ProtocolVersion = String;

/// The newest revision this crate knows how to name.
pub const LATEST_PROTOCOL_VERSION: &str = "2026-07-28";

/// A client or server implementation's name and version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Implementation {
    /// Programmatic name.
    pub name: String,
    /// Version string.
    pub version: String,
    /// Human-readable name, if any.
    pub title: Option<String>,
}

impl Implementation {
    /// An implementation with no title.
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Implementation {
            name: name.into(),
            version: version.into(),
            title: None,
        }
    }

    /// This as a JSON value.
    pub fn to_value(&self) -> Value {
        Obj::new()
            .set("name", self.name.as_str())
            .opt("title", self.title.as_deref())
            .set("version", self.version.as_str())
            .finish()
    }

    /// Decode from a JSON value.
    pub fn from_value(v: &Value) -> Result<Self> {
        expect_object(v, "Implementation")?;
        Ok(Implementation {
            name: string(v, "Implementation", "name")?,
            version: string(v, "Implementation", "version")?,
            title: opt_string(v, "Implementation", "title")?,
        })
    }
}

/// A capability that can announce list changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ListCapability {
    /// The server sends `notifications/.../list_changed`.
    pub list_changed: bool,
}

impl ListCapability {
    fn to_value(self) -> Value {
        let mut obj = Obj::new();
        if self.list_changed {
            obj = obj.set("listChanged", true);
        }
        obj.finish()
    }

    fn from_value(v: &Value, what: &'static str) -> Result<Self> {
        expect_object(v, what)?;
        Ok(ListCapability {
            list_changed: opt_bool(v, what, "listChanged")?.unwrap_or(false),
        })
    }
}

/// The resources capability: list changes plus subscriptions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResourcesCapability {
    /// The server sends `notifications/resources/list_changed`.
    pub list_changed: bool,
    /// Clients may subscribe to changes of one resource.
    pub subscribe: bool,
}

impl ResourcesCapability {
    fn to_value(self) -> Value {
        let mut obj = Obj::new();
        if self.subscribe {
            obj = obj.set("subscribe", true);
        }
        if self.list_changed {
            obj = obj.set("listChanged", true);
        }
        obj.finish()
    }

    fn from_value(v: &Value) -> Result<Self> {
        let what = "ServerCapabilities.resources";
        expect_object(v, what)?;
        Ok(ResourcesCapability {
            list_changed: opt_bool(v, what, "listChanged")?.unwrap_or(false),
            subscribe: opt_bool(v, what, "subscribe")?.unwrap_or(false),
        })
    }
}

/// What a server offers. A `None` capability is not offered.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServerCapabilities {
    /// `tools/list` and `tools/call`.
    pub tools: Option<ListCapability>,
    /// `resources/*`.
    pub resources: Option<ResourcesCapability>,
    /// `prompts/*`.
    pub prompts: Option<ListCapability>,
    /// `logging/setLevel`. Deprecated by SEP-2577 in the newest spec; still read and written for older peers.
    pub logging: bool,
    /// `completion/complete`.
    pub completions: bool,
}

impl ServerCapabilities {
    /// A server that offers tools and nothing else.
    pub fn tools_only() -> Self {
        ServerCapabilities {
            tools: Some(ListCapability::default()),
            ..Self::default()
        }
    }

    /// This as a JSON value.
    pub fn to_value(&self) -> Value {
        let mut obj = Obj::new();
        if self.logging {
            obj = obj.set("logging", Value::object());
        }
        if self.completions {
            obj = obj.set("completions", Value::object());
        }
        if let Some(c) = self.prompts {
            obj = obj.set("prompts", c.to_value());
        }
        if let Some(c) = self.resources {
            obj = obj.set("resources", c.to_value());
        }
        if let Some(c) = self.tools {
            obj = obj.set("tools", c.to_value());
        }
        obj.finish()
    }

    /// Decode from a JSON value.
    pub fn from_value(v: &Value) -> Result<Self> {
        expect_object(v, "ServerCapabilities")?;
        let list = |name: &str, what: &'static str| -> Result<Option<ListCapability>> {
            match v.get(name) {
                None | Some(Value::Null) => Ok(None),
                Some(c) => ListCapability::from_value(c, what).map(Some),
            }
        };
        Ok(ServerCapabilities {
            tools: list("tools", "ServerCapabilities.tools")?,
            resources: match v.get("resources") {
                None | Some(Value::Null) => None,
                Some(c) => Some(ResourcesCapability::from_value(c)?),
            },
            prompts: list("prompts", "ServerCapabilities.prompts")?,
            logging: v.get("logging").is_some_and(|x| !x.is_null()),
            completions: v.get("completions").is_some_and(|x| !x.is_null()),
        })
    }
}

/// The parameters of `initialize`.
#[derive(Clone, Debug, PartialEq)]
pub struct InitializeParams {
    /// The revision the client wants.
    pub protocol_version: ProtocolVersion,
    /// The client's capabilities, kept as-is.
    pub capabilities: Value,
    /// Who the client is.
    pub client_info: Implementation,
}

impl InitializeParams {
    /// This as a JSON value.
    pub fn to_value(&self) -> Value {
        Obj::new()
            .set("protocolVersion", self.protocol_version.as_str())
            .set("capabilities", self.capabilities.clone())
            .set("clientInfo", self.client_info.to_value())
            .finish()
    }

    /// Decode from a JSON value.
    pub fn from_value(v: &Value) -> Result<Self> {
        expect_object(v, "InitializeParams")?;
        Ok(InitializeParams {
            protocol_version: string(v, "InitializeParams", "protocolVersion")?,
            capabilities: opt_value(v, "capabilities").unwrap_or_else(Value::object),
            client_info: Implementation::from_value(field(v, "InitializeParams", "clientInfo")?)?,
        })
    }
}

/// The result of `initialize`.
#[derive(Clone, Debug, PartialEq)]
pub struct InitializeResult {
    /// The revision the server picked.
    pub protocol_version: ProtocolVersion,
    /// What the server offers.
    pub capabilities: ServerCapabilities,
    /// Who the server is.
    pub server_info: Implementation,
    /// Usage hints for the model, if any.
    pub instructions: Option<String>,
}

impl InitializeResult {
    /// This as a JSON value.
    pub fn to_value(&self) -> Value {
        Obj::new()
            .set("protocolVersion", self.protocol_version.as_str())
            .set("capabilities", self.capabilities.to_value())
            .set("serverInfo", self.server_info.to_value())
            .opt("instructions", self.instructions.as_deref())
            .finish()
    }

    /// Decode from a JSON value.
    pub fn from_value(v: &Value) -> Result<Self> {
        expect_object(v, "InitializeResult")?;
        Ok(InitializeResult {
            protocol_version: string(v, "InitializeResult", "protocolVersion")?,
            capabilities: ServerCapabilities::from_value(field(
                v,
                "InitializeResult",
                "capabilities",
            )?)?,
            server_info: Implementation::from_value(field(v, "InitializeResult", "serverInfo")?)?,
            instructions: opt_string(v, "InitializeResult", "instructions")?,
        })
    }
}

/// A tool a server offers.
#[derive(Clone, Debug, PartialEq)]
pub struct Tool {
    /// Programmatic name.
    pub name: String,
    /// Human-readable name, if any.
    pub title: Option<String>,
    /// What the tool does, if described.
    pub description: Option<String>,
    /// JSON Schema for the arguments; build one with [`crate::schema::Schema`].
    pub input_schema: Value,
    /// JSON Schema for `structuredContent`, if the tool declares one.
    pub output_schema: Option<Value>,
}

impl Tool {
    /// A tool with a name, description and input schema.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
    ) -> Self {
        Tool {
            name: name.into(),
            title: None,
            description: Some(description.into()),
            input_schema,
            output_schema: None,
        }
    }

    /// This as a JSON value.
    pub fn to_value(&self) -> Value {
        Obj::new()
            .set("name", self.name.as_str())
            .opt("title", self.title.as_deref())
            .opt("description", self.description.as_deref())
            .set("inputSchema", self.input_schema.clone())
            .opt("outputSchema", self.output_schema.clone())
            .finish()
    }

    /// Decode from a JSON value. A missing `inputSchema` becomes `{"type":"object"}`.
    pub fn from_value(v: &Value) -> Result<Self> {
        expect_object(v, "Tool")?;
        Ok(Tool {
            name: string(v, "Tool", "name")?,
            title: opt_string(v, "Tool", "title")?,
            description: opt_string(v, "Tool", "description")?,
            input_schema: opt_value(v, "inputSchema").unwrap_or_else(|| {
                let mut o = Value::object();
                o.insert("type", "object");
                o
            }),
            output_schema: opt_value(v, "outputSchema"),
        })
    }
}

/// One piece of tool output.
#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    /// Plain text.
    Text(String),
    /// Any other content type (image, audio, resource link, ...), kept as-is.
    Other(Value),
}

impl Content {
    /// This as a JSON value.
    pub fn to_value(&self) -> Value {
        match self {
            Content::Text(text) => Obj::new()
                .set("type", "text")
                .set("text", text.as_str())
                .finish(),
            Content::Other(value) => value.clone(),
        }
    }

    /// Decode from a JSON value.
    pub fn from_value(v: &Value) -> Result<Self> {
        expect_object(v, "Content")?;
        if string(v, "Content", "type")? == "text" {
            return Ok(Content::Text(string(v, "Content", "text")?));
        }
        Ok(Content::Other(v.clone()))
    }
}

/// The parameters of `tools/call`.
#[derive(Clone, Debug, PartialEq)]
pub struct CallToolParams {
    /// Which tool.
    pub name: String,
    /// The arguments object, if any.
    pub arguments: Option<Value>,
}

impl CallToolParams {
    /// This as a JSON value.
    pub fn to_value(&self) -> Value {
        Obj::new()
            .set("name", self.name.as_str())
            .opt("arguments", self.arguments.clone())
            .finish()
    }

    /// Decode from a JSON value; `arguments`, when present, must be an object.
    pub fn from_value(v: &Value) -> Result<Self> {
        expect_object(v, "CallToolParams")?;
        let arguments = opt_value(v, "arguments");
        if arguments.as_ref().is_some_and(|a| !a.is_object()) {
            return Err(Error::decode(
                "CallToolParams",
                "\"arguments\" is not an object",
            ));
        }
        Ok(CallToolParams {
            name: string(v, "CallToolParams", "name")?,
            arguments,
        })
    }
}

/// The result of `tools/call`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CallToolResult {
    /// Unstructured output.
    pub content: Vec<Content>,
    /// Structured output matching the tool's `outputSchema`, if any.
    pub structured_content: Option<Value>,
    /// The tool failed; `content` says why. Tool failures are not protocol errors.
    pub is_error: bool,
}

impl CallToolResult {
    /// A successful result with one text block.
    pub fn text(text: impl Into<String>) -> Self {
        CallToolResult {
            content: vec![Content::Text(text.into())],
            ..Self::default()
        }
    }

    /// A failed result with one text block.
    pub fn error(text: impl Into<String>) -> Self {
        CallToolResult {
            is_error: true,
            ..Self::text(text)
        }
    }

    /// This as a JSON value.
    pub fn to_value(&self) -> Value {
        let mut obj = Obj::new().set(
            "content",
            Value::Array(self.content.iter().map(Content::to_value).collect()),
        );
        obj = obj.opt("structuredContent", self.structured_content.clone());
        if self.is_error {
            obj = obj.set("isError", true);
        }
        obj.finish()
    }

    /// Decode from a JSON value.
    pub fn from_value(v: &Value) -> Result<Self> {
        expect_object(v, "CallToolResult")?;
        Ok(CallToolResult {
            content: opt_array(v, "CallToolResult", "content")?
                .iter()
                .map(Content::from_value)
                .collect::<Result<_>>()?,
            structured_content: opt_value(v, "structuredContent"),
            is_error: opt_bool(v, "CallToolResult", "isError")?.unwrap_or(false),
        })
    }
}

/// The parameters of any paginated list request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListParams {
    /// Where to resume, from a previous `nextCursor`.
    pub cursor: Option<String>,
}

impl ListParams {
    /// This as a JSON value.
    pub fn to_value(&self) -> Value {
        Obj::new().opt("cursor", self.cursor.as_deref()).finish()
    }

    /// Decode from a JSON value.
    pub fn from_value(v: &Value) -> Result<Self> {
        expect_object(v, "ListParams")?;
        Ok(ListParams {
            cursor: opt_string(v, "ListParams", "cursor")?,
        })
    }
}

/// The result of `tools/list`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ListToolsResult {
    /// This page of tools.
    pub tools: Vec<Tool>,
    /// Pass back as `cursor` for the next page; `None` on the last page.
    pub next_cursor: Option<String>,
}

impl ListToolsResult {
    /// This as a JSON value.
    pub fn to_value(&self) -> Value {
        Obj::new()
            .set(
                "tools",
                Value::Array(self.tools.iter().map(Tool::to_value).collect()),
            )
            .opt("nextCursor", self.next_cursor.as_deref())
            .finish()
    }

    /// Decode from a JSON value.
    pub fn from_value(v: &Value) -> Result<Self> {
        expect_object(v, "ListToolsResult")?;
        Ok(ListToolsResult {
            tools: opt_array(v, "ListToolsResult", "tools")?
                .iter()
                .map(Tool::from_value)
                .collect::<Result<_>>()?,
            next_cursor: opt_string(v, "ListToolsResult", "nextCursor")?,
        })
    }
}
