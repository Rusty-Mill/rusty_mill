//! The event types. Field names are the wire names in snake case.

use crate::types::{Message, Role, RunAgentInput};
use rusty_json::Value;

/// How a run or subagent ended, when `RUN_FINISHED` says.
#[derive(Clone, Debug, PartialEq)]
pub enum RunOutcome {
    /// Ran to completion.
    Success,
    /// Paused for input; `interrupts` are the pending interrupt payloads.
    Interrupt(Vec<Value>),
}

/// Fields every event may carry besides its own.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EventMeta {
    /// Milliseconds since the epoch.
    pub timestamp: Option<u64>,
    /// The subagent that produced this event.
    pub subagent_run_id: Option<String>,
    /// Extra information (usage, trace ids).
    pub metadata: Option<Value>,
    /// The original event, when this one was transformed from it.
    pub raw_event: Option<Value>,
}

/// One AG-UI event: its kind plus the shared optional fields.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    /// The event proper.
    pub kind: EventKind,
    /// Shared optional fields.
    pub meta: EventMeta,
}

impl From<EventKind> for Event {
    fn from(kind: EventKind) -> Self {
        Event {
            kind,
            meta: EventMeta::default(),
        }
    }
}

/// The 31 event types, grouped as the specification groups them.
#[derive(Clone, Debug, PartialEq)]
pub enum EventKind {
    // Lifecycle.
    /// Opens a run.
    RunStarted {
        /// Thread id.
        thread_id: String,
        /// Run id.
        run_id: String,
        /// The run this branches from.
        parent_run_id: Option<String>,
        /// The input, echoed.
        input: Option<RunAgentInput>,
    },
    /// Closes a run normally.
    RunFinished {
        /// Thread id.
        thread_id: String,
        /// Run id.
        run_id: String,
        /// The run's result.
        result: Option<Value>,
        /// Success or interrupt.
        outcome: Option<RunOutcome>,
    },
    /// Closes a run with an error.
    RunError {
        /// What went wrong.
        message: String,
        /// Machine-readable code.
        code: Option<String>,
    },
    /// Opens a named step.
    StepStarted {
        /// Step name.
        step_name: String,
    },
    /// Closes a named step.
    StepFinished {
        /// Step name.
        step_name: String,
    },

    // Text messages.
    /// Opens a streamed message.
    TextMessageStart {
        /// Message id.
        message_id: String,
        /// Author role.
        role: Role,
    },
    /// Appends text; `delta` is non-empty.
    TextMessageContent {
        /// Message id.
        message_id: String,
        /// Text to append.
        delta: String,
    },
    /// Closes a streamed message.
    TextMessageEnd {
        /// Message id.
        message_id: String,
    },
    /// A convenience form the verifier expands to start/content/end.
    TextMessageChunk {
        /// Message id; required on the first chunk.
        message_id: Option<String>,
        /// Role; defaults to assistant.
        role: Option<Role>,
        /// Text to append.
        delta: Option<String>,
    },

    // Tool calls.
    /// Opens a streamed tool call.
    ToolCallStart {
        /// Tool call id.
        tool_call_id: String,
        /// Tool name.
        tool_call_name: String,
        /// The assistant message this call belongs to.
        parent_message_id: Option<String>,
    },
    /// Appends argument text.
    ToolCallArgs {
        /// Tool call id.
        tool_call_id: String,
        /// Argument JSON fragment.
        delta: String,
    },
    /// Closes a streamed tool call.
    ToolCallEnd {
        /// Tool call id.
        tool_call_id: String,
    },
    /// The result of a tool call, as a tool message.
    ToolCallResult {
        /// The tool message's id.
        message_id: String,
        /// The call this answers.
        tool_call_id: String,
        /// The result text.
        content: String,
        /// Always `tool` when present.
        role: Option<Role>,
    },
    /// A convenience form the verifier expands to start/args/end.
    ToolCallChunk {
        /// Tool call id; required on the first chunk.
        tool_call_id: Option<String>,
        /// Tool name; required on the first chunk.
        tool_call_name: Option<String>,
        /// Parent assistant message.
        parent_message_id: Option<String>,
        /// Argument fragment.
        delta: Option<String>,
    },

    // State.
    /// Replaces the shared state.
    StateSnapshot {
        /// The whole state.
        snapshot: Value,
    },
    /// Patches the shared state (RFC 6902).
    StateDelta {
        /// The patch operations.
        delta: Value,
    },
    /// Replaces the message list.
    MessagesSnapshot {
        /// All messages.
        messages: Vec<Message>,
    },
    /// Replaces (or merges into) an activity message.
    ActivitySnapshot {
        /// Activity message id.
        message_id: String,
        /// Discriminator.
        activity_type: String,
        /// Structured content.
        content: Value,
        /// Replace (default) rather than merge.
        replace: Option<bool>,
    },
    /// Patches an activity message's content (RFC 6902).
    ActivityDelta {
        /// Activity message id.
        message_id: String,
        /// Discriminator.
        activity_type: String,
        /// The patch operations.
        patch: Value,
    },

