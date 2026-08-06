//! The [`Agent`] trait and the [`RunContext`] handed to it for each run.

use std::{future::Future, sync::Arc, time::Duration};

use chrono::Utc;
use tokio::sync::{mpsc, Mutex};

use crate::{
    server::run::RunHandle,
    types::{
        AgentManifest, AgentName, AwaitRequest, AwaitResume, Error, Event, Message, MessagePart,
        Role, RunId, Session,
    },
};

/// An agent that can be hosted by an [`AcpServer`](crate::server::AcpServer).
///
/// Implementors describe themselves with a [`AgentManifest`] and do their work
/// in [`run`](Agent::run), emitting output through the [`RunContext`].
///
/// Returning `Ok(())` completes the run; returning `Err` fails it with the
/// given protocol error.
///
/// ```
/// use rusty_acp::server::{Agent, RunContext};
/// use rusty_acp::types::{AgentManifest, AgentName, Error};
///
/// struct Upper;
///
/// #[async_trait::async_trait]
/// impl Agent for Upper {
///     fn manifest(&self) -> AgentManifest {
///         AgentManifest::new(AgentName::new("upper").unwrap(), "Uppercases the input")
///     }
///
///     async fn run(&self, ctx: RunContext) -> Result<(), Error> {
///         ctx.reply_text(ctx.input_text().to_uppercase()).await?;
///         Ok(())
///     }
/// }
/// ```
#[async_trait::async_trait]
pub trait Agent: Send + Sync + 'static {
    /// The manifest published for this agent by the discovery endpoints.
    fn manifest(&self) -> AgentManifest;

    /// Execute one run.
    async fn run(&self, ctx: RunContext) -> Result<(), Error>;

    /// Whether a run of this agent may be replayed from its original input.
    ///
    /// **Defaults to `false`**, and that default is the safe one: when the
    /// replica executing a run dies, the run is failed and the client
    /// resubmits. Opting in lets the server instead start a replacement run
    /// automatically — see [the recovery docs](crate::server#recovering-a-lost-run).
    ///
    /// Only return `true` if running the agent twice on the same input is
    /// harmless. It is **not** harmless if the agent takes a payment, sends a
    /// message, or writes anything the second run would duplicate. The server
    /// cannot work this out — ACP carries no idempotency contract — so it has
    /// to be told.
    ///
    /// ```
    /// # use rusty_acp::server::{Agent, RunContext};
    /// # use rusty_acp::types::{AgentManifest, AgentName, Error};
    /// # struct Summarize;
    /// #[async_trait::async_trait]
    /// impl Agent for Summarize {
    ///     # fn manifest(&self) -> AgentManifest {
    ///     #     AgentManifest::new(AgentName::new("summarize").unwrap(), "Summarizes text")
    ///     # }
    ///     // Reading input and producing a summary has no external effects.
    ///     fn recoverable(&self) -> bool {
    ///         true
    ///     }
    ///     # async fn run(&self, ctx: RunContext) -> Result<(), Error> { Ok(()) }
    /// }
    /// ```
    fn recoverable(&self) -> bool {
        false
    }
}

/// Everything an [`Agent`] needs for one run: its input, its session, and the
/// handles used to emit output, pause for client input, and observe
/// cancellation.
///
/// # Why emitting is `async`
///
/// Every emit writes to the configured [`Store`](crate::server::store::Store)
/// and publishes to its subscribers. With the default in-process store that is
/// nearly free; with a shared backend it is a network write that can fail, and
/// the agent is the right place to decide what to do about that. The `?` on
/// each call is what propagates a storage outage into a failed run rather than
/// a silently truncated one.
#[derive(Debug)]
pub struct RunContext {
    agent_name: AgentName,
    run_id: RunId,
    input: Vec<Message>,
    session: Option<Session>,
    history: Vec<Message>,
    base_url: String,
    handle: Arc<RunHandle>,
    resume_rx: Mutex<mpsc::Receiver<AwaitResume>>,
    /// This replica's permission to be running this agent. Given up while the
    /// run is parked awaiting a client, and dropped when the run ends.
    slot: super::Slot,
    /// How long [`await_request`](RunContext::await_request) will wait.
    await_timeout: Option<Duration>,
}

