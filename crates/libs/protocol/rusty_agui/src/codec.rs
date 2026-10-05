//! `Value` in and out for every protocol type. Hand-written against
//! `rusty_json::Value` so the crate carries no serde. Encoding emits only
//! the fields the TypeScript SDK emits; decoding ignores members it does
//! not know, so a newer peer still parses.

use crate::event::{Event, EventKind, EventMeta, RunOutcome};
use crate::types::{
    Content, ContentPart, Context, FunctionCall, Message, Role, RunAgentInput, Tool, ToolCall,
};
use crate::{Error, Result};
use rusty_json::Value;

// ---------------------------------------------------------------- helpers

/// An object under construction; `None` and empty-vector fields are left
/// out, which is what the TypeScript encoder does.
struct Obj(Value);

impl Obj {
    fn new() -> Self {
        Obj(Value::object())
    }

    fn set(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.0.insert(key, value);
        self
    }

    fn opt(self, key: &str, value: Option<impl Into<Value>>) -> Self {
        match value {
            Some(v) => self.set(key, v),
            None => self,
        }
    }

    fn list(self, key: &str, items: Vec<Value>) -> Self {
        if items.is_empty() {
            return self;
        }
        self.set(key, Value::Array(items))
    }
}

fn field<'a>(v: &'a Value, what: &'static str, name: &'static str) -> Result<&'a Value> {
    v.get(name)
        .filter(|f| !f.is_null())
        .ok_or_else(|| Error::decode(what, format!("missing {name:?}")))
}

fn string(v: &Value, what: &'static str, name: &'static str) -> Result<String> {
    field(v, what, name)?
        .as_str()
        .map(String::from)
        .ok_or_else(|| Error::decode(what, format!("{name:?} is not a string")))
}

fn opt_string(v: &Value, what: &'static str, name: &'static str) -> Result<Option<String>> {
    match v.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(Error::decode(what, format!("{name:?} is not a string"))),
    }
}

fn opt_value(v: &Value, name: &str) -> Option<Value> {
    v.get(name).filter(|f| !f.is_null()).cloned()
}

fn array<'a>(v: &'a Value, what: &'static str, name: &'static str) -> Result<&'a [Value]> {
    field(v, what, name)?
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| Error::decode(what, format!("{name:?} is not an array")))
}

fn opt_array<'a>(v: &'a Value, what: &'static str, name: &'static str) -> Result<&'a [Value]> {
    match v.get(name) {
        None | Some(Value::Null) => Ok(&[]),
        Some(_) => array(v, what, name),
    }
}

fn role(v: &Value, what: &'static str, name: &'static str) -> Result<Role> {
    let text = string(v, what, name)?;
    Role::parse(&text).ok_or_else(|| Error::decode(what, format!("unknown role {text:?}")))
}

fn opt_role(v: &Value, what: &'static str, name: &'static str) -> Result<Option<Role>> {
    opt_string(v, what, name)?
        .map(|text| {
            Role::parse(&text).ok_or_else(|| Error::decode(what, format!("unknown role {text:?}")))
        })
        .transpose()
}

// ---------------------------------------------------------------- content

fn content_to_value(content: &Content) -> Value {
    match content {
        Content::Text(s) => Value::from(s.as_str()),
        Content::Parts(parts) => Value::Array(parts.iter().map(part_to_value).collect()),
    }
}

fn part_to_value(part: &ContentPart) -> Value {
    match part {
        ContentPart::Text(text) => Obj::new().set("type", "text").set("text", text.as_str()).0,
        ContentPart::Binary {
            mime_type,
            data,
            url,
            filename,
        } => {
            Obj::new()
                .set("type", "binary")
                .set("mimeType", mime_type.as_str())
                .opt("data", data.as_deref())
                .opt("url", url.as_deref())
                .opt("filename", filename.as_deref())
                .0
        }
    }
}

fn content_from_value(v: &Value, what: &'static str) -> Result<Content> {
    match v {
        Value::String(s) => Ok(Content::Text(s.clone())),
        Value::Array(parts) => parts
            .iter()
            .map(|p| part_from_value(p, what))
            .collect::<Result<Vec<_>>>()
            .map(Content::Parts),
        _ => Err(Error::decode(what, "content is neither a string nor parts")),
    }
}

