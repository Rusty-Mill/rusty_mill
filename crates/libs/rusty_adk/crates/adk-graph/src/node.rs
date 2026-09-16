//! The [`Node`] trait — the unit of execution in an ADK 2.0 workflow graph.
//!
//! In ADK 2.0 everything is a node: agents, tools, and plain functions are all
//! evaluated as nodes within the graph engine.

use adk_core::{Event, InvocationContext, NodeInfo, RequestInput, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::mpsc;

/// The sentinel name of the graph's entry point.
pub const START: &str = "__start__";

/// Per-node execution settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    /// How many times to retry the node after a retryable failure.
    ///
    /// Control-flow signals — an interrupt or a confirmation request — are
    /// never retried; see [`adk_core::AdkError::is_control_flow`].
    #[serde(default)]
    pub max_retries: u32,

    /// Whether a resumed run re-executes this node from the beginning.
    ///
    /// ADK's Go engine re-runs an interrupted node on resume so that the code
    /// before the suspension point runs again with the human's answer in hand.
    /// That is the default here too.
    #[serde(default = "default_true")]
    pub rerun_on_resume: bool,

    /// Human-readable description, surfaced in traces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

fn default_true() -> bool {
    true
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            max_retries: 0,
            rerun_on_resume: true,
            description: None,
        }
    }
}

impl NodeConfig {
    /// Sets the retry budget.
    pub fn with_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// Sets the description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

/// What a node produced.
#[derive(Debug, Clone, Default)]
pub struct NodeOutcome {
    /// The node's typed output, delivered to its successor as that node's input.
    ///
    /// A node may emit at most one output payload per execution.
    pub output: Option<Value>,

    /// Route labels, matched against this node's outgoing edges.
    pub routes: Vec<String>,
}

impl NodeOutcome {
    /// An outcome carrying no output and no routes.
    pub fn empty() -> Self {
        Self::default()
    }

    /// An outcome carrying an output payload.
    pub fn output(value: impl Into<Value>) -> Self {
        Self {
            output: Some(value.into()),
            routes: Vec::new(),
        }
    }

    /// Adds route labels for edge matching.
    pub fn with_routes<I, S>(mut self, routes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.routes = routes.into_iter().map(Into::into).collect();
        self
    }
}

/// What a node sees while it runs.
#[derive(Clone)]
pub struct NodeContext {
    /// The enclosing invocation.
    pub invocation: InvocationContext,

    /// The predecessor's output, or `None` at the graph's entry point.
    pub input: Option<Value>,

    /// Name of the node whose output produced [`NodeContext::input`].
    pub predecessor: Option<String>,

    /// Zero-based execution step within this invocation.
    pub step: u64,

    /// The human's answer, on a run resumed from an interrupt.
    ///
    /// `None` on a first execution. A node that suspends should check this
    /// before suspending again.
    pub resume_payload: Option<Value>,

    node_name: String,
    emitter: mpsc::UnboundedSender<Event>,
}

impl NodeContext {
    /// Builds a context for `node_name`, emitting into `emitter`.
    pub fn new(
        invocation: InvocationContext,
        node_name: impl Into<String>,
        emitter: mpsc::UnboundedSender<Event>,
    ) -> Self {
        Self {
            invocation,
            input: None,
            predecessor: None,
            step: 0,
            resume_payload: None,
            node_name: node_name.into(),
            emitter,
        }
    }

    /// Sets the input payload.
    pub fn with_input(mut self, input: Option<Value>) -> Self {
        self.input = input;
        self
    }

    /// Sets the predecessor name.
    pub fn with_predecessor(mut self, predecessor: Option<String>) -> Self {
        self.predecessor = predecessor;
        self
    }

    /// Sets the execution step index.
    pub fn with_step(mut self, step: u64) -> Self {
        self.step = step;
        self
    }

    /// Sets the resume payload.
    pub fn with_resume_payload(mut self, payload: Option<Value>) -> Self {
        self.resume_payload = payload;
        self
    }

    /// The executing node's name.
    pub fn node_name(&self) -> &str {
        &self.node_name
    }

    /// Deserializes the input into `T`.
    ///
    /// Returns `Ok(None)` when there is no input, and an error when the input
    /// does not fit `T` — a mismatch between a node's declared input type and
    /// its predecessor's output is a graph wiring bug worth surfacing.
    pub fn typed_input<T: for<'de> Deserialize<'de>>(&self) -> Result<Option<T>> {
        match &self.input {
            None => Ok(None),
            Some(value) => serde_json::from_value(value.clone())
                .map(Some)
                .map_err(|e| {
                    adk_core::AdkError::validation(
                        format!("{}.input", self.node_name),
                        format!("predecessor output does not match this node's input type: {e}"),
                    )
                }),
        }
    }

    /// Builds node metadata for an event emitted by this node.
    pub fn node_info(&self, node_type: &str) -> NodeInfo {
        let mut info = NodeInfo::new(&self.node_name)
            .with_type(node_type)
            .with_step(self.step);
        if let Some(predecessor) = &self.predecessor {
            info = info.with_predecessor(predecessor);
        }
        info
    }

    /// Emits an intermediate event without ending the node's execution.
    ///
    /// Use this for user-facing progress messages; the node's *data* output
    /// travels via [`NodeOutcome::output`] instead.
    pub fn emit(&self, event: Event) -> Result<()> {
        self.emitter
            .send(event)
            .map_err(|_| adk_core::AdkError::Cancelled)
    }

    /// Emits a user-facing text message from this node.
    pub fn emit_message(&self, text: impl Into<String>) -> Result<()> {
        let event = Event::new(&self.invocation.invocation_id, &self.node_name)
            .with_text(text)
            .with_node_info(self.node_info("function"));
        self.emit(event)
    }

    /// Suspends the workflow to ask a human for input, or returns their answer.
    ///
    /// On the first execution this emits a [`RequestInput`] event and returns
    /// [`adk_core::AdkError::NodeInterrupted`], which the engine turns into a
    /// clean suspension. When the run is resumed the node executes again and
    /// this returns the supplied payload, so the code around it need not be
    /// restructured into a state machine.
    pub fn resume_or_request_input(
        &self,
        hint: impl Into<String>,
        payload: Option<Value>,
    ) -> Result<Value> {
        if let Some(answer) = &self.resume_payload {
            return Ok(answer.clone());
        }

        let mut request = RequestInput::new(hint);
        request.payload = payload;
        let event = Event::new(&self.invocation.invocation_id, &self.node_name)
            .with_node_info(self.node_info("function"))
            .with_request_input(request.clone());
        self.emit(event)?;

        Err(adk_core::AdkError::NodeInterrupted {
            interrupt_id: request.interrupt_id,
        })
    }
}

impl std::fmt::Debug for NodeContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeContext")
            .field("node", &self.node_name)
            .field("step", &self.step)
            .field("predecessor", &self.predecessor)
            .field("has_input", &self.input.is_some())
            .field("resuming", &self.resume_payload.is_some())
            .finish()
    }
}

/// A unit of execution in a workflow graph.
#[async_trait]
pub trait Node: Send + Sync {
    /// The node's name, unique within its graph.
    fn name(&self) -> &str;

    /// The node's category, recorded in [`NodeInfo::node_type`].
    fn node_type(&self) -> &str {
        "function"
    }

    /// Execution settings.
    fn config(&self) -> &NodeConfig;

    /// Runs the node.
    async fn run(&self, ctx: &NodeContext) -> Result<NodeOutcome>;
}

/// A node behind shared ownership, as graphs hold them.
pub type SharedNode = Arc<dyn Node>;
