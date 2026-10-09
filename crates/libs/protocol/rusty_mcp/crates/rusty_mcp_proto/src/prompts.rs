//! Prompts: `prompts/list` and `prompts/get`.

use crate::types::Content;
use crate::util::{
    expect_object, field, map_to_value, opt_array, opt_bool, opt_string, opt_string_map, string,
    Obj,
};
use crate::{Error, Result};
use rusty_json::Value;
use std::collections::BTreeMap;

/// An argument a prompt accepts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptArgument {
    /// Programmatic name.
    pub name: String,
    /// Human-readable name, if any.
    pub title: Option<String>,
    /// What it is for, if described.
    pub description: Option<String>,
    /// Whether it must be supplied; `None` means the server did not say.
    pub required: Option<bool>,
}

impl PromptArgument {
    /// An argument with only a name.
    pub fn new(name: impl Into<String>) -> Self {
        PromptArgument {
            name: name.into(),
            title: None,
            description: None,
            required: None,
        }
    }

    /// This as a JSON value.
    pub fn to_value(&self) -> Value {
        Obj::new()
            .set("name", self.name.as_str())
            .opt("title", self.title.as_deref())
            .opt("description", self.description.as_deref())
            .opt("required", self.required)
            .finish()
    }

    /// Decode from a JSON value.
    pub fn from_value(v: &Value) -> Result<Self> {
        expect_object(v, "PromptArgument")?;
        Ok(PromptArgument {
            name: string(v, "PromptArgument", "name")?,
            title: opt_string(v, "PromptArgument", "title")?,
            description: opt_string(v, "PromptArgument", "description")?,
            required: opt_bool(v, "PromptArgument", "required")?,
        })
    }
}

/// A prompt template a server offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
    /// Programmatic name.
    pub name: String,
    /// Human-readable name, if any.
    pub title: Option<String>,
    /// What it does, if described.
    pub description: Option<String>,
    /// The arguments it accepts; left out of the wire form when empty.
    pub arguments: Vec<PromptArgument>,
}

impl Prompt {
    /// A prompt with only a name.
    pub fn new(name: impl Into<String>) -> Self {
        Prompt {
            name: name.into(),
            title: None,
            description: None,
            arguments: Vec::new(),
        }
    }

    /// This as a JSON value.
    pub fn to_value(&self) -> Value {
        let mut obj = Obj::new()
            .set("name", self.name.as_str())
            .opt("title", self.title.as_deref())
            .opt("description", self.description.as_deref());
        if !self.arguments.is_empty() {
            obj = obj.set(
                "arguments",
                Value::Array(
                    self.arguments
                        .iter()
                        .map(PromptArgument::to_value)
                        .collect(),
                ),
            );
        }
        obj.finish()
    }

    /// Decode from a JSON value.
    pub fn from_value(v: &Value) -> Result<Self> {
        expect_object(v, "Prompt")?;
        Ok(Prompt {
            name: string(v, "Prompt", "name")?,
            title: opt_string(v, "Prompt", "title")?,
            description: opt_string(v, "Prompt", "description")?,
            arguments: opt_array(v, "Prompt", "arguments")?
                .iter()
                .map(PromptArgument::from_value)
                .collect::<Result<_>>()?,
        })
    }
}

/// The result of `prompts/list`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListPromptsResult {
    /// This page of prompts.
    pub prompts: Vec<Prompt>,
    /// Pass back as `cursor` for the next page.
    pub next_cursor: Option<String>,
}

impl ListPromptsResult {
    /// This as a JSON value.
    pub fn to_value(&self) -> Value {
        Obj::new()
            .set(
                "prompts",
                Value::Array(self.prompts.iter().map(Prompt::to_value).collect()),
            )
            .opt("nextCursor", self.next_cursor.as_deref())
            .finish()
    }

    /// Decode from a JSON value.
    pub fn from_value(v: &Value) -> Result<Self> {
        expect_object(v, "ListPromptsResult")?;
        Ok(ListPromptsResult {
            prompts: opt_array(v, "ListPromptsResult", "prompts")?
                .iter()
                .map(Prompt::from_value)
                .collect::<Result<_>>()?,
            next_cursor: opt_string(v, "ListPromptsResult", "nextCursor")?,
        })
    }
}

/// The parameters of `prompts/get`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GetPromptParams {
    /// Which prompt.
    pub name: String,
    /// Argument values, all strings; left out of the wire form when empty.
    pub arguments: BTreeMap<String, String>,
}

impl GetPromptParams {
    /// This as a JSON value.
    pub fn to_value(&self) -> Value {
        let mut obj = Obj::new().set("name", self.name.as_str());
        if !self.arguments.is_empty() {
            obj = obj.set("arguments", map_to_value(&self.arguments));
        }
        obj.finish()
    }

    /// Decode from a JSON value.
    pub fn from_value(v: &Value) -> Result<Self> {
        expect_object(v, "GetPromptParams")?;
        Ok(GetPromptParams {
            name: string(v, "GetPromptParams", "name")?,
            arguments: opt_string_map(v, "GetPromptParams", "arguments")?,
        })
    }
}

/// Who a prompt message is from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// The user.
    User,
    /// The assistant.
    Assistant,
}

impl Role {
    fn as_str(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }

    fn parse(text: &str) -> Result<Self> {
        match text {
            "user" => Ok(Role::User),
            "assistant" => Ok(Role::Assistant),
            other => Err(Error::decode("Role", format!("unknown role {other:?}"))),
        }
    }
}

/// One message of a rendered prompt.
#[derive(Clone, Debug, PartialEq)]
pub struct PromptMessage {
    /// Who it is from.
    pub role: Role,
    /// What it says.
    pub content: Content,
}

impl PromptMessage {
    /// A text message.
    pub fn text(role: Role, text: impl Into<String>) -> Self {
        PromptMessage {
            role,
            content: Content::Text(text.into()),
        }
    }

    /// This as a JSON value.
    pub fn to_value(&self) -> Value {
        Obj::new()
            .set("role", self.role.as_str())
            .set("content", self.content.to_value())
            .finish()
    }

    /// Decode from a JSON value.
    pub fn from_value(v: &Value) -> Result<Self> {
        expect_object(v, "PromptMessage")?;
        Ok(PromptMessage {
            role: Role::parse(&string(v, "PromptMessage", "role")?)?,
            content: Content::from_value(field(v, "PromptMessage", "content")?)?,
        })
    }
}

/// The result of `prompts/get`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GetPromptResult {
    /// What the rendered prompt is, if described.
    pub description: Option<String>,
    /// The messages.
    pub messages: Vec<PromptMessage>,
}

impl GetPromptResult {
    /// This as a JSON value.
    pub fn to_value(&self) -> Value {
        Obj::new()
            .opt("description", self.description.as_deref())
            .set(
                "messages",
                Value::Array(self.messages.iter().map(PromptMessage::to_value).collect()),
            )
            .finish()
    }

    /// Decode from a JSON value.
    pub fn from_value(v: &Value) -> Result<Self> {
        expect_object(v, "GetPromptResult")?;
        Ok(GetPromptResult {
            description: opt_string(v, "GetPromptResult", "description")?,
            messages: opt_array(v, "GetPromptResult", "messages")?
                .iter()
                .map(PromptMessage::from_value)
                .collect::<Result<_>>()?,
        })
    }
}
