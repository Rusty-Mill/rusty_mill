//! Per-invocation context: the handle agents, nodes, tools, and callbacks use
//! to read state, reach services, and observe cancellation.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use crate::error::{AdkError, Result};
use crate::services::Services;
use crate::session::Session;
use crate::state::State;

/// How model output is delivered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamingMode {
    /// Wait for the complete response, then emit one event.
    #[default]
    None,
    /// Emit partial events as tokens arrive, then a final aggregated event.
    Sse,
}

/// Knobs for a single run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunConfig {
    /// Whether to stream model output.
    #[serde(default)]
    pub streaming_mode: StreamingMode,

    /// Ceiling on model calls per invocation.
    ///
    /// Guards against a tool-calling loop that never converges. Exceeding it
    /// fails the run with [`AdkError::LimitExceeded`].
    #[serde(default = "default_max_llm_calls")]
    pub max_llm_calls: u64,

    /// Ceiling on graph node executions per invocation.
    ///
    /// A cyclic graph whose exit condition never fires would otherwise run
    /// forever; exceeding this fails the run with [`AdkError::LimitExceeded`].
    #[serde(default = "default_max_graph_steps")]
    pub max_graph_steps: u64,
}

fn default_max_llm_calls() -> u64 {
    50
}

fn default_max_graph_steps() -> u64 {
    200
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            streaming_mode: StreamingMode::default(),
            max_llm_calls: default_max_llm_calls(),
            max_graph_steps: default_max_graph_steps(),
        }
    }
}

impl RunConfig {
    /// Enables token-by-token streaming.
    pub fn streaming(mut self) -> Self {
        self.streaming_mode = StreamingMode::Sse;
        self
    }

    /// Sets the model-call ceiling.
    pub fn with_max_llm_calls(mut self, max: u64) -> Self {
        self.max_llm_calls = max;
        self
    }

    /// Sets the graph-step ceiling.
    pub fn with_max_graph_steps(mut self, max: u64) -> Self {
        self.max_graph_steps = max;
        self
    }
}

/// Everything one user request needs while it is being handled.
///
/// Cloning is cheap and shares the underlying session, counters, and flags, so
/// a clone handed to a sub-agent or a parallel branch observes the same state
/// and the same cancellation signal.
#[derive(Clone)]
pub struct InvocationContext {
    /// Groups every event produced while handling this request.
    pub invocation_id: String,
    /// The agent application.
    pub app_name: String,
    /// The user this run belongs to.
    pub user_id: String,
    /// The conversation thread.
    pub session_id: String,
    /// Path through the agent hierarchy, e.g. `root.researcher`.
    pub branch: Option<String>,
    /// Name of the agent currently executing.
    pub agent_name: String,
    /// Run knobs.
    pub run_config: RunConfig,

    session: Arc<RwLock<Session>>,
    services: Services,
    cancelled: Arc<AtomicBool>,
    end_invocation: Arc<AtomicBool>,
    llm_calls: Arc<AtomicU64>,
}

