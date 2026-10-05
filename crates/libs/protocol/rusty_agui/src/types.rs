//! The run input and the message model, as the TypeScript SDK's `core`
//! types define them.

use rusty_json::Value;

/// Who authored a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    /// Instructions from the developer (OpenAI's `developer`).
    Developer,
    /// System instructions.
    System,
    /// The agent.
    Assistant,
    /// The person.
    User,
    /// A tool's result.
    Tool,
    /// Structured in-progress activity (plans, searches).
    Activity,
    /// Model reasoning.
    Reasoning,
}

impl Role {
    /// The wire string.
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Developer => "developer",
            Role::System => "system",
            Role::Assistant => "assistant",
            Role::User => "user",
            Role::Tool => "tool",
            Role::Activity => "activity",
            Role::Reasoning => "reasoning",
        }
    }

    /// Parses the wire string.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "developer" => Role::Developer,
            "system" => Role::System,
            "assistant" => Role::Assistant,
            "user" => Role::User,
            "tool" => Role::Tool,
            "activity" => Role::Activity,
            "reasoning" => Role::Reasoning,
            _ => return None,
        })
    }
}

/// One part of a multimodal message.
#[derive(Clone, Debug, PartialEq)]
pub enum ContentPart {
    /// Plain text.
    Text(String),
    /// A binary attachment, inline (`data`, base64) or by reference (`url`).
    Binary {
        /// Media type, e.g. `image/png`.
        mime_type: String,
        /// Base64 payload, if inline.
        data: Option<String>,
        /// Where to fetch it, if by reference.
        url: Option<String>,
        /// Display name.
        filename: Option<String>,
    },
}

/// Message content: a string, or parts for multimodal input.
#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    /// Plain text.
    Text(String),
    /// Mixed parts.
    Parts(Vec<ContentPart>),
}

impl Content {
    /// The text of a `Text` content, or the concatenated text parts.
    pub fn text(&self) -> String {
        match self {
            Content::Text(s) => s.clone(),
            Content::Parts(parts) => parts
                .iter()
                .filter_map(|p| match p {
                    ContentPart::Text(s) => Some(s.as_str()),
                    ContentPart::Binary { .. } => None,
                })
                .collect(),
        }
    }
}

impl From<&str> for Content {
    fn from(s: &str) -> Self {
        Content::Text(s.into())
    }
}

impl From<String> for Content {
    fn from(s: String) -> Self {
        Content::Text(s)
    }
}

/// The function an assistant asked to call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionCall {
    /// Tool name.
    pub name: String,
    /// Arguments as a JSON text (streamed, so possibly partial).
    pub arguments: String,
}

/// A tool call attached to an assistant message. `type` is always
/// `function` on the wire.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCall {
    /// Correlates with the tool message that answers it.
    pub id: String,
    /// What to call.
    pub function: FunctionCall,
}

/// One message in a thread. A variant per role, so a tool message always
/// has its `tool_call_id` and an assistant message its `tool_calls`.
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    /// From the person.
    User {
        /// Message id.
        id: String,
        /// Text or parts.
        content: Content,
        /// Optional author name.
        name: Option<String>,
    },
    /// From the agent.
    Assistant {
        /// Message id.
        id: String,
        /// Text, absent when the message only carries tool calls.
        content: Option<String>,
        /// Optional author name.
        name: Option<String>,
        /// Tool calls the agent made in this turn.
        tool_calls: Vec<ToolCall>,
    },
    /// System instructions.
    System {
        /// Message id.
        id: String,
        /// Text.
        content: String,
        /// Optional author name.
        name: Option<String>,
    },
    /// Developer instructions.
    Developer {
        /// Message id.
        id: String,
        /// Text.
        content: String,
        /// Optional author name.
        name: Option<String>,
    },
    /// A tool's result, answering one tool call.
    Tool {
        /// Message id.
        id: String,
        /// The result.
        content: Content,
        /// The call this answers.
        tool_call_id: String,
        /// Set when the client-side tool failed.
        error: Option<String>,
    },
    /// Structured activity between messages.
    Activity {
        /// Message id.
        id: String,
        /// Discriminator, e.g. `PLAN`.
        activity_type: String,
        /// The structured content.
        content: Value,
    },
    /// Model reasoning.
    Reasoning {
        /// Message id.
        id: String,
        /// Reasoning text.
        content: String,
        /// Provider-encrypted reasoning, replayed on later turns.
        encrypted_value: Option<String>,
    },
}

impl Message {
    /// The message id.
    pub fn id(&self) -> &str {
        match self {
            Message::User { id, .. }
            | Message::Assistant { id, .. }
            | Message::System { id, .. }
            | Message::Developer { id, .. }
            | Message::Tool { id, .. }
            | Message::Activity { id, .. }
            | Message::Reasoning { id, .. } => id,
        }
    }

    /// The role.
    pub fn role(&self) -> Role {
        match self {
            Message::User { .. } => Role::User,
            Message::Assistant { .. } => Role::Assistant,
            Message::System { .. } => Role::System,
            Message::Developer { .. } => Role::Developer,
            Message::Tool { .. } => Role::Tool,
            Message::Activity { .. } => Role::Activity,
            Message::Reasoning { .. } => Role::Reasoning,
        }
    }

    /// A user message with text content.
    pub fn user(id: impl Into<String>, text: impl Into<Content>) -> Self {
        Message::User {
            id: id.into(),
            content: text.into(),
            name: None,
        }
    }

    /// An assistant message with text and no tool calls.
    pub fn assistant(id: impl Into<String>, text: impl Into<String>) -> Self {
        Message::Assistant {
            id: id.into(),
            content: Some(text.into()),
            name: None,
            tool_calls: Vec::new(),
        }
    }
}

/// A client-side tool the agent may call.
#[derive(Clone, Debug, PartialEq)]
pub struct Tool {
    /// Tool name.
    pub name: String,
    /// What it does, for the model.
    pub description: String,
    /// JSON Schema for the arguments.
    pub parameters: Value,
}

/// A piece of application context the client exposes to the agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Context {
    /// What the value is.
    pub description: String,
    /// The value, as text.
    pub value: String,
}

/// Everything the client sends to start a run.
#[derive(Clone, Debug, PartialEq)]
pub struct RunAgentInput {
    /// Conversation id, stable across runs.
    pub thread_id: String,
    /// This run's id.
    pub run_id: String,
    /// The run this branches from, if any.
    pub parent_run_id: Option<String>,
    /// Shared state as the client last saw it.
    pub state: Value,
    /// The thread so far.
    pub messages: Vec<Message>,
    /// Client-side tools available to the agent.
    pub tools: Vec<Tool>,
    /// Readable context.
    pub context: Vec<Context>,
    /// Anything else the client forwards to the agent.
    pub forwarded_props: Value,
}

impl RunAgentInput {
    /// A run with ids and messages only; the rest empty.
    pub fn new(
        thread_id: impl Into<String>,
        run_id: impl Into<String>,
        messages: Vec<Message>,
    ) -> Self {
        Self {
            thread_id: thread_id.into(),
            run_id: run_id.into(),
            parent_run_id: None,
            state: Value::Null,
            messages,
            tools: Vec::new(),
            context: Vec::new(),
            forwarded_props: Value::Null,
        }
    }
}
