//! The [`Agent`] trait and its bridge into the workflow graph.
//!
//! # "Agents are nodes"
//!
//! ADK 2.0 made `BaseAgent` a subclass of `BaseNode` so that agents are
//! evaluated inside the workflow graph. Rust cannot express that as trait
//! inheritance here, because the two have genuinely different execution
//! shapes: an agent streams [`Event`]s, while a node returns a single
//! [`adk_graph::NodeOutcome`]. Instead, [`AgentNode`] adapts any
//! agent into a [`Node`], which gives the same composition — agents, tools,
//! and functions all appearing as nodes in one graph — without pretending the
//! two signatures are the same.

use adk_core::{Content, Event, InvocationContext, Result};
use adk_graph::{Node, NodeConfig, NodeContext, NodeOutcome};
use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;
use serde_json::Value;
use std::sync::Arc;

/// Something that can handle a turn and stream events back.
#[async_trait]
pub trait Agent: Send + Sync {
    /// The agent's name, unique among its siblings.
    fn name(&self) -> &str;

    /// What this agent is for.
    ///
    /// Other agents read this when deciding whether to delegate, so it should
    /// describe capability rather than implementation.
    fn description(&self) -> &str {
        ""
    }

    /// Agents this one may delegate to.
    fn sub_agents(&self) -> &[SharedAgent] {
        &[]
    }

    /// Runs the agent, yielding events as they are produced.
    fn run<'a>(&'a self, ctx: &'a InvocationContext) -> BoxStream<'a, Result<Event>>;

    /// Finds an agent by name in this subtree, including this agent.
    fn find_agent(&self, name: &str) -> Option<SharedAgent> {
        for sub in self.sub_agents() {
            if sub.name() == name {
                return Some(Arc::clone(sub));
            }
            if let Some(found) = sub.find_agent(name) {
                return Some(found);
            }
        }
        None
    }
}

/// An agent behind shared ownership.
pub type SharedAgent = Arc<dyn Agent>;

/// Adapts an [`Agent`] into a graph [`Node`].
///
/// The agent's events are forwarded to the graph's event stream; its final
/// response text becomes the node's output, so a downstream node receives what
/// the agent concluded rather than its whole transcript.
pub struct AgentNode {
    agent: SharedAgent,
    config: NodeConfig,
    /// When set, the node's output is this JSON shape instead of plain text.
    structured: bool,
}

impl AgentNode {
    /// Wraps an agent as a node.
    pub fn new(agent: SharedAgent) -> Self {
        Self {
            agent,
            config: NodeConfig::default(),
            structured: false,
        }
    }

    /// Sets the node configuration.
    pub fn with_config(mut self, config: NodeConfig) -> Self {
        self.config = config;
        self
    }

    /// Parses the agent's final response as JSON for the node's output.
    ///
    /// Use this when the agent is configured with an output schema and the
    /// successor expects a typed value rather than a string.
    pub fn structured(mut self) -> Self {
        self.structured = true;
        self
    }

    /// Wraps this node for registration with a graph.
    pub fn shared(self) -> Arc<dyn Node> {
        Arc::new(self)
    }
}

#[async_trait]
impl Node for AgentNode {
    fn name(&self) -> &str {
        self.agent.name()
    }

    fn node_type(&self) -> &str {
        "agent"
    }

    fn config(&self) -> &NodeConfig {
        &self.config
    }

    async fn run(&self, ctx: &NodeContext) -> Result<NodeOutcome> {
        // A node's input is the predecessor's output; deliver it to the agent
        // as the user turn so the agent sees what the graph handed it.
        if let Some(input) = &ctx.input {
            let text = match input {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            ctx.invocation.with_session_mut(|session| {
                session.events.push(
                    Event::new(&ctx.invocation.invocation_id, "user")
                        .with_content(Content::user_text(text)),
                );
            });
        }

        let mut final_text = String::new();
        let mut stream = self.agent.run(&ctx.invocation);
        while let Some(event) = stream.next().await {
            let event = event?;
            if event.is_final_response() {
                let text = event.text();
                if !text.is_empty() {
                    final_text = text;
                }
            }
            ctx.emit(event)?;
        }

        let output = if self.structured {
            serde_json::from_str::<Value>(final_text.trim()).unwrap_or(Value::String(final_text))
        } else {
            Value::String(final_text)
        };
        Ok(NodeOutcome::output(output))
    }
}

/// Extension trait for turning an agent into a graph node.
pub trait IntoAgentNode {
    /// Wraps this agent as a graph node.
    fn into_node(self) -> AgentNode;
}

impl<A: Agent + 'static> IntoAgentNode for A {
    fn into_node(self) -> AgentNode {
        AgentNode::new(Arc::new(self))
    }
}
