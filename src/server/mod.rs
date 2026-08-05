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
//!         ctx.reply_text(ctx.input_text()).await?;
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
//!
//! # Running several replicas
//!
//! By default runs live in process memory, which is right for a single agent
//! host. To put several replicas behind a load balancer, give each the *same*
//! shared [`Store`] — then any replica can serve any request for
//! any run, and no session affinity is needed:
//!
//! ```no_run
//! # #[cfg(feature = "redis-store")]
//! # async fn serve() -> Result<(), Box<dyn std::error::Error>> {
//! # use rusty_acp::server::{AcpServer, store::RedisStore};
//! # let my_agent = rusty_acp::server::agent_fn(
//! #     rusty_acp::types::AgentManifest::new(
//! #         rusty_acp::types::AgentName::new("echo")?, "Echoes"),
//! #     |ctx| async move { ctx.reply_text(ctx.input_text()).await?; Ok(()) });
//! let store = RedisStore::connect("redis://127.0.0.1/").await?;
//!
//! let router = AcpServer::builder()
//!     .agent(my_agent)
//!     .store(std::sync::Arc::new(store))
//!     .build()?
//!     .into_router();
//! # Ok(())
//! # }
//! ```
//!
//! See the [`store`] module for what a backend must guarantee.

mod agent;
mod routes;
mod run;
pub mod store;

use std::{collections::HashMap, sync::Arc};

use axum::{http::HeaderMap, Router};

use crate::types::{
    AgentManifest, AgentName, Error, Message, Run, RunCreateRequest, RunId, Session, SessionId,
};

pub use agent::{agent_fn, Agent, FnAgent, MessageWriter, RunContext};
pub use run::RunHandle;
pub use store::{InMemoryStore, Notification, SessionRecord, Store, DEFAULT_MAX_RUNS};

#[cfg(feature = "redis-store")]
#[cfg_attr(docsrs, doc(cfg(feature = "redis-store")))]
pub use store::{RedisStore, RedisStoreConfig};

use store::NotificationStream;

/// The default base URL used to build session history links when none is
/// configured and the request carries no `Host` header.
const DEFAULT_BASE_URL: &str = "http://localhost:8000";

/// A configured ACP server: a set of agents plus the store backing their runs.
///
/// Build one with [`AcpServer::builder`], then call
/// [`into_router`](AcpServer::into_router).
#[derive(Debug)]
pub struct AcpServer {
    agents: HashMap<AgentName, Arc<dyn Agent>>,
    order: Vec<AgentName>,
    store: Arc<dyn Store>,
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