fn part_from_value(v: &Value, what: &'static str) -> Result<ContentPart> {
    match string(v, what, "type")?.as_str() {
        "text" => Ok(ContentPart::Text(string(v, what, "text")?)),
        "binary" => Ok(ContentPart::Binary {
            mime_type: string(v, what, "mimeType")?,
            data: opt_string(v, what, "data")?,
            url: opt_string(v, what, "url")?,
            filename: opt_string(v, what, "filename")?,
        }),
        other => Err(Error::decode(
            what,
            format!("unknown content part {other:?}"),
        )),
    }
}

// --------------------------------------------------------------- messages

fn tool_call_to_value(call: &ToolCall) -> Value {
    Obj::new()
        .set("id", call.id.as_str())
        .set("type", "function")
        .set(
            "function",
            Obj::new()
                .set("name", call.function.name.as_str())
                .set("arguments", call.function.arguments.as_str())
                .0,
        )
        .0
}

fn tool_call_from_value(v: &Value) -> Result<ToolCall> {
    const WHAT: &str = "tool call";
    let function = field(v, WHAT, "function")?;
    Ok(ToolCall {
        id: string(v, WHAT, "id")?,
        function: FunctionCall {
            name: string(function, WHAT, "name")?,
            arguments: string(function, WHAT, "arguments")?,
        },
    })
}

/// Encodes a message.
pub fn message_to_value(message: &Message) -> Value {
    let base = Obj::new()
        .set("id", message.id())
        .set("role", message.role().as_str());
    match message {
        Message::User { content, name, .. } => {
            base.set("content", content_to_value(content))
                .opt("name", name.as_deref())
                .0
        }
        Message::Assistant {
            content,
            name,
            tool_calls,
            ..
        } => {
            base.opt("content", content.as_deref())
                .opt("name", name.as_deref())
                .list(
                    "toolCalls",
                    tool_calls.iter().map(tool_call_to_value).collect(),
                )
                .0
        }
        Message::System { content, name, .. } | Message::Developer { content, name, .. } => {
            base.set("content", content.as_str())
                .opt("name", name.as_deref())
                .0
        }
        Message::Tool {
            content,
            tool_call_id,
            error,
            ..
        } => {
            base.set("content", content_to_value(content))
                .set("toolCallId", tool_call_id.as_str())
                .opt("error", error.as_deref())
                .0
        }
        Message::Activity {
            activity_type,
            content,
            ..
        } => {
            base.set("activityType", activity_type.as_str())
                .set("content", content.clone())
                .0
        }
        Message::Reasoning {
            content,
            encrypted_value,
            ..
        } => {
            base.set("content", content.as_str())
                .opt("encryptedValue", encrypted_value.as_deref())
                .0
        }
    }
}

/// Decodes a message.
pub fn message_from_value(v: &Value) -> Result<Message> {
    const WHAT: &str = "message";
    let id = string(v, WHAT, "id")?;
    let name = opt_string(v, WHAT, "name")?;
    Ok(match role(v, WHAT, "role")? {
        Role::User => Message::User {
            id,
            content: content_from_value(field(v, WHAT, "content")?, WHAT)?,
            name,
        },
        Role::Assistant => Message::Assistant {
            id,
            content: opt_string(v, WHAT, "content")?,
            name,
            tool_calls: opt_array(v, WHAT, "toolCalls")?
                .iter()
                .map(tool_call_from_value)
                .collect::<Result<_>>()?,
        },
        Role::System => Message::System {
            id,
            content: string(v, WHAT, "content")?,
            name,
        },
        Role::Developer => Message::Developer {
            id,
            content: string(v, WHAT, "content")?,
            name,
        },
        Role::Tool => Message::Tool {
            id,
            content: content_from_value(field(v, WHAT, "content")?, WHAT)?,
            tool_call_id: string(v, WHAT, "toolCallId")?,
            error: opt_string(v, WHAT, "error")?,
        },
        Role::Activity => Message::Activity {
            id,
            activity_type: string(v, WHAT, "activityType")?,
            content: field(v, WHAT, "content")?.clone(),
        },
        Role::Reasoning => Message::Reasoning {
            id,
            content: string(v, WHAT, "content")?,
            encrypted_value: opt_string(v, WHAT, "encryptedValue")?,
        },
    })
}

