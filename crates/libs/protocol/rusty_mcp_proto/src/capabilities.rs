//! What each side says it can do, exchanged on connect.

use crate::codec::{object, opt_bool, opt_value, Obj, Wire};
use crate::Result;
use rusty_json::Value;

/// `prompts` capability of a server.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PromptsCapability {
    /// The server sends `notifications/prompts/list_changed`.
    pub list_changed: Option<bool>,
}

/// `resources` capability of a server.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResourcesCapability {
    /// Clients may subscribe to single resources.
    pub subscribe: Option<bool>,
    /// The server sends `notifications/resources/list_changed`.
    pub list_changed: Option<bool>,
}

/// `tools` capability of a server.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolsCapability {
    /// The server sends `notifications/tools/list_changed`.
    pub list_changed: Option<bool>,
}

impl Wire for PromptsCapability {
    fn to_value(&self) -> Value {
        Obj::new().opt("listChanged", self.list_changed).done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        object(v, "PromptsCapability")?;
        Ok(Self {
            list_changed: opt_bool(v, "PromptsCapability", "listChanged")?,
        })
    }
}

impl Wire for ResourcesCapability {
    fn to_value(&self) -> Value {
        Obj::new()
            .opt("subscribe", self.subscribe)
            .opt("listChanged", self.list_changed)
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "ResourcesCapability";
        object(v, W)?;
        Ok(Self {
            subscribe: opt_bool(v, W, "subscribe")?,
            list_changed: opt_bool(v, W, "listChanged")?,
        })
    }
}

impl Wire for ToolsCapability {
    fn to_value(&self) -> Value {
        Obj::new().opt("listChanged", self.list_changed).done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        object(v, "ToolsCapability")?;
        Ok(Self {
            list_changed: opt_bool(v, "ToolsCapability", "listChanged")?,
        })
    }
}

/// What a server offers. The features a server builds on (prompts,
/// resources, tools) are typed; the open-ended ones stay raw JSON.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServerCapabilities {
    /// Non-standard capabilities, raw.
    pub experimental: Option<Value>,
    /// Extensions by identifier (e.g. `io.modelcontextprotocol/tasks`), raw.
    pub extensions: Option<Value>,
    /// The server can send log messages, raw.
    pub logging: Option<Value>,
    /// The server can complete arguments, raw.
    pub completions: Option<Value>,
    /// Prompts.
    pub prompts: Option<PromptsCapability>,
    /// Resources.
    pub resources: Option<ResourcesCapability>,
    /// Tools.
    pub tools: Option<ToolsCapability>,
}

fn typed<T: Wire>(v: &Value, name: &str) -> Result<Option<T>> {
    opt_value(v, name).as_ref().map(T::from_value).transpose()
}

impl Wire for ServerCapabilities {
    fn to_value(&self) -> Value {
        Obj::new()
            .opt_value("experimental", &self.experimental)
            .opt_value("extensions", &self.extensions)
            .opt_value("logging", &self.logging)
            .opt_value("completions", &self.completions)
            .opt_value("prompts", &self.prompts.as_ref().map(Wire::to_value))
            .opt_value("resources", &self.resources.as_ref().map(Wire::to_value))
            .opt_value("tools", &self.tools.as_ref().map(Wire::to_value))
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        object(v, "ServerCapabilities")?;
        Ok(Self {
            experimental: opt_value(v, "experimental"),
            extensions: opt_value(v, "extensions"),
            logging: opt_value(v, "logging"),
            completions: opt_value(v, "completions"),
            prompts: typed(v, "prompts")?,
            resources: typed(v, "resources")?,
            tools: typed(v, "tools")?,
        })
    }
}

/// What a client offers. The server only reads these, so all are raw JSON.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClientCapabilities {
    /// Non-standard capabilities.
    pub experimental: Option<Value>,
    /// Extensions by identifier.
    pub extensions: Option<Value>,
    /// The client can list roots.
    pub roots: Option<Value>,
    /// The client can sample from a model.
    pub sampling: Option<Value>,
    /// The client can elicit input from the user.
    pub elicitation: Option<Value>,
}

impl Wire for ClientCapabilities {
    fn to_value(&self) -> Value {
        Obj::new()
            .opt_value("experimental", &self.experimental)
            .opt_value("extensions", &self.extensions)
            .opt_value("roots", &self.roots)
            .opt_value("sampling", &self.sampling)
            .opt_value("elicitation", &self.elicitation)
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        object(v, "ClientCapabilities")?;
        Ok(Self {
            experimental: opt_value(v, "experimental"),
            extensions: opt_value(v, "extensions"),
            roots: opt_value(v, "roots"),
            sampling: opt_value(v, "sampling"),
            elicitation: opt_value(v, "elicitation"),
        })
    }
}