    /// The store backing runs and sessions.
    pub fn store(&self) -> &Arc<dyn Store> {
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
    /// Prefers the explicitly configured base URL, then the request's `Host`
    /// header (honouring `X-Forwarded-Proto` and `X-Forwarded-Host`), and
    /// finally [`DEFAULT_BASE_URL`].
    ///
    /// With several replicas, set the base URL explicitly to the load
    /// balancer's address: a session's history links must stay resolvable no
    /// matter which replica wrote them.
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
    ///
    /// Returns the run id and a notification stream subscribed *before* the
    /// run emitted anything, so the caller sees every event from `run.created`
    /// onwards.
    async fn start_run(
        self: &Arc<Self>,
        request: RunCreateRequest,
        base_url: &str,
    ) -> Result<(RunId, NotificationStream), Error> {
        let agent =
            self.agents.get(&request.agent_name).cloned().ok_or_else(|| {
                Error::not_found(format!("agent {} not found", request.agent_name))
            })?;

        let manifest = agent.manifest();
        check_input_content_types(&manifest, &request.input)?;

        // Resolve the session before the run, so history is captured as it
        // stood before this run's input was added.
        let (session, history) = self.resolve_session(&request).await?;
        let session_id = session.as_ref().map(|session| session.id);

        let run = Run::new(request.agent_name.clone(), session_id);
        let run_id = run.run_id;
        self.store.put_run(&run).await?;

        // Subscribe before emitting anything so a streaming client sees
        // `run.created` and everything after it.
        let client_stream = self.store.subscribe(run_id).await?;
        let control_stream = self.store.subscribe(run_id).await?;

        let (handle, resume_rx) = RunHandle::new(Arc::clone(&self.store), run);
        handle.spawn_control_listener(control_stream);
        handle.set_created().await?;

        if let Some(session_id) = session_id {
            self.store.append_session_messages(session_id, base_url, request.input.clone()).await?;
        }

        let ctx = RunContext::new(
            request.agent_name,
            run_id,
            request.input,
            session,
            history,
            Arc::clone(&handle),
            resume_rx,
        );

        let server = Arc::clone(self);
        let base_url = base_url.to_string();
        tokio::spawn(async move {
            execute(server, agent, ctx, handle, session_id, base_url).await;
        });

        Ok((run_id, client_stream))
    }

    /// Determine the session for a run and the history it should see.
    async fn resolve_session(
        &self,
        request: &RunCreateRequest,
    ) -> Result<(Option<Session>, Vec<Message>), Error> {
        let record = match (&request.session, request.session_id) {
            (Some(session), _) => Some(self.store.ensure_session(session.clone()).await?),
            (None, Some(session_id)) => {
                Some(self.store.ensure_session(Session::with_id(session_id)).await?)
            }
            (None, None) => None,
        };
        Ok(match record {
            Some(record) => (Some(record.session), record.messages),
            None => (None, Vec::new()),
        })
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
    let run_id = handle.run_id();
    if let Err(error) = handle.set_in_progress().await {
        tracing::error!(%run_id, %error, "failed to start run");
        return;
    }
    let cancel = handle.cancel_token();

    let outcome = tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            tracing::debug!(%run_id, "run cancelled");
            handle.set_cancelled().await
        }
        result = agent.run(ctx) => match result {
            Ok(()) => handle.set_completed().await,
            Err(error) => {
                tracing::warn!(%run_id, %error, "agent run failed");
                handle.set_failed(error).await
            }
        },
    };

    if let Err(error) = outcome {
        tracing::error!(%run_id, %error, "failed to record run outcome");
    }

    if let Some(session_id) = session_id {
        let output = handle.snapshot().output;
        if !output.is_empty() {
            if let Err(error) =
                server.store.append_session_messages(session_id, &base_url, output).await
            {
                tracing::error!(%run_id, %error, "failed to append run output to session");
            }
        }
    }
}

/// Builder for [`AcpServer`].
#[derive(Default)]
pub struct AcpServerBuilder {
    agents: Vec<Arc<dyn Agent>>,
    store: Option<Arc<dyn Store>>,
    base_url: Option<String>,
    max_runs: Option<usize>,
}

impl std::fmt::Debug for AcpServerBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcpServerBuilder")
            .field("agents", &self.agents.len())
            .field("store", &self.store)
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

    /// Use a specific storage backend.
    ///
    /// Give every replica the same shared store to run several behind a load
    /// balancer. Defaults to an [`InMemoryStore`], which confines a run to the
    /// process that started it.
    ///
    /// Takes precedence over [`max_runs`](AcpServerBuilder::max_runs), which
    /// only configures the default store.
    pub fn store(mut self, store: Arc<dyn Store>) -> Self {
        self.store = Some(store);
        self
    }

    /// Set the public base URL used to build session history links.
    ///
    /// When unset, the URL is derived from each request's `Host` header, which
    /// is usually right for direct deployments but not behind a proxy that
    /// rewrites paths — nor across replicas, where links must point at the load
    /// balancer rather than at whichever replica happened to serve the request.
    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
    }

    /// Cap how many runs the default [`InMemoryStore`] retains. Defaults to
    /// [`DEFAULT_MAX_RUNS`]. Active runs are never evicted.
    ///
    /// Ignored when a store is supplied with [`store`](AcpServerBuilder::store).
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

        let store = self.store.unwrap_or_else(|| {
            Arc::new(InMemoryStore::new(self.max_runs.unwrap_or(DEFAULT_MAX_RUNS)))
        });

        Ok(AcpServer {
            agents,
            order,
            store,
            base_url: self.base_url.map(|url| url.trim_end_matches('/').to_string()),
        })
    }
}