impl RunContext {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        agent_name: AgentName,
        run_id: RunId,
        input: Vec<Message>,
        session: Option<Session>,
        history: Vec<Message>,
        base_url: String,
        handle: Arc<RunHandle>,
        resume_rx: mpsc::Receiver<AwaitResume>,
        slot: super::Slot,
        await_timeout: Option<Duration>,
    ) -> Self {
        Self {
            agent_name,
            run_id,
            input,
            session,
            history,
            base_url,
            handle,
            resume_rx: Mutex::new(resume_rx),
            slot,
            await_timeout,
        }
    }

    /// The name of the agent being run.
    pub fn agent_name(&self) -> &AgentName {
        &self.agent_name
    }

    /// The identifier of this run.
    pub fn run_id(&self) -> RunId {
        self.run_id
    }

    /// The input messages supplied by the client.
    pub fn input(&self) -> &[Message] {
        &self.input
    }

    /// The concatenated plain text of every input message.
    pub fn input_text(&self) -> String {
        self.input.iter().map(Message::text).collect::<Vec<_>>().join("\n")
    }

    /// The session this run belongs to, if any.
    ///
    /// Its `history` holds message *URLs*; the messages the store already holds
    /// are materialised in [`history`](RunContext::history). Anything hosted
    /// elsewhere must be fetched, for example with
    /// [`AcpClient::fetch_session_history`](crate::client::AcpClient::fetch_session_history).
    pub fn session(&self) -> Option<&Session> {
        self.session.as_ref()
    }

    /// Messages from earlier runs in this session that the store holds, in
    /// order.
    pub fn history(&self) -> &[Message] {
        &self.history
    }

    /// Read the state this agent stored for the session on an earlier run.
    ///
    /// State is how a [stateful agent][sa] carries something forward — a
    /// conversation summary, accumulated preferences, a working set — without
    /// replaying the whole history. It is scoped to the session, shared by
    /// every run in it, and survives across replicas.
    ///
    /// Returns `Ok(None)` when nothing has been stored yet. Fails if the run is
    /// not part of a session, since there would be nothing to scope state to.
    ///
    /// ```no_run
    /// # use rusty_acp::server::RunContext;
    /// # use rusty_acp::types::Error;
    /// # #[derive(serde::Serialize, serde::Deserialize, Default)]
    /// # struct Memory { turns: u32 }
    /// # async fn run(ctx: RunContext) -> Result<(), Error> {
    /// let mut memory: Memory = ctx.load_state().await?.unwrap_or_default();
    /// memory.turns += 1;
    /// ctx.store_state(&memory).await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// [sa]: https://agentcommunicationprotocol.dev/core-concepts/stateful-agents
    pub async fn load_state<T: serde::de::DeserializeOwned>(&self) -> Result<Option<T>, Error> {
        let session_id = self.require_session_id()?;
        let Some(value) = self.handle.store().get_session_state(session_id).await? else {
            return Ok(None);
        };
        serde_json::from_value(value)
            .map(Some)
            .map_err(|err| Error::server_error(format!("failed to decode session state: {err}")))
    }

    /// Replace the session's state document.
    ///
    /// The session's `state` field is pointed at the stored document, following
    /// ACP's model of state as a link rather than inline content — so
    /// `GET /session/{id}` stays small however large the state grows.
    ///
    /// Fails if the run is not part of a session.
    pub async fn store_state<T: serde::Serialize + ?Sized>(&self, state: &T) -> Result<(), Error> {
        let session_id = self.require_session_id()?;
        let value = serde_json::to_value(state).map_err(|err| {
            Error::invalid_input(format!("failed to encode session state: {err}"))
        })?;
        self.handle.store().put_session_state(session_id, &self.base_url, value).await
    }

    fn require_session_id(&self) -> Result<crate::types::SessionId, Error> {
        self.session.as_ref().map(|session| session.id).ok_or_else(|| {
            Error::invalid_input(
                "this run is not part of a session; state is session-scoped, so start the run \
                 with a `session_id`",
            )
        })
    }

    /// The role this agent should use for the messages it emits.
    pub fn role(&self) -> Role {
        Role::agent(self.agent_name.as_str())
    }

    /// Emit an arbitrary event on the run's stream.
    pub async fn emit(&self, event: Event) -> Result<(), Error> {
        self.handle.emit(event).await
    }

    /// Emit an agent-defined `generic` event.
    pub async fn emit_generic(&self, payload: serde_json::Value) -> Result<(), Error> {
        self.handle.emit(Event::generic(payload)).await
    }

    /// Emit a complete message as `message.created` + `message.completed` and
    /// append it to the run's output.
    pub async fn emit_message(&self, mut message: Message) -> Result<(), Error> {
        if message.created_at.is_none() {
            message.created_at = Some(Utc::now());
        }
        let opening = Message { parts: Vec::new(), ..message.clone() };
        self.handle.emit(Event::MessageCreated { message: opening }).await?;
        for part in &message.parts {
            self.handle.emit_part(part.clone()).await?;
        }
        if message.completed_at.is_none() {
            message.complete();
        }
        self.handle.emit(Event::MessageCompleted { message }).await
    }

    /// Emit a single-part `text/plain` message attributed to this agent.
    pub async fn reply_text(&self, text: impl Into<String>) -> Result<(), Error> {
        self.emit_message(Message::new(self.role(), [MessagePart::text(text)])).await
    }

    /// Emit a single-part message with an explicit content type.
    pub async fn reply_part(&self, part: MessagePart) -> Result<(), Error> {
        self.emit_message(Message::new(self.role(), [part])).await
    }

    /// Emit a named [artifact][ar] — a file, image, or structured output a
    /// client can offer for download or render richly.
    ///
    /// ```no_run
    /// # use rusty_acp::server::RunContext;
    /// # use rusty_acp::types::Error;
    /// # async fn run(ctx: RunContext) -> Result<(), Error> {
    /// ctx.reply_artifact("result.json", "application/json", r#"{"ok": true}"#).await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// For binary content, build the part with
    /// [`MessagePart::binary_artifact`] and pass it to
    /// [`reply_part`](RunContext::reply_part).
    ///
    /// [ar]: https://agentcommunicationprotocol.dev/how-to/generate-artifacts
    pub async fn reply_artifact(
        &self,
        name: impl Into<String>,
        content_type: impl Into<String>,
        content: impl Into<String>,
    ) -> Result<(), Error> {
        self.reply_part(MessagePart::artifact(name, content_type, content)).await
    }

    /// Begin a message that will be streamed part by part.
    ///
    /// The message is opened with `message.created`; each
    /// [`push`](MessageWriter::push) emits `message.part`, and
    /// [`finish`](MessageWriter::finish) emits `message.completed`. Dropping
    /// the writer without finishing leaves the message open; it is flushed
    /// automatically when the run reaches a terminal state.
    pub async fn begin_message(&self) -> Result<MessageWriter<'_>, Error> {
        self.begin_message_as(self.role()).await
    }

    /// Begin a streamed message attributed to a specific role.
    pub async fn begin_message_as(&self, role: Role) -> Result<MessageWriter<'_>, Error> {
        let message =
            Message { role, parts: Vec::new(), created_at: Some(Utc::now()), completed_at: None };
        self.handle.emit(Event::MessageCreated { message: message.clone() }).await?;
        Ok(MessageWriter { handle: &self.handle, message })
    }

    /// Pause the run and ask the client for more input.
    ///
    /// The run moves to [`RunStatus::Awaiting`](crate::types::RunStatus::Awaiting)
    /// with `await_request` set, and resolves when a client calls
    /// `POST /runs/{run_id}` with an [`AwaitResume`] — against *any* replica,
    /// not necessarily this one. If the run is cancelled while awaiting, this
    /// returns an error, which the executor turns into a cancellation rather
    /// than a failure.
    pub async fn await_request(&self, request: AwaitRequest) -> Result<AwaitResume, Error> {
        self.handle.set_awaiting(request).await?;

        // Give the execution slot back while parked. The client this run is
        // waiting on may never answer, and holding capacity for it would let
        // idle conversations starve work that is ready to run.
        self.slot.park(self.run_id);
        let mut resume_rx = self.resume_rx.lock().await;
        let cancel = self.handle.cancel_token();

        // A deadline the client never has to know about: it exists so a
        // conversation nobody answers stops costing a task, a run entry and a
        // lease renewal every few seconds, forever.
        //
        // A future that never resolves when unbounded, rather than duplicating
        // the whole `select!` for the two cases.
        let expiry = async {
            match self.await_timeout {
                Some(timeout) => tokio::time::sleep(timeout).await,
                None => std::future::pending().await,
            }
        };

        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                Err(Error::server_error("run was cancelled while awaiting client input"))
            }
            resume = resume_rx.recv() => match resume {
                Some(resume) => {
                    self.slot.unpark(self.run_id);
                    self.handle.set_in_progress().await?;
                    Ok(resume)
                }
                None => Err(Error::server_error("run can no longer be resumed")),
            },
            () = expiry => {
                // Named in the message: a bare `server_error` here sends
                // whoever reads the log hunting for a bug in their agent, when
                // what happened is that nobody answered.
                let waited = self.await_timeout.unwrap_or_default();
                tracing::info!(
                    run_id = %self.run_id,
                    ?waited,
                    "failing a run nobody answered"
                );
                Err(Error::server_error(format!(
                    "timed out after {waited:?} awaiting client input"
                )))
            }
        }
    }

    /// Ask the client for input with an arbitrary JSON payload.
    pub async fn await_json(&self, payload: serde_json::Value) -> Result<AwaitResume, Error> {
        self.await_request(AwaitRequest::new(payload)).await
    }

    /// Whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.handle.cancel_token().is_cancelled()
    }

    /// Resolves when cancellation is requested.
    ///
    /// Long-running agents should select on this so they can stop promptly; the
    /// executor also drops the agent future when the token fires.
    pub async fn cancelled(&self) {
        self.handle.cancel_token().cancelled().await;
    }

    /// A snapshot of the run as this replica currently sees it.
    pub fn run(&self) -> crate::types::Run {
        self.handle.snapshot()
    }
}

