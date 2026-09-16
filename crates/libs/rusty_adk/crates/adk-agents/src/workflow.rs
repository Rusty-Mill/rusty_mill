//! Deterministic workflow agents: sequential, parallel, and loop.
//!
//! These are the template workflows ADK ships alongside the graph engine. They
//! control *when* sub-agents run rather than reasoning about it, which makes
//! them the right tool when the control flow is known in advance.

use adk_core::{AdkError, Event, InvocationContext, Result};
use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;
use std::sync::Arc;

use crate::agent::{Agent, SharedAgent};

/// Runs its sub-agents one after another, in order.
///
/// Each sub-agent sees whatever the previous one wrote to session state, which
/// is how a pipeline passes work along.
pub struct SequentialAgent {
    name: String,
    description: String,
    sub_agents: Vec<SharedAgent>,
}

impl SequentialAgent {
    /// Builds a sequential agent.
    pub fn new(name: impl Into<String>, sub_agents: Vec<SharedAgent>) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
            sub_agents,
        }
    }

    /// Sets the description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Wraps this agent for sharing.
    pub fn shared(self) -> SharedAgent {
        Arc::new(self)
    }
}

#[async_trait]
impl Agent for SequentialAgent {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn sub_agents(&self) -> &[SharedAgent] {
        &self.sub_agents
    }

    fn run<'a>(&'a self, ctx: &'a InvocationContext) -> BoxStream<'a, Result<Event>> {
        Box::pin(async_stream::try_stream! {
            let ctx = ctx.for_agent(&self.name);
            for sub in &self.sub_agents {
                ctx.check_cancelled()?;
                let mut stream = sub.run(&ctx);
                while let Some(event) = stream.next().await {
                    let event = event?;
                    // An escalation ends the pipeline early; the remaining
                    // stages would be operating on an abandoned task.
                    let escalated = event.actions.escalate;
                    yield event;
                    if escalated {
                        return;
                    }
                }
                if ctx.should_end_invocation() {
                    return;
                }
            }
        })
    }
}

/// Runs its sub-agents concurrently.
///
/// Each sub-agent gets its own branch path, so their events stay separable in
/// the session history even though they interleave on the stream.
pub struct ParallelAgent {
    name: String,
    description: String,
    sub_agents: Vec<SharedAgent>,
}

impl ParallelAgent {
    /// Builds a parallel agent.
    pub fn new(name: impl Into<String>, sub_agents: Vec<SharedAgent>) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
            sub_agents,
        }
    }

    /// Sets the description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Wraps this agent for sharing.
    pub fn shared(self) -> SharedAgent {
        Arc::new(self)
    }
}

#[async_trait]
impl Agent for ParallelAgent {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn sub_agents(&self) -> &[SharedAgent] {
        &self.sub_agents
    }

    fn run<'a>(&'a self, ctx: &'a InvocationContext) -> BoxStream<'a, Result<Event>> {
        let ctx = ctx.for_agent(&self.name);
        // Each branch runs against its own context so its events carry a
        // distinct branch path. `select_all` interleaves them as they arrive
        // rather than waiting for the slowest.
        let branches: Vec<BoxStream<'a, Result<Event>>> = self
            .sub_agents
            .iter()
            .map(|sub| {
                // The sub-agent stamps its own branch from this context, so
                // don't pre-nest its name here — that would double it up.
                let branch_ctx = ctx.clone();
                let branch = ctx.for_agent(sub.name()).branch;
                Box::pin(async_stream::try_stream! {
                    let mut stream = sub.run_owned(branch_ctx);
                    while let Some(event) = stream.next().await {
                        let mut event = event?;
                        if event.branch.is_none() {
                            event.branch = branch.clone();
                        }
                        yield event;
                    }
                }) as BoxStream<'a, Result<Event>>
            })
            .collect();

        Box::pin(futures::stream::select_all(branches))
    }
}

/// Repeats its sub-agents until a sub-agent escalates or the cap is reached.
///
/// Escalation is the intended exit: a sub-agent that decides the work is done
/// sets `escalate` on an event, and the loop stops after that iteration.
pub struct LoopAgent {
    name: String,
    description: String,
    sub_agents: Vec<SharedAgent>,
    max_iterations: u32,
}

impl LoopAgent {
    /// Builds a loop agent with an iteration cap.
    ///
    /// The cap is required rather than optional: a loop whose escalation
    /// condition never fires would otherwise run until the step budget or the
    /// model-call budget stopped it, with a far less obvious error.
    pub fn new(name: impl Into<String>, sub_agents: Vec<SharedAgent>, max_iterations: u32) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
            sub_agents,
            max_iterations,
        }
    }

    /// Sets the description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// The iteration cap.
    pub fn max_iterations(&self) -> u32 {
        self.max_iterations
    }

    /// Wraps this agent for sharing.
    pub fn shared(self) -> SharedAgent {
        Arc::new(self)
    }
}

#[async_trait]
impl Agent for LoopAgent {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn sub_agents(&self) -> &[SharedAgent] {
        &self.sub_agents
    }

    fn run<'a>(&'a self, ctx: &'a InvocationContext) -> BoxStream<'a, Result<Event>> {
        Box::pin(async_stream::try_stream! {
            let ctx = ctx.for_agent(&self.name);
            let mut iteration = 0u32;

            'outer: loop {
                if iteration >= self.max_iterations {
                    Err(AdkError::LimitExceeded(format!(
                        "loop agent '{}' reached max_iterations ({}) without escalating",
                        self.name, self.max_iterations
                    )))?;
                }
                iteration += 1;
                ctx.set_state("temp:loop_iteration", iteration);

                for sub in &self.sub_agents {
                    ctx.check_cancelled()?;
                    let mut stream = sub.run(&ctx);
                    while let Some(event) = stream.next().await {
                        let event = event?;
                        let escalated = event.actions.escalate;
                        yield event;
                        if escalated {
                            break 'outer;
                        }
                    }
                }

                if ctx.should_end_invocation() {
                    break;
                }
            }
        })
    }
}

/// Runs an agent against an owned context.
///
/// [`Agent::run`] borrows its context, which is what callers want in the common
/// case; a parallel branch needs to hand each sub-agent a context it owns.
pub trait AgentRunOwned {
    /// Runs the agent, taking ownership of the context.
    fn run_owned(&self, ctx: InvocationContext) -> BoxStream<'_, Result<Event>>;
}

impl<A: Agent + ?Sized> AgentRunOwned for A {
    fn run_owned(&self, ctx: InvocationContext) -> BoxStream<'_, Result<Event>> {
        Box::pin(async_stream::try_stream! {
            let mut stream = self.run(&ctx);
            while let Some(event) = stream.next().await {
                yield event?;
            }
        })
    }
}
