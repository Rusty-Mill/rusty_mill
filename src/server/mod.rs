//! Host your own agents behind the standard ACP HTTP endpoints.
//!
//! Implement [`Agent`], register it with [`AcpServer::builder`], and serve the
//! resulting [`axum::Router`]. The server implements the whole protocol
//! surface: discovery, the three run modes, await/resume, cancellation, the
//! event log and distributed sessions.
//!
//! ```no_run
//! use rusty_acp::server::{agent_fn, AcpServer};
//! use rusty_acp::types::{AgentManifest, AgentName};
//!
//! # async fn serve() -> Result<(), Box<dyn std::error::Error>> {
//! let echo = agent_fn(
//!     AgentManifest::new(AgentName::new("echo")?, "Echoes the input back"),
//!     |ctx| async move {
//!         ctx.reply_text(ctx.input_text());
//!         Ok(())
//!     },
//! );
//!
//! let router = AcpServer::builder().agent(echo).build()?.into_router();
//! let listener = tokio::net::TcpListener::bind("0.0.0.0:8000").await?;
//! axum::serve(listener, router).await?;
//! # Ok(())
//! # }
//! ```

mod agent;
mod routes;
pub mod store;

use std::{collections::HashMap, sync::Arc};

use axum::{http::HeaderMap, Router};

use crate::types::{
    AgentManifest, AgentName, Error, Message, Run, RunCreateRequest, Session, SessionId,
};

pub use agent::{agent_fn, Agent, FnAgent, MessageWriter, RunContext};
pub use store::{RunHandle, SessionRecord, Store, DEFAULT_MAX_RUNS};

use routes::Ready;

/// The default base URL used to build session history links when none is
/// configured and the request carries no `Host` header.
const DEFAULT_BASE_URL: &str = "http://localhost:8000";

/// A configured ACP server: a set of agents plus the store backing their runs.
///
/// Build one with [`AcpServer::builder`], then call [`into_router`](AcpServer::into_router).
#[derive(Debug)]
pub struct AcpServer {
    agents: HashMap<AgentName, Arc<dyn Agent>>,
    order: Vec<AgentName>,
    store: Arc<Store>,
    base_url: Option<String>,
}

impl std::fmt::Debug for dyn Agent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Agent").field("name", &self.manifest().name).finish()
    }
}

impl AcpServer {
    /// Start configuring a server.
    pub fn builder() -> AcpServerBuilder {
        AcpServerBuilder::default()
    }

    /// Consume the server, producing the router that serves the ACP endpoints.
    pub fn into_router(self) -> Router {
        routes::router(Arc::new(self))
    }

    /// Produce the router while keeping a handle on the server, so runs and
    /// sessions can be inspected from outside the HTTP layer.
    pub fn into_shared_router(self) -> (Arc<Self>, Router) {
        let server = Arc::new(self);
        (Arc::clone(&server), routes::router(Arc::clone(&server)))
    }

    /// The run and session store.
    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }

    /// Manifests of every registered agent, in registration order.
    pub fn manifests(&self) -> Vec<AgentManifest> {
        self.order.iter().filter_map(|name| self.manifest(name)).collect()
    }

    /// The manifest of one agent.
    pub fn manifest(&self, name: &AgentName) -> Option<AgentManifest> {
        self.agents.get(name).map(|agent| agent.manifest())
    }

    /// The configured base URL, if one was set.
    pub fn base_url(&self) -> Option<&str> {
        self.base_url.as_deref()
    }

    /// The base URL to use for links in a response.
    ///
    /// Prefers the explicitly configured base URL, then the request's
    /// `Host` header (honouring `X-Forwarded-Proto` and `X-Forwarded-Host`),
    /// and finally [`DEFAULT_BASE_URL`].
    fn resolve_base_url(&self, headers: &HeaderMap) -> String {
        if let Some(base_url) = &self.base_url {
            return base_url.trim_end_matches('/').to_string();
        }
        let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
        let host = header("x-forwarded-host").or_else(|| header("host"));
        match host {
            Some(host) => {
                let scheme = header("x-forwarded-proto").unwrap_or("http");
                format!("{scheme}://{host}")
            }
            None => DEFAULT_BASE_URL.to_string(),
        }
    }

    /// Validate a create request, register the run, and spawn its agent.
    async fn start_run(
        self: &Arc<Self>,
        request: RunCreateRequest,
        base_url: &str,
    ) -> Result<(Arc<RunHandle>, Ready), Error> {
        let agent =
            self.agents.get(&request.agent_name).cloned().ok_or_else(|| {
                Error::not_found(format!("agent {} not found", request.agent_name))
            })?;

        let manifest = agent.manifest();
        check_input_content_types(&manifest, &request.input)?;

        // Resolve the session before the run so history is captured as it stood
        // before this run's input was added.
        let (session, history) = self.resolve_session(&request);
        let session_id = session.as_ref().map(|session| session.id);

        let run = Run { session_id, ..Run::new(request.agent_name.clone(), session_id) };
        let (handle, resume_rx) = self.store.insert_run(run);

        // Subscribe before emitting anything so a streaming client sees
        // `run.created` and everything after it.
        let events = handle.subscribe();
        let mut status = handle.watch_status();
        status.borrow_and_update();
        handle.set_created();

        if let Some(session_id) = session_id {
            self.store.append_session_messages(session_id, base_url, request.input.iter().cloned());
        }

        let ctx = RunContext::new(
            request.agent_name,
            handle.run_id(),
            request.input,
            session,
            history,
            Arc::clone(&handle),
            resume_rx,
        );

        let server = Arc::clone(self);
        let executor_handle = Arc::clone(&handle);
        let base_url = base_url.to_string();
        tokio::spawn(async move {
            execute(server, agent, ctx, executor_handle, session_id, base_url).await;
        });

        Ok((handle, Ready { events, status }))
    }

    /// Determine the session for a run and the local history it should see.
    fn resolve_session(&self, request: &RunCreateRequest) -> (Option<Session>, Vec<Message>) {
        let record = match (&request.session, request.session_id) {
            (Some(session), _) => Some(self.store.ensure_session(session.clone())),
            (None, Some(session_id)) => {
                Some(self.store.ensure_session(Session::with_id(session_id)))
            }
            (None, None) => None,
        };
        match record {
            Some(record) => (Some(record.session), record.messages),
            None => (None, Vec::new()),
        }
    }
}