fn messages_from_value(items: &[Value]) -> Result<Vec<Message>> {
    items.iter().map(message_from_value).collect()
}

// ------------------------------------------------------------------ input

fn tool_to_value(tool: &Tool) -> Value {
    Obj::new()
        .set("name", tool.name.as_str())
        .set("description", tool.description.as_str())
        .set("parameters", tool.parameters.clone())
        .0
}

fn tool_from_value(v: &Value) -> Result<Tool> {
    const WHAT: &str = "tool";
    Ok(Tool {
        name: string(v, WHAT, "name")?,
        description: opt_string(v, WHAT, "description")?.unwrap_or_default(),
        parameters: opt_value(v, "parameters").unwrap_or_else(Value::object),
    })
}

fn context_to_value(context: &Context) -> Value {
    Obj::new()
        .set("description", context.description.as_str())
        .set("value", context.value.as_str())
        .0
}

fn context_from_value(v: &Value) -> Result<Context> {
    const WHAT: &str = "context";
    Ok(Context {
        description: string(v, WHAT, "description")?,
        value: string(v, WHAT, "value")?,
    })
}

/// Encodes a run input.
pub fn input_to_value(input: &RunAgentInput) -> Value {
    Obj::new()
        .set("threadId", input.thread_id.as_str())
        .set("runId", input.run_id.as_str())
        .opt("parentRunId", input.parent_run_id.as_deref())
        .set("state", input.state.clone())
        .set(
            "messages",
            Value::Array(input.messages.iter().map(message_to_value).collect()),
        )
        .set(
            "tools",
            Value::Array(input.tools.iter().map(tool_to_value).collect()),
        )
        .set(
            "context",
            Value::Array(input.context.iter().map(context_to_value).collect()),
        )
        .set("forwardedProps", input.forwarded_props.clone())
        .0
}

/// Decodes a run input. `state`, `messages`, `tools`, `context` and
/// `forwardedProps` may be absent.
pub fn input_from_value(v: &Value) -> Result<RunAgentInput> {
    const WHAT: &str = "run input";
    Ok(RunAgentInput {
        thread_id: string(v, WHAT, "threadId")?,
        run_id: string(v, WHAT, "runId")?,
        parent_run_id: opt_string(v, WHAT, "parentRunId")?,
        state: opt_value(v, "state").unwrap_or(Value::Null),
        messages: messages_from_value(opt_array(v, WHAT, "messages")?)?,
        tools: opt_array(v, WHAT, "tools")?
            .iter()
            .map(tool_from_value)
            .collect::<Result<_>>()?,
        context: opt_array(v, WHAT, "context")?
            .iter()
            .map(context_from_value)
            .collect::<Result<_>>()?,
        forwarded_props: opt_value(v, "forwardedProps").unwrap_or(Value::Null),
    })
}

// ----------------------------------------------------------------- events

fn outcome_to_value(outcome: &RunOutcome) -> Value {
    match outcome {
        RunOutcome::Success => Obj::new().set("type", "success").0,
        RunOutcome::Interrupt(interrupts) => {
            Obj::new()
                .set("type", "interrupt")
                .set("interrupts", Value::Array(interrupts.clone()))
                .0
        }
    }
}

fn outcome_from_value(v: &Value) -> Result<RunOutcome> {
    const WHAT: &str = "run outcome";
    match string(v, WHAT, "type")?.as_str() {
        "success" => Ok(RunOutcome::Success),
        "interrupt" => Ok(RunOutcome::Interrupt(
            opt_array(v, WHAT, "interrupts")?.to_vec(),
        )),
        other => Err(Error::decode(WHAT, format!("unknown outcome {other:?}"))),
    }
}

/// Encodes an event, shared fields included.
pub fn event_to_value(event: &Event) -> Value {
    let meta = &event.meta;
    let obj = Obj::new()
        .set("type", event.kind.type_name())
        .opt("timestamp", meta.timestamp)
        .opt("subagentRunId", meta.subagent_run_id.as_deref())
        .opt("metadata", meta.metadata.clone())
        .opt("rawEvent", meta.raw_event.clone());
    kind_into(obj, &event.kind).0
}

