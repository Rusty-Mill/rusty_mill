//! `prompts/list` and `prompts/get`.

use crate::codec::{
    decode_all, encode_all, field, object, opt_array, opt_bool, opt_string, opt_value, string, Obj,
    Wire,
};
use crate::content::ContentBlock;
use crate::page::{decode_list, encode_list, Paging, ResultType};
use crate::{Error, Result};
use rusty_json::Value;

/// Method names for prompts.
pub mod method {
    /// List the prompts a server offers.
    pub const LIST: &str = "prompts/list";
    /// Get one prompt, filled in.
    pub const GET: &str = "prompts/get";
}

/// Who a message is from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// The human side.
    User,
    /// The model side.
    Assistant,
}

impl Wire for Role {
    fn to_value(&self) -> Value {
        Value::from(match self {
            Role::User => "user",
            Role::Assistant => "assistant",
        })
    }

    fn from_value(v: &Value) -> Result<Self> {
        match v.as_str() {
            Some("user") => Ok(Role::User),
            Some("assistant") => Ok(Role::Assistant),
            _ => Err(Error::decode("role", "not \"user\" or \"assistant\"")),
        }
    }
}

/// An argument a prompt takes.
#[derive(Clone, Debug, PartialEq)]
pub struct PromptArgument {
    /// Programmatic name.
    pub name: String,
    /// Human-readable name.
    pub title: Option<String>,
    /// What it is for.
    pub description: Option<String>,
    /// Whether it must be given.
    pub required: Option<bool>,
}

impl Wire for PromptArgument {
    fn to_value(&self) -> Value {
        Obj::new()
            .set("name", self.name.as_str())
            .opt("title", self.title.clone())
            .opt("description", self.description.clone())
            .opt("required", self.required)
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "PromptArgument";
        object(v, W)?;
        Ok(Self {
            name: string(v, W, "name")?,
            title: opt_string(v, W, "title")?,
            description: opt_string(v, W, "description")?,
            required: opt_bool(v, W, "required")?,
        })
    }
}

/// A prompt template a server offers.
#[derive(Clone, Debug, PartialEq)]
pub struct Prompt {
    /// Programmatic name.
    pub name: String,
    /// Human-readable name.
    pub title: Option<String>,
    /// What it does.
    pub description: Option<String>,
    /// The arguments it takes.
    pub arguments: Option<Vec<PromptArgument>>,
    /// Icons, raw JSON.
    pub icons: Option<Value>,
    /// `_meta`, raw JSON.
    pub meta: Option<Value>,
}

impl Prompt {
    /// A prompt with only the required member.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            title: None,
            description: None,
            arguments: None,
            icons: None,
            meta: None,
        }
    }
}

impl Wire for Prompt {
    fn to_value(&self) -> Value {
        Obj::new()
            .set("name", self.name.as_str())
            .opt("title", self.title.clone())
            .opt("description", self.description.clone())
            .opt("arguments", self.arguments.as_deref().map(encode_all))
            .opt_value("icons", &self.icons)
            .opt_value("_meta", &self.meta)
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "Prompt";
        object(v, W)?;
        let arguments = match v.get("arguments") {
            None | Some(Value::Null) => None,
            Some(_) => Some(decode_all(opt_array(v, W, "arguments")?)?),
        };
        Ok(Self {
            name: string(v, W, "name")?,
            title: opt_string(v, W, "title")?,
            description: opt_string(v, W, "description")?,
            arguments,
            icons: opt_value(v, "icons"),
            meta: opt_value(v, "_meta"),
        })
    }
}

/// A page of `prompts/list`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ListPromptsResult {
    /// The prompts on this page.
    pub prompts: Vec<Prompt>,
    /// Cursor, cache hints and `_meta`.
    pub paging: Paging,
}

impl Wire for ListPromptsResult {
    fn to_value(&self) -> Value {
        encode_list("prompts", &self.prompts, &self.paging)
    }

    fn from_value(v: &Value) -> Result<Self> {
        let (prompts, paging) = decode_list(v, "ListPromptsResult", "prompts")?;
        Ok(Self { prompts, paging })
    }
}

/// Parameters of `prompts/get`.
#[derive(Clone, Debug, PartialEq)]
pub struct GetPromptParams {
    /// Which prompt.
    pub name: String,
    /// The arguments, a JSON object of strings.
    pub arguments: Option<Value>,
    /// 2026-07-28: answers to an earlier `input_required` result.
    pub input_responses: Option<Value>,
    /// 2026-07-28: the opaque state the server asked to have echoed.
    pub request_state: Option<String>,
    /// Request `_meta`, raw JSON.
    pub meta: Option<Value>,
}

impl GetPromptParams {
    /// A request with no arguments.
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

impl Wire for GetPromptParams {
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
        const W: &str = "GetPromptParams";
        object(v, W)?;
        let arguments = opt_value(v, "arguments");
        if let Some(a) = &arguments {
            object(a, "GetPromptParams arguments")?;
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

/// One message of a filled-in prompt.
#[derive(Clone, Debug, PartialEq)]
pub struct PromptMessage {
    /// Who it is from.
    pub role: Role,
    /// What it says.
    pub content: ContentBlock,
}

impl Wire for PromptMessage {
    fn to_value(&self) -> Value {
        Obj::new()
            .set("role", self.role.to_value())
            .set("content", self.content.to_value())
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "PromptMessage";
        object(v, W)?;
        Ok(Self {
            role: Role::from_value(field(v, W, "role")?)?,
            content: ContentBlock::from_value(field(v, W, "content")?)?,
        })
    }
}

/// The result of a finished `prompts/get`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GetPromptResult {
    /// `resultType`.
    pub result_type: Option<ResultType>,
    /// What the prompt is.
    pub description: Option<String>,
    /// The messages.
    pub messages: Vec<PromptMessage>,
    /// Result `_meta`, raw JSON.
    pub meta: Option<Value>,
}

impl Wire for GetPromptResult {
    fn to_value(&self) -> Value {
        Obj::new()
            .opt("resultType", self.result_type.as_ref().map(|t| t.0.clone()))
            .opt("description", self.description.clone())
            .set("messages", encode_all(&self.messages))
            .opt_value("_meta", &self.meta)
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "GetPromptResult";
        object(v, W)?;
        Ok(Self {
            result_type: opt_string(v, W, "resultType")?.map(ResultType),
            description: opt_string(v, W, "description")?,
            messages: decode_all(crate::codec::array(v, W, "messages")?)?,
            meta: opt_value(v, "_meta"),
        })
    }
}