/// Streams the parts of a single message.
///
/// Created by [`RunContext::begin_message`].
#[derive(Debug)]
pub struct MessageWriter<'a> {
    handle: &'a Arc<RunHandle>,
    message: Message,
}

impl MessageWriter<'_> {
    /// Emit one more part of the message.
    pub async fn push(&mut self, part: MessagePart) -> Result<(), Error> {
        self.message.parts.push(part.clone());
        self.handle.emit_part(part).await
    }

    /// Emit one more `text/plain` part.
    pub async fn push_text(&mut self, text: impl Into<String>) -> Result<(), Error> {
        self.push(MessagePart::text(text)).await
    }

    /// The parts emitted so far.
    pub fn parts(&self) -> &[MessagePart] {
        &self.message.parts
    }

    /// Complete the message, emitting `message.completed` and appending it to
    /// the run's output.
    pub async fn finish(mut self) -> Result<Message, Error> {
        self.message.complete();
        let message = self.message.clone();
        self.handle.emit(Event::MessageCompleted { message: message.clone() }).await?;
        Ok(message)
    }
}

/// Build an [`Agent`] from a manifest and an async closure.
///
/// ```
/// use rusty_acp::server::agent_fn;
/// use rusty_acp::types::{AgentManifest, AgentName};
///
/// let agent = agent_fn(
///     AgentManifest::new(AgentName::new("echo").unwrap(), "Echoes the input"),
///     |ctx| async move {
///         ctx.reply_text(ctx.input_text()).await?;
///         Ok(())
///     },
/// );
/// ```
pub fn agent_fn<F, Fut>(manifest: AgentManifest, run: F) -> FnAgent<F>
where
    F: Fn(RunContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<(), Error>> + Send + 'static,
{
    FnAgent { manifest, run, recoverable: false }
}

/// An [`Agent`] backed by a closure. Built by [`agent_fn`].
#[derive(Debug, Clone)]
pub struct FnAgent<F> {
    manifest: AgentManifest,
    run: F,
    recoverable: bool,
}

impl<F> FnAgent<F> {
    /// Declare that runs of this agent may be replayed from their input.
    ///
    /// The same opt-in as [`Agent::recoverable`], and subject to the same
    /// caveat: only safe when running the closure twice on one input is
    /// harmless.
    ///
    /// Named `with_recovery` rather than `recoverable` so it cannot be confused
    /// with the trait method it sets.
    pub fn with_recovery(mut self) -> Self {
        self.recoverable = true;
        self
    }
}

#[async_trait::async_trait]
impl<F, Fut> Agent for FnAgent<F>
where
    F: Fn(RunContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<(), Error>> + Send + 'static,
{
    fn manifest(&self) -> AgentManifest {
        self.manifest.clone()
    }

    async fn run(&self, ctx: RunContext) -> Result<(), Error> {
        (self.run)(ctx).await
    }

    fn recoverable(&self) -> bool {
        self.recoverable
    }
}
