//! The [`Graph`] and its execution engine.
//!
//! Execution proceeds in frontiers. Every node in a frontier runs
//! concurrently; when they finish, their emitted routes select the outgoing
//! edges that form the next frontier. Join nodes hold their successors back
//! until every predecessor has delivered.

use adk_core::{AdkError, Event, InvocationContext, Result};
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::edge::Edge;
use crate::node::{NodeContext, NodeOutcome, SharedNode, START};

/// Session-state key holding a suspended run's resume point.
///
/// Deliberately not `temp:`-prefixed: a suspension outlives the invocation that
/// created it, so it must survive to the run that resumes it.
pub const PENDING_STATE_KEY: &str = "adk:graph_pending";

/// The record persisted when a node suspends the workflow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingInterrupt {
    /// Correlates the suspension with the response that lifts it.
    pub interrupt_id: String,
    /// The node that suspended.
    pub node: String,
    /// The input that node was executing against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
    /// The node whose output produced that input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predecessor: Option<String>,
    /// The step index at which the suspension happened.
    pub step: u64,
}

/// One item of work in a frontier.
#[derive(Debug, Clone)]
struct WorkItem {
    node: String,
    input: Option<Value>,
    predecessor: Option<String>,
}

/// A workflow graph: named nodes plus the edges between them.
///
/// `Debug` prints the topology — node names and edges — rather than the node
/// implementations, which are trait objects with nothing useful to show.
pub struct Graph {
    nodes: HashMap<String, SharedNode>,
    edges: Vec<Edge>,
    /// Number of incoming edges per node, used to know when a join is ready.
    in_degree: HashMap<String, usize>,
    join_nodes: HashSet<String>,
}

impl Graph {
    /// Builds and validates a graph.
    ///
    /// Fails when an edge names a node that does not exist, when two nodes
    /// share a name, or when there is no entry point — all of which would
    /// otherwise surface as a confusing mid-run failure.
    pub fn new(nodes: Vec<SharedNode>, edges: Vec<Edge>) -> Result<Self> {
        let mut map: HashMap<String, SharedNode> = HashMap::new();
        for node in nodes {
            if map.insert(node.name().to_string(), Arc::clone(&node)).is_some() {
                return Err(AdkError::Graph(format!(
                    "duplicate node name '{}'",
                    node.name()
                )));
            }
        }

        let mut in_degree: HashMap<String, usize> = HashMap::new();
        for edge in &edges {
            if edge.from != START && !map.contains_key(&edge.from) {
                return Err(AdkError::Graph(format!(
                    "edge source '{}' is not a node in this graph",
                    edge.from
                )));
            }
            if !map.contains_key(&edge.to) {
                return Err(AdkError::Graph(format!(
                    "edge target '{}' is not a node in this graph",
                    edge.to
                )));
            }
            *in_degree.entry(edge.to.clone()).or_insert(0) += 1;
        }

        if !edges.iter().any(|e| e.from == START) {
            return Err(AdkError::Graph(
                "graph has no entry point: no edge originates from START".into(),
            ));
        }

        let join_nodes = map
            .values()
            .filter(|n| n.node_type() == "join")
            .map(|n| n.name().to_string())
            .collect();

        Ok(Self {
            nodes: map,
            edges,
            in_degree,
            join_nodes,
        })
    }

    /// The node registered under `name`.
    pub fn node(&self, name: &str) -> Option<&SharedNode> {
        self.nodes.get(name)
    }

    /// Every node name in the graph.
    pub fn node_names(&self) -> Vec<&str> {
        self.nodes.keys().map(String::as_str).collect()
    }

    /// The graph's edges.
    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    /// The successors of `from`, given the routes it emitted.
    ///
    /// Matching edges win; when none match, [`Route::Default`] edges are used.
    /// A node with outgoing edges that emitted routes matching none of them and
    /// has no default is a routing dead end, reported as [`AdkError::NoRoute`].
    fn successors(&self, from: &str, routes: &[String]) -> Result<Vec<&Edge>> {
        let outgoing: Vec<&Edge> = self.edges.iter().filter(|e| e.from == from).collect();
        if outgoing.is_empty() {
            return Ok(Vec::new());
        }

        let matched: Vec<&Edge> = outgoing
            .iter()
            .copied()
            .filter(|e| e.route.matches(routes))
            .collect();
        if !matched.is_empty() {
            return Ok(matched);
        }

        let defaults: Vec<&Edge> = outgoing
            .iter()
            .copied()
            .filter(|e| e.route.is_default())
            .collect();
        if !defaults.is_empty() {
            return Ok(defaults);
        }

        Err(AdkError::NoRoute {
            node: from.to_string(),
            routes: routes.to_vec(),
        })
    }