fn kind_into(obj: Obj, kind: &EventKind) -> Obj {
    use EventKind::*;
    match kind {
        RunStarted {
            thread_id,
            run_id,
            parent_run_id,
            input,
        } => obj
            .set("threadId", thread_id.as_str())
            .set("runId", run_id.as_str())
            .opt("parentRunId", parent_run_id.as_deref())
            .opt("input", input.as_ref().map(input_to_value)),
        RunFinished {
            thread_id,
            run_id,
            result,
            outcome,
        } => obj
            .set("threadId", thread_id.as_str())
            .set("runId", run_id.as_str())
            .opt("result", result.clone())
            .opt("outcome", outcome.as_ref().map(outcome_to_value)),
        RunError { message, code } => obj
            .set("message", message.as_str())
            .opt("code", code.as_deref()),
        StepStarted { step_name } | StepFinished { step_name } => {
            obj.set("stepName", step_name.as_str())
        }
        TextMessageStart { message_id, role } => obj
            .set("messageId", message_id.as_str())
            .set("role", role.as_str()),
        TextMessageContent { message_id, delta } => obj
            .set("messageId", message_id.as_str())
            .set("delta", delta.as_str()),
        TextMessageEnd { message_id } => obj.set("messageId", message_id.as_str()),
        TextMessageChunk {
            message_id,
            role,
            delta,
        } => obj
            .opt("messageId", message_id.as_deref())
            .opt("role", role.map(Role::as_str))
            .opt("delta", delta.as_deref()),
        ToolCallStart {
            tool_call_id,
            tool_call_name,
            parent_message_id,
        } => obj
            .set("toolCallId", tool_call_id.as_str())
            .set("toolCallName", tool_call_name.as_str())
            .opt("parentMessageId", parent_message_id.as_deref()),
        ToolCallArgs {
            tool_call_id,
            delta,
        } => obj
            .set("toolCallId", tool_call_id.as_str())
            .set("delta", delta.as_str()),
        ToolCallEnd { tool_call_id } => obj.set("toolCallId", tool_call_id.as_str()),
        ToolCallResult {
            message_id,
            tool_call_id,
            content,
            role,
        } => obj
            .set("messageId", message_id.as_str())
            .set("toolCallId", tool_call_id.as_str())
            .set("content", content.as_str())
            .opt("role", role.map(Role::as_str)),
        ToolCallChunk {
            tool_call_id,
            tool_call_name,
            parent_message_id,
            delta,
        } => obj
            .opt("toolCallId", tool_call_id.as_deref())
            .opt("toolCallName", tool_call_name.as_deref())
            .opt("parentMessageId", parent_message_id.as_deref())
            .opt("delta", delta.as_deref()),
        StateSnapshot { snapshot } => obj.set("snapshot", snapshot.clone()),
        StateDelta { delta } => obj.set("delta", delta.clone()),
        MessagesSnapshot { messages } => obj.set(
            "messages",
            Value::Array(messages.iter().map(message_to_value).collect()),
        ),
        ActivitySnapshot {
            message_id,
            activity_type,
            content,
            replace,
        } => obj
            .set("messageId", message_id.as_str())
            .set("activityType", activity_type.as_str())
            .set("content", content.clone())
            .opt("replace", *replace),
        ActivityDelta {
            message_id,
            activity_type,
            patch,
        } => obj
            .set("messageId", message_id.as_str())
            .set("activityType", activity_type.as_str())
            .set("patch", patch.clone()),
        ReasoningStart { message_id }
        | ReasoningMessageEnd { message_id }
        | ReasoningEnd { message_id } => obj.set("messageId", message_id.as_str()),
        ReasoningMessageStart { message_id } => obj
            .set("messageId", message_id.as_str())
            .set("role", "reasoning"),
        ReasoningMessageContent { message_id, delta } => obj
            .set("messageId", message_id.as_str())
            .set("delta", delta.as_str()),
        ReasoningMessageChunk { message_id, delta } => obj
            .opt("messageId", message_id.as_deref())
            .opt("delta", delta.as_deref()),
        ReasoningEncryptedValue {
            subtype,
            entity_id,
            encrypted_value,
        } => obj
            .set("subtype", subtype.as_str())
            .set("entityId", entity_id.as_str())
            .set("encryptedValue", encrypted_value.as_str()),
        SubagentStarted {
            subagent_run_id,
            name,
            description,
            parent_subagent_run_id,
            parent_tool_call_id,
            parent_message_id,
        } => obj
            .set("subagentRunId", subagent_run_id.as_str())
            .set("name", name.as_str())
            .opt("description", description.as_deref())
            .opt("parentSubagentRunId", parent_subagent_run_id.as_deref())
            .opt("parentToolCallId", parent_tool_call_id.as_deref())
            .opt("parentMessageId", parent_message_id.as_deref()),
        SubagentFinished {
            subagent_run_id,
            result,
            outcome,
        } => obj
            .set("subagentRunId", subagent_run_id.as_str())
            .opt("result", result.clone())
            .opt("outcome", outcome.clone()),
        SubagentError {
            subagent_run_id,
            message,
            code,
        } => obj
            .set("subagentRunId", subagent_run_id.as_str())
            .set("message", message.as_str())
            .opt("code", code.as_deref()),
        Raw { event, source } => obj
            .set("event", event.clone())
            .opt("source", source.as_deref()),
        Custom { name, value } => obj.set("name", name.as_str()).set("value", value.clone()),
    }
}

