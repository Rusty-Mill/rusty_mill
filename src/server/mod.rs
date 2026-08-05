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
//! If a replica dies mid-run, the run it was executing would otherwise stay
//! non-terminal forever. Each executing replica holds a renewed lease on its
//! runs, so a non-terminal run whose lease has lapsed is recognised as
//! abandoned and failed by whichever replica next reads it — see
//! [`AcpServerBuilder::lease_ttl`] and [`Store::renew_lease`].
//!
//! # Recovering a lost run
//!
//! Failing an abandoned run is always correct but unambitious: the work is
//! lost and the client resubmits. An agent that declares itself
//! [`recoverable`](Agent::recoverable) gets more — when its replica dies, the
//! server starts a **replacement run** with the same input and session and a
//! fresh id, and links the two:
//!
//! ```text
//! run A: failed   error.data.replaced_by = <run B>
//!    └── run B: running   generic event { replaces: <run A>, attempt: 2 }
//! ```
//!
//! The abandoned run keeps its own history and stays failed. Nothing already
//! streamed to a client is retracted, and no run ever ends up with two sets of
//! output — which is why this is a new run rather than a re-execution in place.
//!
//! Three things are worth knowing before opting in:
//!
//! - **The default is `false`, deliberately.** Replaying an agent that takes a
//!   payment or sends a message repeats it. ACP carries no idempotency
//!   contract, so the server cannot infer which agents are safe.
//! - **Every replica must host the same agents.** The replica that notices an
//!   abandoned run is the one that re-runs it; if it does not have that agent
//!   registered, the run is failed as usual.
//! - **There is an attempt ceiling** — see
//!   [`AcpServerBuilder::max_recovery_attempts`] — so a run that kills whatever
//!   executes it cannot migrate around the fleet forever.
//!
//! See the [`store`] module for what a backend must guarantee.

mod agent;
mod routes;
mod run;
pub mod store;

use std::{collections::HashMap, sync::Arc, time::Duration};

use axum::{http::HeaderMap, Router};

use crate::types::{
    AgentManifest, AgentName, Error, Event, Message, Run, RunCreateRequest, RunId, RunStatus,
    Session, SessionId,
};

pub use agent::{agent_fn, Agent, FnAgent, MessageWriter, RunContext};
pub use run::RunHandle;
pub use store::{
    InMemoryStore, Notification, RecoveryRecord, SessionRecord, Store, DEFAULT_MAX_RUNS,
};

#[cfg(feature = "redis-store")]
#[cfg_attr(docsrs, doc(cfg(feature = "redis-store")))]
pub use store::{RedisStore, RedisStoreConfig};

use store::NotificationStream;

/// The default base URL used to build session history links when none is
/// configured and the request carries no `Host` header.
const DEFAULT_BASE_URL: &str = "http://localhost:8000";

/// How long a run's ownership lease survives without renewal.
///
/// A replica that stops renewing for this long is treated as gone and its runs
/// are failed. Renewal happens three times per lease lifetime, so several
/// renewals have to be missed before the lease lapses and a slow tick or brief
/// pause is not mistaken for death.
pub const DEFAULT_LEASE_TTL: Duration = Duration::from_secs(30);

/// The lease is renewed every `ttl / LEASE_RENEW_DIVISOR`, so several renewals
/// have to be missed before the lease lapses.
const LEASE_RENEW_DIVISOR: u32 = 3;

/// How long a `sync` request waits before returning whatever the run has
/// reached so far.
///
/// A `sync` call that never returns is worse than one that returns an
/// unfinished snapshot: proxies and load balancers cut the connection anyway,
/// and the caller is left with nothing to act on.
pub const DEFAULT_SYNC_TIMEOUT: Duration = Duration::from_secs(300);

