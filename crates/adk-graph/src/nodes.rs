//! Built-in node types: functions, emitting functions, and joins.

use adk_core::Result;
use async_trait::async_trait;
use futures::future::BoxFuture;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use std::sync::Arc;

use crate::node::{Node, NodeConfig, NodeContext, NodeOutcome};

/// The closure shape a [`FunctionNode`] wraps.
pub type NodeFn = Arc<
    dyn for<'a> Fn(&'a NodeContext) -> BoxFuture<'a, Result<NodeOutcome>> + Send + Sync,
>;

/// A node that runs a closure.
///
/// Prefer [`FunctionNode::typed`] when the node has a concrete input and output
/// type — it handles the JSON conversion and reports a wiring mismatch as a
/// clear validation error instead of a silent `None`.
pub struct FunctionNode {
    name: String,
    config: NodeConfig,
    func: NodeFn,
}

impl FunctionNode {
    /// Builds a node from a closure over the raw [`NodeContext`].
    pub fn new<F>(name: impl Into<String>, config: NodeConfig, func: F) -> Self
    where
        F: for<'a> Fn(&'a NodeContext) -> BoxFuture<'a, Result<NodeOutcome>>
            + Send
            + Sync
            + 'static,
    {
        Self {
            name: name.into(),
            config,
            func: Arc::new(func),
        }
    }

    /// Builds a node from a typed `In -> Out` function.
    ///
    /// The predecessor's output is deserialized into `In`, and the returned
    /// `Out` becomes this node's output. Use `()` for a node that takes no
    /// input, such as one wired directly to the graph's entry point.
    pub fn typed<In, Out, F>(name: impl Into<String>, config: NodeConfig, func: F) -> Self
    where
        In: DeserializeOwned + Default + Send + Sync + 'static,
        Out: Serialize + Send + Sync + 'static,
        F: for<'a> Fn(In, &'a NodeContext) -> BoxFuture<'a, Result<Out>> + Send + Sync + 'static,
    {
        // The closure is shared rather than borrowed so the returned future can
        // outlive the outer call frame.
        let func = Arc::new(func);
        Self::new(name, config, move |ctx| {
            let func = Arc::clone(&func);
            Box::pin(async move {
                let input: In = ctx.typed_input()?.unwrap_or_default();
                let output = func(input, ctx).await?;
                Ok(NodeOutcome::output(serde_json::to_value(output)?))
            })
        })
    }

    /// Sets the node configuration.
    pub fn with_config(mut self, config: NodeConfig) -> Self {
        self.config = config;
        self
    }

    /// Wraps this node for registration with a graph.
    pub fn shared(self) -> Arc<dyn Node> {
        Arc::new(self)
    }
}

#[async_trait]
impl Node for FunctionNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn config(&self) -> &NodeConfig {
        &self.config
    }

    async fn run(&self, ctx: &NodeContext) -> Result<NodeOutcome> {
        (self.func)(ctx).await
    }
}

/// A node that decides where the graph goes next.
///
/// The closure returns the routes to emit alongside the output payload. The
/// engine matches those labels against this node's outgoing edges.
pub struct RouterNode {
    name: String,
    config: NodeConfig,
    func: NodeFn,
}

impl RouterNode {
    /// Builds a router from a closure returning `(output, routes)`.
    pub fn new<F>(name: impl Into<String>, config: NodeConfig, func: F) -> Self
    where
        F: for<'a> Fn(&'a NodeContext) -> BoxFuture<'a, Result<(Value, Vec<String>)>>
            + Send
            + Sync
            + 'static,
    {
        let func: NodeFn = Arc::new(move |ctx| {
            let fut = func(ctx);
            Box::pin(async move {
                let (output, routes) = fut.await?;
                Ok(NodeOutcome::output(output).with_routes(routes))
            })
        });
        Self {
            name: name.into(),
            config,
            func,
        }
    }

    /// Wraps this node for registration with a graph.
    pub fn shared(self) -> Arc<dyn Node> {
        Arc::new(self)
    }
}

#[async_trait]
impl Node for RouterNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn node_type(&self) -> &str {
        "emitting_function"
    }

    fn config(&self) -> &NodeConfig {
        &self.config
    }

    async fn run(&self, ctx: &NodeContext) -> Result<NodeOutcome> {
        (self.func)(ctx).await
    }
}

/// A fan-in point that waits for every predecessor before continuing.
///
/// Its output is a JSON object keyed by predecessor node name, so the
/// successor can address each branch's result by the node that produced it.
///
/// # Stalling
///
/// A join waits for *all* its predecessors. If one fails without producing an
/// output, the join never fires — ADK documents the same behaviour. The engine
/// surfaces this as an explicit error at the end of the run rather than
/// hanging, so a stalled join is diagnosable.
pub struct JoinNode {
    name: String,
    config: NodeConfig,
}

impl JoinNode {
    /// Builds a join node.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            config: NodeConfig::default(),
        }
    }

    /// Wraps this node for registration with a graph.
    pub fn shared(self) -> Arc<dyn Node> {
        Arc::new(self)
    }
}

#[async_trait]
impl Node for JoinNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn node_type(&self) -> &str {
        "join"
    }

    fn config(&self) -> &NodeConfig {
        &self.config
    }

    async fn run(&self, ctx: &NodeContext) -> Result<NodeOutcome> {
        // The engine assembles the keyed map and hands it in as the input.
        Ok(match &ctx.input {
            Some(value) => NodeOutcome::output(value.clone()),
            None => NodeOutcome::empty(),
        })
    }
}

/// Builds a node that returns a constant, useful for seeding a graph.
pub fn constant_node(name: impl Into<String>, value: Value) -> Arc<dyn Node> {
    FunctionNode::new(name, NodeConfig::default(), move |_ctx| {
        let value = value.clone();
        Box::pin(async move { Ok(NodeOutcome::output(value)) })
    })
    .shared()
}