impl InvocationContext {
    /// Builds a context for a run over `session`.
    pub fn new(session: Session, services: Services, run_config: RunConfig) -> Self {
        Self {
            invocation_id: crate::new_id("inv"),
            app_name: session.app_name.clone(),
            user_id: session.user_id.clone(),
            session_id: session.id.clone(),
            branch: None,
            agent_name: String::new(),
            run_config,
            session: Arc::new(RwLock::new(session)),
            services,
            cancelled: Arc::new(AtomicBool::new(false)),
            end_invocation: Arc::new(AtomicBool::new(false)),
            llm_calls: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Overrides the generated invocation id.
    pub fn with_invocation_id(mut self, id: impl Into<String>) -> Self {
        self.invocation_id = id.into();
        self
    }

    /// Returns a context scoped to a child agent, sharing all run state.
    ///
    /// The child's branch extends the parent's, which is what keeps parallel
    /// branches distinguishable in the event history.
    pub fn for_agent(&self, agent_name: impl Into<String>) -> Self {
        let agent_name = agent_name.into();
        let branch = match &self.branch {
            Some(parent) => format!("{parent}.{agent_name}"),
            None => agent_name.clone(),
        };
        Self {
            branch: Some(branch),
            agent_name,
            ..self.clone()
        }
    }

    /// The service bundle.
    pub fn services(&self) -> &Services {
        &self.services
    }

    /// Reads the session under a shared lock.
    pub fn with_session<R>(&self, f: impl FnOnce(&Session) -> R) -> R {
        // A poisoned lock means another thread panicked mid-run. There is no
        // meaningful repair, so recover the guard and let the run continue to
        // its own error rather than compounding one panic with another.
        let guard = self.session.read().unwrap_or_else(|e| e.into_inner());
        f(&guard)
    }

    /// Mutates the session under an exclusive lock.
    pub fn with_session_mut<R>(&self, f: impl FnOnce(&mut Session) -> R) -> R {
        let mut guard = self.session.write().unwrap_or_else(|e| e.into_inner());
        f(&mut guard)
    }

    /// Reads state under a shared lock.
    pub fn with_state<R>(&self, f: impl FnOnce(&State) -> R) -> R {
        self.with_session(|s| f(&s.state))
    }

    /// Mutates state under an exclusive lock.
    ///
    /// Writes are staged, not persisted. They land when an event carrying the
    /// resulting delta is processed by the runner.
    pub fn with_state_mut<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        self.with_session_mut(|s| f(&mut s.state))
    }

    /// Reads one state key.
    pub fn get_state(&self, key: &str) -> Option<Value> {
        self.with_state(|s| s.get(key).cloned())
    }

    /// Stages one state write.
    pub fn set_state(&self, key: impl Into<String>, value: impl Into<Value>) {
        self.with_state_mut(|s| s.set(key, value));
    }

    /// Takes the pending state delta, for attaching to an event.
    pub fn take_state_delta(&self) -> std::collections::BTreeMap<String, Value> {
        self.with_state_mut(|s| s.take_delta())
    }

    /// A snapshot of the session.
    pub fn session_snapshot(&self) -> Session {
        self.with_session(Clone::clone)
    }

    /// The shared session handle, for services that must hold it across awaits.
    pub fn session_handle(&self) -> Arc<RwLock<Session>> {
        Arc::clone(&self.session)
    }

    /// Signals that this run should stop as soon as it can.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Fails with [`AdkError::Cancelled`] if cancellation was requested.
    pub fn check_cancelled(&self) -> Result<()> {
        if self.is_cancelled() {
            return Err(AdkError::Cancelled);
        }
        Ok(())
    }

    /// Asks the runtime to stop after the current event.
    ///
    /// Unlike [`InvocationContext::cancel`] this is a clean finish, not an
    /// error: the run ends with whatever it has already produced.
    pub fn end_invocation(&self) {
        self.end_invocation.store(true, Ordering::SeqCst);
    }

    /// Whether a clean early finish was requested.
    pub fn should_end_invocation(&self) -> bool {
        self.end_invocation.load(Ordering::SeqCst)
    }

    /// Counts a model call and enforces [`RunConfig::max_llm_calls`].
    pub fn track_llm_call(&self) -> Result<u64> {
        let count = self.llm_calls.fetch_add(1, Ordering::SeqCst) + 1;
        if count > self.run_config.max_llm_calls {
            return Err(AdkError::LimitExceeded(format!(
                "exceeded max_llm_calls ({}) in invocation {}",
                self.run_config.max_llm_calls, self.invocation_id
            )));
        }
        Ok(count)
    }

    /// Model calls made so far in this invocation.
    pub fn llm_call_count(&self) -> u64 {
        self.llm_calls.load(Ordering::SeqCst)
    }
}

impl std::fmt::Debug for InvocationContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InvocationContext")
            .field("invocation_id", &self.invocation_id)
            .field("app_name", &self.app_name)
            .field("user_id", &self.user_id)
            .field("session_id", &self.session_id)
            .field("agent_name", &self.agent_name)
            .field("branch", &self.branch)
            .finish_non_exhaustive()
    }
}