    /// Runs one node, applying its retry policy.
    async fn run_node(&self, node: &SharedNode, ctx: &NodeContext) -> Result<NodeOutcome> {
        let max_retries = node.config().max_retries;
        let mut attempt = 0;
        loop {
            match node.run(ctx).await {
                Ok(outcome) => return Ok(outcome),
                Err(err) => {
                    // A suspension or confirmation request is control flow, not
                    // a failure; retrying it would re-ask the user forever.
                    if err.is_control_flow() || !err.is_retryable() || attempt >= max_retries {
                        return Err(err);
                    }
                    attempt += 1;
                    tracing::warn!(
                        node = node.name(),
                        attempt,
                        error = %err,
                        "retrying node after failure"
                    );
                }
            }
        }
    }

    /// Executes the graph, yielding events as they are produced.
    ///
    /// Pass `resume` to continue a run that a node suspended; see
    /// [`Graph::pending_interrupt`] for reading the suspension record.
    pub fn run<'a>(
        &'a self,
        invocation: InvocationContext,
        resume: Option<ResumeRequest>,
    ) -> BoxStream<'a, Result<Event>> {
        Box::pin(async_stream::try_stream! {
            let mut step: u64 = 0;
            let mut pending_joins: HashMap<String, BTreeMap<String, Value>> = HashMap::new();
            let mut resume_payload: Option<Value> = None;

            // Seed the frontier: either the graph's entry nodes, or the single
            // node that suspended on a previous run.
            let mut frontier: Vec<WorkItem> = match &resume {
                Some(request) => {
                    let pending = Self::read_pending(&invocation).ok_or_else(|| {
                        AdkError::Graph("no suspended run to resume".into())
                    })?;
                    if pending.interrupt_id != request.interrupt_id {
                        Err(AdkError::Graph(format!(
                            "interrupt id mismatch: suspended on '{}', resumed with '{}'",
                            pending.interrupt_id, request.interrupt_id
                        )))?;
                    }
                    step = pending.step;
                    resume_payload = Some(request.payload.clone());
                    Self::clear_pending(&invocation);
                    vec![WorkItem {
                        node: pending.node,
                        input: pending.input,
                        predecessor: pending.predecessor,
                    }]
                }
                None => self
                    .edges
                    .iter()
                    .filter(|e| e.from == START)
                    .map(|e| WorkItem {
                        node: e.to.clone(),
                        input: None,
                        predecessor: None,
                    })
                    .collect(),
            };

            while !frontier.is_empty() {
                invocation.check_cancelled()?;
                if step >= invocation.run_config.max_graph_steps {
                    Err(AdkError::LimitExceeded(format!(
                        "exceeded max_graph_steps ({})",
                        invocation.run_config.max_graph_steps
                    )))?;
                }

                // Run the whole frontier concurrently. Each node gets its own
                // channel so its progress events stay attributable to it.
                let mut running = Vec::new();
                for item in &frontier {
                    let node = self.nodes.get(&item.node).ok_or_else(|| {
                        AdkError::Graph(format!("node '{}' is not in this graph", item.node))
                    })?;
                    let (tx, rx) = mpsc::unbounded_channel();
                    let ctx = NodeContext::new(invocation.clone(), &item.node, tx)
                        .with_input(item.input.clone())
                        .with_predecessor(item.predecessor.clone())
                        .with_step(step)
                        .with_resume_payload(resume_payload.take());
                    running.push((item.clone(), Arc::clone(node), ctx, rx));
                }

                let outcomes = futures::future::join_all(
                    running
                        .iter()
                        .map(|(_, node, ctx, _)| self.run_node(node, ctx)),
                )
                .await;

                let mut next_frontier: Vec<WorkItem> = Vec::new();

                for ((item, node, ctx, mut rx), outcome) in running.into_iter().zip(outcomes) {
                    // Collect whatever the node emitted while it ran. These are
                    // held rather than yielded immediately so that a suspension
                    // can attach its resume record to the outgoing event.
                    rx.close();
                    let mut emitted: Vec<Event> = Vec::new();
                    while let Some(event) = rx.recv().await {
                        emitted.push(event);
                    }

                    let outcome = match outcome {
                        Ok(outcome) => {
                            for event in emitted {
                                yield event;
                            }
                            outcome
                        }
                        Err(AdkError::NodeInterrupted { interrupt_id }) => {
                            // Persist the resume point and end the run cleanly.
                            // The record is staged as a state write, so it must
                            // ride out on an event or it will never be
                            // committed and the run could not be resumed.
                            Self::write_pending(
                                &invocation,
                                &PendingInterrupt {
                                    interrupt_id,
                                    node: item.node.clone(),
                                    input: item.input.clone(),
                                    predecessor: item.predecessor.clone(),
                                    step,
                                },
                            );
                            // Carry the record out on the event so a session
                            // service persists it, and apply it locally too so
                            // a graph driven without a runner can still be
                            // resumed from the same context.
                            let delta = invocation.take_state_delta();
                            invocation.with_state_mut(|state| state.commit(delta.clone()));
                            match emitted.pop() {
                                Some(mut last) => {
                                    last.actions.state_delta.extend(delta);
                                    for event in emitted {
                                        yield event;
                                    }
                                    yield last;
                                }
                                None => {
                                    let mut carrier =
                                        Event::new(&invocation.invocation_id, &item.node)
                                            .with_node_info(ctx.node_info(node.node_type()));
                                    carrier.actions.state_delta = delta;
                                    yield carrier;
                                }
                            }
                            return;
                        }
                        Err(err) => {
                            for event in emitted {
                                yield event;
                            }
                            yield Event::new(&invocation.invocation_id, &item.node)
                                .with_node_info(ctx.node_info(node.node_type()))
                                .with_error(err.code(), err.to_string());
                            Err(err)?;
                            return;
                        }
                    };

                    // The node's own completion event carries its output and
                    // the state delta accumulated while it ran.
                    let mut event = Event::new(&invocation.invocation_id, &item.node)
                        .with_node_info(ctx.node_info(node.node_type()))
                        .with_routes(outcome.routes.clone());
                    if let Some(output) = &outcome.output {
                        event.output = Some(output.clone());
                    }
                    event.actions.state_delta = invocation.take_state_delta();
                    yield event;

                    for edge in self.successors(&item.node, &outcome.routes)? {
                        if self.join_nodes.contains(&edge.to) {
                            let arrivals = pending_joins.entry(edge.to.clone()).or_default();
                            arrivals.insert(
                                item.node.clone(),
                                outcome.output.clone().unwrap_or(Value::Null),
                            );
                            let expected =
                                self.in_degree.get(&edge.to).copied().unwrap_or(0);
                            if arrivals.len() >= expected {
                                let merged: Map<String, Value> =
                                    arrivals.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                                pending_joins.remove(&edge.to);
                                next_frontier.push(WorkItem {
                                    node: edge.to.clone(),
                                    input: Some(Value::Object(merged)),
                                    predecessor: Some(item.node.clone()),
                                });
                            }
                        } else {
                            next_frontier.push(WorkItem {
                                node: edge.to.clone(),
                                input: outcome.output.clone(),
                                predecessor: Some(item.node.clone()),
                            });
                        }
                    }
                }

                frontier = next_frontier;
                step += 1;

                if invocation.should_end_invocation() {
                    break;
                }
            }

            // A join whose predecessors never all arrived would otherwise look
            // like a run that simply ended early. Say so instead.
            if !pending_joins.is_empty() {
                let stalled: Vec<String> = pending_joins.keys().cloned().collect();
                Err(AdkError::Graph(format!(
                    "join node(s) {stalled:?} never received every predecessor's output; \
                     a branch failed or was routed around"
                )))?;
            }
        })
    }

    /// The suspension record for `invocation`, if a node suspended the run.
    pub fn pending_interrupt(invocation: &InvocationContext) -> Option<PendingInterrupt> {
        Self::read_pending(invocation)
    }

    fn read_pending(invocation: &InvocationContext) -> Option<PendingInterrupt> {
        invocation
            .get_state(PENDING_STATE_KEY)
            .and_then(|value| serde_json::from_value(value).ok())
    }

    fn write_pending(invocation: &InvocationContext, pending: &PendingInterrupt) {
        if let Ok(value) = serde_json::to_value(pending) {
            invocation.set_state(PENDING_STATE_KEY, value);
        }
    }

    fn clear_pending(invocation: &InvocationContext) {
        invocation.set_state(PENDING_STATE_KEY, Value::Null);
    }
}

impl std::fmt::Debug for Graph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut names: Vec<&str> = self.nodes.keys().map(String::as_str).collect();
        names.sort_unstable();
        f.debug_struct("Graph")
            .field("nodes", &names)
            .field("edges", &self.edges)
            .finish()
    }
}

/// The answer that lifts a suspension and resumes a workflow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResumeRequest {
    /// Must match the id of the suspension being resumed.
    pub interrupt_id: String,
    /// The human's answer, handed to the node as its resume payload.
    pub payload: Value,
}

impl ResumeRequest {
    /// Builds a resume request.
    pub fn new(interrupt_id: impl Into<String>, payload: Value) -> Self {
        Self {
            interrupt_id: interrupt_id.into(),
            payload,
        }
    }
}
