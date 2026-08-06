//! The [`Event`] type — the single unit of communication in the ADK runtime.
//!
//! Everything an agent, tool, or graph node wants to report travels as an
//! `Event`. The [`Runner`](../../adk_runner/index.html) consumes each one,
//! commits its [`EventActions`] through the session service, and forwards it
//! to the caller.
//!
//! # ADK 2.0
//!
//! 2.0 adds [`Event::node_info`] and [`Event::output`] to track graph state
//! and workflow outputs. Both are represented here.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

use crate::content::{Content, FunctionCall, FunctionResponse, Part};

/// Identifies the workflow-graph node that emitted an event.
///
/// # Compatibility note
///
/// ADK 2.0 documents that `node_info` exists on the Event schema and that
/// custom session stores with rigid columns must be widened to hold it, but
/// the published docs do not specify its internal field layout. The shape
/// below is this implementation's own, chosen to carry the information the
/// graph engine actually needs. It serializes as a JSON object, so stores
/// that keep events as serialized JSON — the case ADK says needs no migration
/// — round-trip it unchanged.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NodeInfo {
    /// Name of the emitting node, unique within its graph.
    pub name: String,
    /// Node category, e.g. `function`, `agent`, `join`, `emitting_function`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_type: Option<String>,
    /// Zero-based execution step within the current invocation.
    ///
    /// A node revisited by a loop reports an increasing index, which makes an
    /// event trace readable without reconstructing the traversal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<u64>,
    /// Name of the node whose output fed this node, when there was one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predecessor: Option<String>,
}

impl NodeInfo {
    /// Builds node info for a named node.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }

    /// Sets the node category.
    pub fn with_type(mut self, node_type: impl Into<String>) -> Self {
        self.node_type = Some(node_type.into());
        self
    }

    /// Sets the execution step index.
    pub fn with_step(mut self, step: u64) -> Self {
        self.step = Some(step);
        self
    }

    /// Sets the predecessor node name.
    pub fn with_predecessor(mut self, predecessor: impl Into<String>) -> Self {
        self.predecessor = Some(predecessor.into());
        self
    }
}

/// A pending request for human input that suspends a workflow.
///
/// Emitted by a node that needs a person to act before the graph can continue.
/// The runtime persists it, ends the run, and resumes from the same point when
/// the caller supplies a matching response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestInput {
    /// Correlates the suspension with the response that lifts it.
    pub interrupt_id: String,
    /// Prompt to show the human.
    pub hint: String,
    /// Optional structured data for the client to render or pre-fill.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
}

impl RequestInput {
    /// Builds a request with a freshly generated interrupt id.
    pub fn new(hint: impl Into<String>) -> Self {
        Self {
            interrupt_id: crate::new_id("interrupt"),
            hint: hint.into(),
            payload: None,
        }
    }

    /// Attaches a structured payload.
    pub fn with_payload(mut self, payload: Value) -> Self {
        self.payload = Some(payload);
        self
    }
}

/// A tool's request for user approval before it runs.
///
/// Answered by a [`Part::FunctionResponse`] named
/// [`crate::TOOL_CONFIRMATION_FUNCTION_NAME`], whose `id` matches the
/// originating function call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolConfirmation {
    /// Prompt explaining what is about to happen.
    pub hint: String,
    /// Whether the user approved. `false` until a response arrives.
    #[serde(default)]
    pub confirmed: bool,
    /// Structured data the client fills in alongside the yes/no answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
}

impl ToolConfirmation {
    /// Builds an unconfirmed request.
    pub fn new(hint: impl Into<String>) -> Self {
        Self {
            hint: hint.into(),
            confirmed: false,
            payload: None,
        }
    }

    /// Attaches a structured payload.
    pub fn with_payload(mut self, payload: Value) -> Self {
        self.payload = Some(payload);
        self
    }
}

/// Side effects and control-flow directives carried by an [`Event`].
///
/// The runner applies these when it processes the event; nothing here takes
/// effect until then.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EventActions {
    /// State keys to merge into the session, honouring `app:` / `user:` /
    /// `temp:` prefixes. See [`crate::state`].
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub state_delta: BTreeMap<String, Value>,

    /// Artifact filenames mapped to the version produced by this event.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub artifact_delta: BTreeMap<String, u64>,

    /// Hand control to the named agent instead of continuing here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transfer_to_agent: Option<String>,

    /// Terminate the enclosing loop, or escalate to the parent agent.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub escalate: bool,

    /// Return the tool result to the caller verbatim, without asking the model
    /// to summarize it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub skip_summarization: bool,

    /// Auth configurations the tool needs credentials for, keyed by function
    /// call id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub requested_auth_configs: BTreeMap<String, Value>,

    /// Confirmations the tool is waiting on, keyed by function call id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub requested_tool_confirmations: BTreeMap<String, ToolConfirmation>,
}