/// Decodes an event, shared fields included.
pub fn event_from_value(v: &Value) -> Result<Event> {
    const WHAT: &str = "event";
    let meta = EventMeta {
        timestamp: match v.get("timestamp") {
            None | Some(Value::Null) => None,
            Some(t) => Some(
                t.as_f64()
                    .filter(|f| *f >= 0.0)
                    .map(|f| f as u64)
                    .ok_or_else(|| Error::decode(WHAT, "\"timestamp\" is not a number"))?,
            ),
        },
        subagent_run_id: opt_string(v, WHAT, "subagentRunId")?,
        metadata: opt_value(v, "metadata"),
        raw_event: opt_value(v, "rawEvent"),
    };
    Ok(Event {
        kind: kind_from_value(v)?,
        meta,
    })
}

fn kind_from_value(v: &Value) -> Result<EventKind> {
    const WHAT: &str = "event";
    use EventKind::*;
    let kind = string(v, WHAT, "type")?;
    Ok(match kind.as_str() {
        "RUN_STARTED" => RunStarted {
            thread_id: string(v, WHAT, "threadId")?,
            run_id: string(v, WHAT, "runId")?,
            parent_run_id: opt_string(v, WHAT, "parentRunId")?,
            input: opt_value(v, "input")
                .as_ref()
                .map(input_from_value)
                .transpose()?,
        },
        "RUN_FINISHED" => RunFinished {
            thread_id: string(v, WHAT, "threadId")?,
            run_id: string(v, WHAT, "runId")?,
            result: opt_value(v, "result"),
            outcome: opt_value(v, "outcome")
                .as_ref()
                .map(outcome_from_value)
                .transpose()?,
        },
        "RUN_ERROR" => RunError {
            message: string(v, WHAT, "message")?,
            code: opt_string(v, WHAT, "code")?,
        },
        "STEP_STARTED" => StepStarted {
            step_name: string(v, WHAT, "stepName")?,
        },
        "STEP_FINISHED" => StepFinished {
            step_name: string(v, WHAT, "stepName")?,
        },
        "TEXT_MESSAGE_START" => TextMessageStart {
            message_id: string(v, WHAT, "messageId")?,
            role: role(v, WHAT, "role")?,
        },
        "TEXT_MESSAGE_CONTENT" => TextMessageContent {
            message_id: string(v, WHAT, "messageId")?,
            delta: string(v, WHAT, "delta")?,
        },
        "TEXT_MESSAGE_END" => TextMessageEnd {
            message_id: string(v, WHAT, "messageId")?,
        },
        "TEXT_MESSAGE_CHUNK" => TextMessageChunk {
            message_id: opt_string(v, WHAT, "messageId")?,
            role: opt_role(v, WHAT, "role")?,
            delta: opt_string(v, WHAT, "delta")?,
        },
        "TOOL_CALL_START" => ToolCallStart {
            tool_call_id: string(v, WHAT, "toolCallId")?,
            tool_call_name: string(v, WHAT, "toolCallName")?,
            parent_message_id: opt_string(v, WHAT, "parentMessageId")?,
        },
        "TOOL_CALL_ARGS" => ToolCallArgs {
            tool_call_id: string(v, WHAT, "toolCallId")?,
            delta: string(v, WHAT, "delta")?,
        },
        "TOOL_CALL_END" => ToolCallEnd {
            tool_call_id: string(v, WHAT, "toolCallId")?,
        },
        "TOOL_CALL_RESULT" => ToolCallResult {
            message_id: string(v, WHAT, "messageId")?,
            tool_call_id: string(v, WHAT, "toolCallId")?,
            content: string(v, WHAT, "content")?,
            role: opt_role(v, WHAT, "role")?,
        },
        "TOOL_CALL_CHUNK" => ToolCallChunk {
            tool_call_id: opt_string(v, WHAT, "toolCallId")?,
            tool_call_name: opt_string(v, WHAT, "toolCallName")?,
            parent_message_id: opt_string(v, WHAT, "parentMessageId")?,
            delta: opt_string(v, WHAT, "delta")?,
        },
        "STATE_SNAPSHOT" => StateSnapshot {
            snapshot: opt_value(v, "snapshot").unwrap_or(Value::Null),
        },
        "STATE_DELTA" => StateDelta {
            delta: Value::Array(array(v, WHAT, "delta")?.to_vec()),
        },
        "MESSAGES_SNAPSHOT" => MessagesSnapshot {
            messages: messages_from_value(array(v, WHAT, "messages")?)?,
        },
        "ACTIVITY_SNAPSHOT" => ActivitySnapshot {
            message_id: string(v, WHAT, "messageId")?,
            activity_type: string(v, WHAT, "activityType")?,
            content: field(v, WHAT, "content")?.clone(),
            replace: match v.get("replace") {
                None | Some(Value::Null) => None,
                Some(b) => Some(
                    b.as_bool()
                        .ok_or_else(|| Error::decode(WHAT, "\"replace\" is not a boolean"))?,
                ),
            },
        },
        "ACTIVITY_DELTA" => ActivityDelta {
            message_id: string(v, WHAT, "messageId")?,
            activity_type: string(v, WHAT, "activityType")?,
            patch: Value::Array(array(v, WHAT, "patch")?.to_vec()),
        },
        "REASONING_START" => ReasoningStart {
            message_id: string(v, WHAT, "messageId")?,
        },
        "REASONING_MESSAGE_START" => ReasoningMessageStart {
            message_id: string(v, WHAT, "messageId")?,
        },
        "REASONING_MESSAGE_CONTENT" => ReasoningMessageContent {
            message_id: string(v, WHAT, "messageId")?,
            delta: string(v, WHAT, "delta")?,
        },
        "REASONING_MESSAGE_END" => ReasoningMessageEnd {
            message_id: string(v, WHAT, "messageId")?,
        },
        "REASONING_MESSAGE_CHUNK" => ReasoningMessageChunk {
            message_id: opt_string(v, WHAT, "messageId")?,
            delta: opt_string(v, WHAT, "delta")?,
        },
        "REASONING_END" => ReasoningEnd {
            message_id: string(v, WHAT, "messageId")?,
        },
        "REASONING_ENCRYPTED_VALUE" => ReasoningEncryptedValue {
            subtype: string(v, WHAT, "subtype")?,
            entity_id: string(v, WHAT, "entityId")?,
            encrypted_value: string(v, WHAT, "encryptedValue")?,
        },
        "SUBAGENT_STARTED" => SubagentStarted {
            subagent_run_id: string(v, WHAT, "subagentRunId")?,
            name: string(v, WHAT, "name")?,
            description: opt_string(v, WHAT, "description")?,
            parent_subagent_run_id: opt_string(v, WHAT, "parentSubagentRunId")?,
            parent_tool_call_id: opt_string(v, WHAT, "parentToolCallId")?,
            parent_message_id: opt_string(v, WHAT, "parentMessageId")?,
        },
        "SUBAGENT_FINISHED" => SubagentFinished {
            subagent_run_id: string(v, WHAT, "subagentRunId")?,
            result: opt_value(v, "result"),
            outcome: opt_value(v, "outcome"),
        },
        "SUBAGENT_ERROR" => SubagentError {
            subagent_run_id: string(v, WHAT, "subagentRunId")?,
            message: string(v, WHAT, "message")?,
            code: opt_string(v, WHAT, "code")?,
        },
        "RAW" => Raw {
            event: field(v, WHAT, "event")?.clone(),
            source: opt_string(v, WHAT, "source")?,
        },
        "CUSTOM" => Custom {
            name: string(v, WHAT, "name")?,
            value: opt_value(v, "value").unwrap_or(Value::Null),
        },
        other => return Err(Error::decode(WHAT, format!("unknown event type {other:?}"))),
    })
}