/// Reject input the agent has not declared it can consume.
fn check_input_content_types(manifest: &AgentManifest, input: &[Message]) -> Result<(), Error> {
    for message in input {
        for part in &message.parts {
            if !manifest.accepts_input(&part.content_type) {
                return Err(Error::invalid_input(format!(
                    "agent {} does not accept input of type `{}`; supported types: {}",
                    manifest.name,
                    part.content_type,
                    manifest.input_content_types.join(", ")
                )));
            }
        }
    }
    Ok(())
}

/// Drive one run to a terminal state.
async fn execute(
    server: Arc<AcpServer>,
    agent: Arc<dyn Agent>,
    ctx: RunContext,
    handle: Arc<RunHandle>,
    session_id: Option<SessionId>,
    base_url: String,
) {
    handle.set_in_progress();
    let cancel = handle.cancel_token();

    tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            tracing::debug!(run_id = %handle.run_id(), "run cancelled");
            handle.set_cancelled();
        }
        result = agent.run(ctx) => match result {
            Ok(()) => handle.set_completed(),
            Err(error) => {
                tracing::warn!(run_id = %handle.run_id(), %error, "agent run failed");
                handle.set_failed(error);
            }
        },
    }

    if let Some(session_id) = session_id {
        let output = handle.snapshot().output;
        if !output.is_empty() {
            server.store.append_session_messages(session_id, &base_url, output);
        }
    }
}

/// Builder for [`AcpServer`].
#[derive(Default)]
pub struct AcpServerBuilder {
    agents: Vec<Arc<dyn Agent>>,
    base_url: Option<String>,
    max_runs: Option<usize>,
}

impl std::fmt::Debug for AcpServerBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcpServerBuilder")
            .field("agents", &self.agents.len())
            .field("base_url", &self.base_url)
            .field("max_runs", &self.max_runs)
            .finish()
    }
}

impl AcpServerBuilder {
    /// Register an agent.
    pub fn agent(mut self, agent: impl Agent) -> Self {
        self.agents.push(Arc::new(agent));
        self
    }

    /// Register an already-shared agent.
    pub fn shared_agent(mut self, agent: Arc<dyn Agent>) -> Self {
        self.agents.push(agent);
        self
    }

    /// Register several agents at once.
    pub fn agents(mut self, agents: impl IntoIterator<Item = Arc<dyn Agent>>) -> Self {
        self.agents.extend(agents);
        self
    }

    /// Set the public base URL used to build session history links.
    ///
    /// When unset, the URL is derived from each request's `Host` header, which
    /// is usually right for direct deployments but not behind a proxy that
    /// rewrites paths.
    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
    }

    /// Cap how many runs are retained in memory. Defaults to
    /// [`DEFAULT_MAX_RUNS`]. Active runs are never evicted.
    pub fn max_runs(mut self, max_runs: usize) -> Self {
        self.max_runs = Some(max_runs);
        self
    }

    /// Validate the configuration and build the server.
    ///
    /// Fails if no agent was registered, if two agents share a name, or if a
    /// manifest violates the specification.
    pub fn build(self) -> Result<AcpServer, Error> {
        if self.agents.is_empty() {
            return Err(Error::invalid_input("an ACP server must register at least one agent"));
        }

        let mut agents = HashMap::with_capacity(self.agents.len());
        let mut order = Vec::with_capacity(self.agents.len());
        for agent in self.agents {
            let manifest = agent.manifest();
            manifest.validate()?;
            let name = manifest.name.clone();
            if agents.insert(name.clone(), agent).is_some() {
                return Err(Error::invalid_input(format!("agent {name} is registered twice")));
            }
            order.push(name);
        }

        Ok(AcpServer {
            agents,
            order,
            store: Arc::new(Store::new(self.max_runs.unwrap_or(DEFAULT_MAX_RUNS))),
            base_url: self.base_url.map(|url| url.trim_end_matches('/').to_string()),
        })
    }
}