/// How many times a run may be started before recovery gives up on it.
///
/// Counts attempts, not retries: `1` disables recovery entirely. Without a
/// ceiling, a run that reliably kills whatever executes it would migrate around
/// the fleet forever.
pub const DEFAULT_MAX_RECOVERY_ATTEMPTS: u32 = 3;

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
    replica_id: String,
    lease_ttl: Duration,
    sync_timeout: Option<Duration>,
    max_recovery_attempts: u32,
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

    /// This replica's identifier, used as the owner on run leases.
    pub fn replica_id(&self) -> &str {
        &self.replica_id
    }

    /// How long this replica's run leases survive without renewal.
    pub fn lease_ttl(&self) -> Duration {
        self.lease_ttl
    }

    /// How long a `sync` request waits before returning the run as it stands.
    pub fn sync_timeout(&self) -> Option<Duration> {
        self.sync_timeout
    }

    /// How many times a run may be started before recovery gives up.
    pub fn max_recovery_attempts(&self) -> u32 {
        self.max_recovery_attempts
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

        check_input_content_types(&agent.manifest(), &request.input)?;

        // Resolve the session before the run, so history is captured as it
        // stood before this run's input was added.
        let (session, history) = self.resolve_session(&request).await?;

        self.launch(
            LaunchSpec {
                agent,
                agent_name: request.agent_name,
                input: request.input,
                session,
                history,
                attempt: 1,
                // A first attempt contributes its input to the session; a
                // replacement must not, since the run it replaces already did.
                record_input_in_session: true,
                replaces: None,
            },
            base_url,
        )
        .await
    }

    /// Create a run, take its lease, and spawn its agent.
    ///
    /// Shared by fresh runs and by the replacements created when a run is
    /// recovered, so both go through exactly the same setup.
    async fn launch(
        self: &Arc<Self>,
        spec: LaunchSpec,
        base_url: &str,
    ) -> Result<(RunId, NotificationStream), Error> {
        let LaunchSpec {
            agent,
            agent_name,
            input,
            session,
            history,
            attempt,
            record_input_in_session,
            replaces,
        } = spec;

        let session_id = session.as_ref().map(|session| session.id);
        let run = Run::new(agent_name.clone(), session_id);
        let run_id = run.run_id;

        // Take ownership *before* the run exists in the store. A run that is
        // visible but unowned would look abandoned, and a reaper could fail it
        // before it ever started.
        self.store.renew_lease(run_id, &self.replica_id, self.lease_ttl).await?;

        // Only recoverable agents get their input persisted. For everyone else
        // the absence of a record is what makes fail-fast the default.
        if agent.recoverable() {
            self.store
                .put_recovery_record(
                    run_id,
                    Some(&RecoveryRecord { input: input.clone(), attempt }),
                )
                .await?;
        }

        self.store.put_run(&run).await?;

        // Subscribe before emitting anything so a streaming client sees
        // `run.created` and everything after it.
        let client_stream = self.store.subscribe(run_id).await?;
        let control_stream = self.store.subscribe(run_id).await?;

        // Before `run.created` goes out, for the same reason the output is
        // appended before the terminal event: a client woken by an event must
        // not read a session that is behind the run it just heard about.
        if record_input_in_session {
            if let Some(session_id) = session_id {
                self.store.append_session_messages(session_id, base_url, input.clone()).await?;
            }
        }

        let (handle, resume_rx) = RunHandle::new(Arc::clone(&self.store), run);
        handle.spawn_control_listener(control_stream);
        handle.set_created().await?;

        // Record the lineage on the run itself, using the one extension point
        // the specification gives us for agent-defined data.
        if let Some(replaced) = replaces {
            handle
                .emit(Event::generic(serde_json::json!({
                    "replaces": replaced.to_string(),
                    "attempt": attempt,
                })))
                .await?;
        }

        let ctx = RunContext::new(
            agent_name,
            run_id,
            input,
            session,
            history,
            base_url.to_string(),
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

    /// Start a fresh run to replace one that was abandoned.
    ///
    /// Same agent, same input, same session, new id. The session's history is
    /// *not* extended with the input again — the abandoned run already recorded
    /// it, and duplicating it would corrupt the conversation.
    async fn start_replacement(
        self: &Arc<Self>,
        abandoned: &Run,
        record: &RecoveryRecord,
    ) -> Result<RunId, Error> {
        let agent = self.agents.get(&abandoned.agent_name).cloned().ok_or_else(|| {
            Error::not_found(format!(
                "agent {} is no longer registered on this replica, so run {} cannot be recovered",
                abandoned.agent_name, abandoned.run_id
            ))
        })?;

        let (session, history) = match abandoned.session_id {
            Some(session_id) => {
                let record = self.store.ensure_session(Session::with_id(session_id)).await?;
                (Some(record.session), record.messages)
            }
            None => (None, Vec::new()),
        };

        let base_url = self.base_url.clone().unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        let (replacement, _) = self
            .launch(
                LaunchSpec {
                    agent,
                    agent_name: abandoned.agent_name.clone(),
                    input: record.input.clone(),
                    session,
                    history,
                    attempt: record.attempt + 1,
                    record_input_in_session: false,
                    replaces: Some(abandoned.run_id),
                },
                &base_url,
            )
            .await?;

        tracing::info!(
            abandoned = %abandoned.run_id,
            %replacement,
            attempt = record.attempt + 1,
            "started a replacement run"
        );
        Ok(replacement)
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

/// Everything [`AcpServer::launch`] needs to start one run.
struct LaunchSpec {
    agent: Arc<dyn Agent>,
    agent_name: AgentName,
    input: Vec<Message>,
    session: Option<Session>,
    history: Vec<Message>,
    attempt: u32,
    record_input_in_session: bool,
    replaces: Option<RunId>,
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

/// Keep renewing a run's lease until the task is dropped.
///
/// Runs for as long as the agent does. Aborting it — which happens the moment
/// the run finishes, or the moment the process dies — is what lets the lease
/// lapse and the run be recognised as abandoned.
async fn renew_lease_until_dropped(
    store: Arc<dyn Store>,
    run_id: RunId,
    owner: String,
    ttl: Duration,
) {
    let interval = ttl / LEASE_RENEW_DIVISOR;
    loop {
        tokio::time::sleep(interval).await;
        if let Err(error) = store.renew_lease(run_id, &owner, ttl).await {
            // Losing a renewal is not fatal on its own; several must be missed
            // before the lease lapses.
            tracing::warn!(%run_id, %error, "failed to renew run lease");
        }
    }
}

/// Fail a run whose executing replica is gone, and replace it if it may be
/// replayed.
///
/// A non-terminal run with no live lease has lost its only writer: nothing is
/// left to consume a resume, apply a cancel, or ever write a terminal state.
/// Rather than leave it hanging, mark it failed and publish `run.failed` so
/// waiters on every replica unblock.
///
/// If the agent declared itself [`recoverable`](Agent::recoverable) and the
/// attempt budget is not spent, a **replacement run** is then started here,
/// with the same input and session and a fresh id. The abandoned run keeps its
/// own history and stays failed — the two are linked rather than merged, so no
/// run ever ends up with two sets of output and nothing already streamed to a
/// client is retracted.
///
/// Returns the run as it stands — reaped or not.
pub(crate) async fn reap_if_abandoned(server: &Arc<AcpServer>, run: Run) -> Result<Run, Error> {
    let store = &server.store;
    if run.status.is_terminal() || store.lease_owner(run.run_id).await?.is_some() {
        return Ok(run);
    }

    let run_id = run.run_id;

    // Failing is idempotent, but starting a replacement is not: two replicas
    // must not both decide to recover the same run. Whoever wins this claim
    // does the work; the others leave the run alone and will see the outcome on
    // their next read.
    if !store.try_claim_lease(run_id, &server.replica_id, server.lease_ttl).await? {
        return Ok(run);
    }

    tracing::warn!(%run_id, "run has no live lease; failing it as abandoned");

    // Winning the claim is not the same as the run still needing to be failed.
    // `run` was read before the lease check, and the executing replica may have
    // reached a terminal state in between — writing its own outcome and
    // releasing the lease, which is what let this claim succeed at all. Failing
    // it now from the stale snapshot would overwrite a completed or cancelled
    // run, breaking the terminal-once rule.
    //
    // The window is microseconds on an in-process store and wide enough to hit
    // every time on one whose round-trips are milliseconds.
    let run = store.require_run(run_id).await?;
    if run.status.is_terminal() {
        store.release_lease(run_id).await?;
        return Ok(run);
    }

    let recovery = store.recovery_record(run_id).await?;
    let replacement = match &recovery {
        Some(record) if record.attempt < server.max_recovery_attempts => {
            match server.start_replacement(&run, record).await {
                Ok(replacement) => Some(replacement),
                Err(error) => {
                    // A failed replacement must not stop the original being
                    // failed, or the client is back to waiting forever.
                    tracing::error!(%run_id, %error, "failed to start a replacement run");
                    None
                }
            }
        }
        Some(record) => {
            tracing::warn!(
                %run_id,
                attempt = record.attempt,
                "not replacing the run: the recovery attempt budget is spent"
            );
            None
        }
        None => None,
    };

    let mut reaped = run;
    reaped.status = RunStatus::Failed;
    reaped.finished_at = Some(chrono::Utc::now());
    reaped.await_request = None;
    reaped.error = Some(match replacement {
        Some(replacement) => Error::server_error(
            "the replica executing this run stopped responding, so the run was abandoned. \
             A replacement run was started; see `data.replaced_by`.",
        )
        // `Error::data` is the specification's own slot for structured detail,
        // so the link travels to the client without inventing a field.
        .with_data(serde_json::json!({ "replaced_by": replacement.to_string() })),
        None => Error::server_error(
            "the replica executing this run stopped responding, so the run was abandoned. \
             Its lease expired without renewal; any output it had already produced is preserved.",
        ),
    });

    store.put_run(&reaped).await?;
    let event = Event::RunFailed { run: Box::new(reaped.clone()) };
    let index = store.append_event(run_id, &event).await?;
    store.publish(run_id, Notification::event_at(index, event)).await?;

    // The abandoned run is finished and its input is no longer needed.
    store.put_recovery_record(run_id, None).await?;
    store.release_lease(run_id).await?;

    Ok(reaped)
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

    // Held for exactly as long as the agent runs. If this process dies the task
    // dies with it, the lease lapses, and another replica can reap the run.
    let renewal = tokio::spawn(renew_lease_until_dropped(
        Arc::clone(&server.store),
        run_id,
        server.replica_id.clone(),
        server.lease_ttl,
    ));

    if let Err(error) = handle.set_in_progress().await {
        tracing::error!(%run_id, %error, "failed to start run");
        renewal.abort();
        return;
    }
    let cancel = handle.cancel_token();

    let outcome = tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            tracing::debug!(%run_id, "run cancelled");
            Outcome::Cancelled
        }
        result = agent.run(ctx) => match result {
            Ok(()) => Outcome::Completed,
            Err(error) => {
                tracing::warn!(%run_id, %error, "agent run failed");
                Outcome::Failed(error)
            }
        },
    };

    // Finish the output and get it into the session *before* the run is marked
    // terminal. That transition publishes the event which releases a `sync`
    // caller, and a caller told its run is done must not then read a session
    // history missing that run's output.
    let outcome = match record_output(&server, &handle, session_id, &base_url).await {
        Ok(()) => outcome,
        // A history write we could not complete is not a run that completed.
        // Same reasoning as emitting: a storage outage should fail the run
        // rather than leave it looking finished with its output missing.
        Err(error) => {
            tracing::error!(%run_id, %error, "failed to append run output to session");
            Outcome::Failed(error)
        }
    };

    let recorded = match outcome {
        Outcome::Completed => handle.set_completed().await,
        Outcome::Cancelled => handle.set_cancelled().await,
        Outcome::Failed(error) => handle.set_failed(error).await,
    };
    if let Err(error) = recorded {
        tracing::error!(%run_id, %error, "failed to record run outcome");
    }

    // The run is finished, so stop claiming it. Releasing is a courtesy — the
    // lease would expire anyway — but it stops a finished run looking owned.
    renewal.abort();
    if let Err(error) = server.store.release_lease(run_id).await {
        tracing::warn!(%run_id, %error, "failed to release run lease");
    }
    // A finished run will never be replayed, so stop holding its input.
    if let Err(error) = server.store.put_recovery_record(run_id, None).await {
        tracing::warn!(%run_id, %error, "failed to clear the recovery record");
    }
}

/// How a run ended, before that ending is written down.
///
/// The terminal write is deferred so the session history can be brought up to
/// date first, which means the outcome has to be carried rather than applied
/// where it is decided.
enum Outcome {
    Completed,
    Cancelled,
    Failed(Error),
}

/// Close off the run's output and append it to the session, if it has one.
///
/// Always finalises the output, session or not: an agent that returned
/// mid-message has a message to flush either way.
async fn record_output(
    server: &Arc<AcpServer>,
    handle: &Arc<RunHandle>,
    session_id: Option<SessionId>,
    base_url: &str,
) -> Result<(), Error> {
    let output = handle.finalize_output().await?;

    let Some(session_id) = session_id else { return Ok(()) };
    if output.is_empty() {
        return Ok(());
    }
    server.store.append_session_messages(session_id, base_url, output).await?;
    Ok(())
}

/// Builder for [`AcpServer`].
#[derive(Default)]
pub struct AcpServerBuilder {
    agents: Vec<Arc<dyn Agent>>,
    store: Option<Arc<dyn Store>>,
    base_url: Option<String>,
    max_runs: Option<usize>,
    replica_id: Option<String>,
    lease_ttl: Option<Duration>,
    sync_timeout: Option<Option<Duration>>,
    max_recovery_attempts: Option<u32>,
}

impl std::fmt::Debug for AcpServerBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcpServerBuilder")
            .field("agents", &self.agents.len())
            .field("store", &self.store)
            .field("base_url", &self.base_url)
            .field("max_runs", &self.max_runs)
            .field("replica_id", &self.replica_id)
            .field("lease_ttl", &self.lease_ttl)
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

    /// Name this replica. Defaults to a fresh identifier per process.
    ///
    /// Unlike the other settings here, this one is *meant* to differ between
    /// replicas — it is what identifies the holder of a run's lease.
    ///
    /// The value is the owner recorded on run leases, so a stable, meaningful
    /// name (a pod name, a hostname) makes it obvious in logs which replica was
    /// executing an abandoned run.
    pub fn replica_id(mut self, replica_id: impl Into<String>) -> Self {
        self.replica_id = Some(replica_id.into());
        self
    }

    /// How long a run's lease survives without renewal. Defaults to
    /// [`DEFAULT_LEASE_TTL`].
    ///
    /// This is the window between a replica dying and its runs being failed.
    /// Shorter reacts faster; too short risks mistaking a stalled process for a
    /// dead one and failing a run that was about to finish.
    pub fn lease_ttl(mut self, lease_ttl: Duration) -> Self {
        self.lease_ttl = Some(lease_ttl);
        self
    }

    /// Cap how many times a run may be started before recovery gives up.
    /// Defaults to [`DEFAULT_MAX_RECOVERY_ATTEMPTS`].
    ///
    /// Counts attempts, not retries, so `1` turns recovery off while leaving
    /// abandoned runs still promptly failed.
    ///
    /// Set this to the same value on every replica. The replica that *notices*
    /// an abandoned run is the one that decides whether to replace it, so a
    /// fleet with mismatched budgets behaves like whichever replica got there
    /// first.
    pub fn max_recovery_attempts(mut self, attempts: u32) -> Self {
        self.max_recovery_attempts = Some(attempts.max(1));
        self
    }

    /// How long a `sync` request waits before returning the run as it stands.
    /// Defaults to [`DEFAULT_SYNC_TIMEOUT`].
    ///
    /// The response is still a valid [`Run`]; it just may not be settled, so
    /// callers should check `status` rather than assume it is terminal.
    pub fn sync_timeout(mut self, sync_timeout: Duration) -> Self {
        self.sync_timeout = Some(Some(sync_timeout));
        self
    }

    /// Let `sync` requests wait indefinitely.
    ///
    /// Only sound when something else bounds the request — and note that a
    /// caller whose executing replica dies still unblocks, because the run gets
    /// failed once its lease lapses.
    pub fn without_sync_timeout(mut self) -> Self {
        self.sync_timeout = Some(None);
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
            replica_id: self
                .replica_id
                .unwrap_or_else(|| format!("replica-{}", uuid::Uuid::new_v4())),
            lease_ttl: self.lease_ttl.unwrap_or(DEFAULT_LEASE_TTL),
            sync_timeout: self.sync_timeout.unwrap_or(Some(DEFAULT_SYNC_TIMEOUT)),
            max_recovery_attempts: self
                .max_recovery_attempts
                .unwrap_or(DEFAULT_MAX_RECOVERY_ATTEMPTS),
        })
    }
}