// --------------------------------------------------------- inherent sugar

impl Event {
    /// Encodes to a `Value`.
    pub fn to_value(&self) -> Value {
        event_to_value(self)
    }

    /// Decodes from a `Value`.
    pub fn from_value(v: &Value) -> Result<Self> {
        event_from_value(v)
    }

    /// Encodes to compact JSON text.
    pub fn to_json(&self) -> String {
        self.to_value().to_json_string()
    }

    /// Decodes from JSON text.
    pub fn from_json(text: &str) -> Result<Self> {
        event_from_value(&Value::parse(text)?)
    }
}

impl Message {
    /// Encodes to a `Value`.
    pub fn to_value(&self) -> Value {
        message_to_value(self)
    }

    /// Decodes from a `Value`.
    pub fn from_value(v: &Value) -> Result<Self> {
        message_from_value(v)
    }
}

impl RunAgentInput {
    /// Encodes to a `Value`.
    pub fn to_value(&self) -> Value {
        input_to_value(self)
    }

    /// Decodes from a `Value`.
    pub fn from_value(v: &Value) -> Result<Self> {
        input_from_value(v)
    }

    /// Decodes from JSON text (a request body).
    pub fn from_json(text: &str) -> Result<Self> {
        input_from_value(&Value::parse(text)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_json::json;

    fn round_trip(wire: Value) {
        let event = Event::from_value(&wire).unwrap();
        assert_eq!(event.to_value(), wire, "{}", wire.to_json_string());
        assert_eq!(Event::from_json(&event.to_json()).unwrap(), event);
    }

    /// One wire sample per event type, as the TypeScript SDK emits them.
    /// The fixture is shared with the TypeScript core's tests.
    #[test]
    fn every_event_type_round_trips() {
        let samples = Value::parse(include_str!("../fixtures/events.json")).unwrap();
        let samples = samples.as_array().unwrap();
        assert!(
            samples.len() >= 35,
            "one sample per event type, plus variants"
        );
        for sample in samples {
            round_trip(sample.clone());
        }
    }

    #[test]
    fn decode_errors_name_the_field() {
        let err = Event::from_value(&json!({"type": "TEXT_MESSAGE_CONTENT", "messageId": "m"}))
            .unwrap_err();
        assert_eq!(
            err,
            Error::Decode {
                what: "event",
                reason: "missing \"delta\"".into()
            }
        );
        assert!(matches!(
            Event::from_value(&json!({"type": "NOPE"})),
            Err(Error::Decode { .. })
        ));
        assert!(matches!(Event::from_json("{"), Err(Error::Json(_))));
        assert!(matches!(
            message_from_value(&json!({"id": "1", "role": "wizard", "content": ""})),
            Err(Error::Decode { .. })
        ));
    }

    #[test]
    fn unknown_members_are_ignored_and_sparse_input_fills_defaults() {
        let event =
            Event::from_value(&json!({"type": "TEXT_MESSAGE_END", "messageId": "m", "future": 1}))
                .unwrap();
        assert_eq!(
            event.kind,
            EventKind::TextMessageEnd {
                message_id: "m".into()
            }
        );

        let input = RunAgentInput::from_json(r#"{"threadId":"t","runId":"r"}"#).unwrap();
        assert_eq!(input, RunAgentInput::new("t", "r", vec![]));
    }

    #[test]
    fn content_text_concatenates_parts() {
        let content = Content::Parts(vec![
            ContentPart::Text("a".into()),
            ContentPart::Binary {
                mime_type: "image/png".into(),
                data: None,
                url: None,
                filename: None,
            },
            ContentPart::Text("b".into()),
        ]);
        assert_eq!(content.text(), "ab");
    }
}