impl EventActions {
    /// Records a state change.
    pub fn set_state(&mut self, key: impl Into<String>, value: impl Into<Value>) -> &mut Self {
        self.state_delta.insert(key.into(), value.into());
        self
    }

    /// True when this carries no side effects at all.
    pub fn is_empty(&self) -> bool {
        self == &EventActions::default()
    }
}

/// The unit of communication between agents, tools, graph nodes, and the runner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// Unique id for this event instance, assigned at construction.
    pub id: String,

    /// Groups every event produced while handling one user request.
    pub invocation_id: String,

    /// Who produced this — `user`, or an agent/node name.
    pub author: String,

    /// The conversational payload, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Content>,

    /// Side effects and control-flow directives.
    #[serde(default, skip_serializing_if = "EventActions::is_empty")]
    pub actions: EventActions,

    /// `true` for an incomplete streaming chunk.
    ///
    /// The runner forwards partial events but does not commit their actions —
    /// state is applied once, from the final aggregated event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partial: Option<bool>,

    /// `true` on the event that closes a conversational turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_complete: Option<bool>,

    /// Dot-separated path through the agent hierarchy, e.g. `root.researcher`.
    ///
    /// Parallel branches get distinct values so their histories stay separable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,

    /// Creation time in seconds since the Unix epoch.
    pub timestamp: f64,

    /// Machine-readable error classification, when the event reports a failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,

    /// Human-readable error detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,

    /// Ids of function calls that run in the background rather than blocking.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub long_running_tool_ids: BTreeSet<String>,

    // ---- ADK 2.0 additions ----
    /// Which graph node emitted this event. `None` outside a graph run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_info: Option<NodeInfo>,

    /// The node's typed output, passed to its successor as that node's input.
    ///
    /// A node may emit at most one output payload per execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,

    /// Route labels this node emitted, matched against outgoing edges.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<String>,

    /// Present when this event suspends the workflow pending human input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_input: Option<RequestInput>,
}

impl Event {
    /// Builds an empty event attributed to `author` within `invocation_id`.
    pub fn new(invocation_id: impl Into<String>, author: impl Into<String>) -> Self {
        Self {
            id: crate::new_id("evt"),
            invocation_id: invocation_id.into(),
            author: author.into(),
            content: None,
            actions: EventActions::default(),
            partial: None,
            turn_complete: None,
            branch: None,
            timestamp: crate::now_seconds(),
            error_code: None,
            error_message: None,
            long_running_tool_ids: BTreeSet::new(),
            node_info: None,
            output: None,
            routes: Vec::new(),
            request_input: None,
        }
    }

    /// Attaches content.
    pub fn with_content(mut self, content: Content) -> Self {
        self.content = Some(content);
        self
    }