    // Reasoning.
    /// Opens a reasoning phase.
    ReasoningStart {
        /// Reasoning message id.
        message_id: String,
    },
    /// Opens a reasoning message.
    ReasoningMessageStart {
        /// Reasoning message id.
        message_id: String,
    },
    /// Appends reasoning text.
    ReasoningMessageContent {
        /// Reasoning message id.
        message_id: String,
        /// Text to append.
        delta: String,
    },
    /// Closes a reasoning message.
    ReasoningMessageEnd {
        /// Reasoning message id.
        message_id: String,
    },
    /// A convenience form the verifier expands.
    ReasoningMessageChunk {
        /// Reasoning message id; required on the first chunk.
        message_id: Option<String>,
        /// Text; an empty string closes the message.
        delta: Option<String>,
    },
    /// Closes a reasoning phase.
    ReasoningEnd {
        /// Reasoning message id.
        message_id: String,
    },
    /// Provider-encrypted reasoning to replay later.
    ReasoningEncryptedValue {
        /// `message` or `tool-call`.
        subtype: String,
        /// The message or tool call id.
        entity_id: String,
        /// The blob.
        encrypted_value: String,
    },

    // Subagents.
    /// A child agent began.
    SubagentStarted {
        /// Its run id, carried by its events' `subagentRunId`.
        subagent_run_id: String,
        /// Display name.
        name: String,
        /// Description.
        description: Option<String>,
        /// Enclosing subagent.
        parent_subagent_run_id: Option<String>,
        /// Spawning tool call.
        parent_tool_call_id: Option<String>,
        /// Spawning message.
        parent_message_id: Option<String>,
    },
    /// A child agent finished.
    SubagentFinished {
        /// Its run id.
        subagent_run_id: String,
        /// Its result.
        result: Option<Value>,
        /// Its outcome object, verbatim.
        outcome: Option<Value>,
    },
    /// A child agent failed.
    SubagentError {
        /// Its run id.
        subagent_run_id: String,
        /// What went wrong.
        message: String,
        /// Machine-readable code.
        code: Option<String>,
    },

    // Special.
    /// An event from an external system, passed through.
    Raw {
        /// The event.
        event: Value,
        /// Where it came from.
        source: Option<String>,
    },
    /// An application-defined event.
    Custom {
        /// Event name.
        name: String,
        /// Payload.
        value: Value,
    },
}

impl EventKind {
    /// The wire `type` string.
    pub fn type_name(&self) -> &'static str {
        use EventKind::*;
        match self {
            RunStarted { .. } => "RUN_STARTED",
            RunFinished { .. } => "RUN_FINISHED",
            RunError { .. } => "RUN_ERROR",
            StepStarted { .. } => "STEP_STARTED",
            StepFinished { .. } => "STEP_FINISHED",
            TextMessageStart { .. } => "TEXT_MESSAGE_START",
            TextMessageContent { .. } => "TEXT_MESSAGE_CONTENT",
            TextMessageEnd { .. } => "TEXT_MESSAGE_END",
            TextMessageChunk { .. } => "TEXT_MESSAGE_CHUNK",
            ToolCallStart { .. } => "TOOL_CALL_START",
            ToolCallArgs { .. } => "TOOL_CALL_ARGS",
            ToolCallEnd { .. } => "TOOL_CALL_END",
            ToolCallResult { .. } => "TOOL_CALL_RESULT",
            ToolCallChunk { .. } => "TOOL_CALL_CHUNK",
            StateSnapshot { .. } => "STATE_SNAPSHOT",
            StateDelta { .. } => "STATE_DELTA",
            MessagesSnapshot { .. } => "MESSAGES_SNAPSHOT",
            ActivitySnapshot { .. } => "ACTIVITY_SNAPSHOT",
            ActivityDelta { .. } => "ACTIVITY_DELTA",
            ReasoningStart { .. } => "REASONING_START",
            ReasoningMessageStart { .. } => "REASONING_MESSAGE_START",
            ReasoningMessageContent { .. } => "REASONING_MESSAGE_CONTENT",
            ReasoningMessageEnd { .. } => "REASONING_MESSAGE_END",
            ReasoningMessageChunk { .. } => "REASONING_MESSAGE_CHUNK",
            ReasoningEnd { .. } => "REASONING_END",
            ReasoningEncryptedValue { .. } => "REASONING_ENCRYPTED_VALUE",
            SubagentStarted { .. } => "SUBAGENT_STARTED",
            SubagentFinished { .. } => "SUBAGENT_FINISHED",
            SubagentError { .. } => "SUBAGENT_ERROR",
            Raw { .. } => "RAW",
            Custom { .. } => "CUSTOM",
        }
    }
}