    /// Attaches a single text part authored by the model.
    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.content = Some(Content::model_text(text));
        self
    }

    /// Attaches the node's typed output.
    pub fn with_output(mut self, output: impl Into<Value>) -> Self {
        self.output = Some(output.into());
        self
    }

    /// Attaches route labels for edge matching.
    pub fn with_routes<I, S>(mut self, routes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.routes = routes.into_iter().map(Into::into).collect();
        self
    }

    /// Attaches graph node metadata.
    pub fn with_node_info(mut self, node_info: NodeInfo) -> Self {
        self.node_info = Some(node_info);
        self
    }

    /// Replaces the actions block.
    pub fn with_actions(mut self, actions: EventActions) -> Self {
        self.actions = actions;
        self
    }

    /// Sets the branch path.
    pub fn with_branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = Some(branch.into());
        self
    }

    /// Marks this as a partial streaming chunk.
    pub fn as_partial(mut self) -> Self {
        self.partial = Some(true);
        self
    }

    /// Records an error on this event.
    pub fn with_error(
        mut self,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        self.error_code = Some(code.into());
        self.error_message = Some(message.into());
        self
    }

    /// Marks this event as suspending the run pending human input.
    pub fn with_request_input(mut self, request: RequestInput) -> Self {
        self.request_input = Some(request);
        self
    }

    /// Function calls carried by this event.
    pub fn function_calls(&self) -> Vec<&FunctionCall> {
        self.content
            .as_ref()
            .map(Content::function_calls)
            .unwrap_or_default()
    }

    /// Function responses carried by this event.
    pub fn function_responses(&self) -> Vec<&FunctionResponse> {
        self.content
            .as_ref()
            .map(Content::function_responses)
            .unwrap_or_default()
    }

    /// True when this is a streaming chunk rather than a complete event.
    pub fn is_partial(&self) -> bool {
        self.partial.unwrap_or(false)
    }

    /// Whether this event should be surfaced to the user as a final answer.
    ///
    /// Follows ADK's `is_final_response` rules: a tool result marked
    /// `skip_summarization`, or a call to a long-running tool, is final because
    /// nothing further will be generated for it; otherwise an event is final
    /// only when it is complete and carries no pending tool traffic.
    pub fn is_final_response(&self) -> bool {
        if self.actions.skip_summarization && !self.function_responses().is_empty() {
            return true;
        }
        if !self.long_running_tool_ids.is_empty() {
            return true;
        }
        !self.is_partial()
            && self.function_calls().is_empty()
            && self.function_responses().is_empty()
            && self.request_input.is_none()
    }

    /// Concatenated text of this event's content, or an empty string.
    pub fn text(&self) -> String {
        self.content.as_ref().map(Content::text).unwrap_or_default()
    }

    /// Builds the event that reports a tool's result back to the model.
    pub fn tool_response(
        invocation_id: impl Into<String>,
        author: impl Into<String>,
        response: FunctionResponse,
    ) -> Self {
        Event::new(invocation_id, author).with_content(Content::new(
            crate::content::Role::User,
            vec![Part::FunctionResponse(response)],
        ))
    }
}

/// Convenience alias for the argument maps carried by function calls.
pub type Args = Map<String, Value>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::Role;
    use serde_json::json;

    fn call_event() -> Event {
        Event::new("inv", "agent").with_content(Content::new(
            Role::Model,
            vec![Part::FunctionCall(FunctionCall::new("t", Args::new()))],
        ))
    }

    #[test]
    fn plain_text_event_is_final() {
        assert!(Event::new("inv", "agent").with_text("done").is_final_response());
    }

    #[test]
    fn pending_function_call_is_not_final() {
        assert!(!call_event().is_final_response());
    }

    #[test]
    fn long_running_call_is_final_despite_pending_call() {
        let mut ev = call_event();
        ev.long_running_tool_ids.insert("call-1".into());
        assert!(ev.is_final_response());
    }

    #[test]
    fn skip_summarization_response_is_final() {
        let mut ev = Event::tool_response(
            "inv",
            "agent",
            FunctionResponse {
                id: Some("c1".into()),
                name: "t".into(),
                response: json!({"status": "success"}),
            },
        );
        assert!(!ev.is_final_response());
        ev.actions.skip_summarization = true;
        assert!(ev.is_final_response());
    }

    #[test]
    fn partial_chunk_is_not_final() {
        assert!(!Event::new("inv", "agent").with_text("Par").as_partial().is_final_response());
    }

    #[test]
    fn interrupt_event_is_not_final() {
        let ev = Event::new("inv", "node").with_request_input(RequestInput::new("approve?"));
        assert!(!ev.is_final_response());
    }

    #[test]
    fn empty_actions_are_omitted_from_json() {
        let ev = Event::new("inv", "agent").with_text("hi");
        let v = serde_json::to_value(&ev).unwrap();
        assert!(v.get("actions").is_none());
        assert!(v.get("node_info").is_none());
        assert!(v.get("output").is_none());
    }

    #[test]
    fn node_info_and_output_round_trip() {
        let ev = Event::new("inv", "node_a")
            .with_node_info(NodeInfo::new("node_a").with_type("function").with_step(3))
            .with_output(json!({"city": "Tokyo"}))
            .with_routes(["BUG"]);
        let round: Event = serde_json::from_str(&serde_json::to_string(&ev).unwrap()).unwrap();
        assert_eq!(round, ev);
        assert_eq!(round.node_info.unwrap().step, Some(3));
    }
}
