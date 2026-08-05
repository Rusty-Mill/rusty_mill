//! The [`Agent`] trait and the [`RunContext`] handed to it for each run.

use std::{future::Future, sync::Arc};

use chrono::Utc;
use tokio::sync::{mpsc, Mutex};

use crate::{
    server::store::RunHandle,
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
///         ctx.reply_text(ctx.input_text().to_uppercase());
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
}

/// Everything an [`Agent`] needs for one run: its input, its session, and the
/// handles used to emit output, pause for client input, and observe
/// cancellation.
#[derive(Debug)]
pub struct RunContext {
    agent_name: AgentName,
    run_id: RunId,
    input: Vec<Message>,
    session: Option<Session>,
    history: Vec<Message>,
    handle: Arc<RunHandle>,
    resume_rx: Mutex<mpsc::Receiver<AwaitResume>>,
}

impl RunContext {
    pub(crate) fn new(
        agent_name: AgentName,
        run_id: RunId,
        input: Vec<Message>,
        session: Option<Session>,
        history: Vec<Message>,
        handle: Arc<RunHandle>,
        resume_rx: mpsc::Receiver<AwaitResume>,
    ) -> Self {
        Self {
            agent_name,
            run_id,
            input,
            session,
            history,
            handle,
            resume_rx: Mutex::new(resume_rx),
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
    /// Its `history` holds message *URLs*; the messages this server itself
    /// stores are already materialised in [`history`](RunContext::history).
    /// Remote history must be fetched, for example with
    /// [`AcpClient::fetch_session_history`](crate::client::AcpClient::fetch_session_history).
    pub fn session(&self) -> Option<&Session> {
        self.session.as_ref()
    }

    /// Messages from earlier runs in this session that this server holds
    /// locally, in order.
    pub fn history(&self) -> &[Message] {
        &self.history
    }

    /// The role this agent should use for the messages it emits.
    pub fn role(&self) -> Role {
        Role::agent(self.agent_name.as_str())
    }

    /// Emit an arbitrary event on the run's stream.
    pub fn emit(&self, event: Event) {
        self.handle.emit(event);
    }

    /// Emit an agent-defined `generic` event.
    pub fn emit_generic(&self, payload: serde_json::Value) {
        self.handle.emit(Event::generic(payload));
    }

    /// Emit a complete message as `message.created` + `message.completed` and
    /// append it to the run's output.
    pub fn emit_message(&self, mut message: Message) {
        if message.created_at.is_none() {
            message.created_at = Some(Utc::now());
        }
        let opening = Message { parts: Vec::new(), ..message.clone() };
        self.handle.emit(Event::MessageCreated { message: opening });
        for part in &message.parts {
            self.handle.emit_part(part.clone());
        }
        if message.completed_at.is_none() {
            message.complete();
        }
        self.handle.emit(Event::MessageCompleted { message });
    }

    /// Emit a single-part `text/plain` message attributed to this agent.
    pub fn reply_text(&self, text: impl Into<String>) {
        self.emit_message(Message::new(self.role(), [MessagePart::text(text)]));
    }

    /// Emit a single-part message with an explicit content type.
    pub fn reply_part(&self, part: MessagePart) {
        self.emit_message(Message::new(self.role(), [part]));
    }

    /// Begin a message that will be streamed part by part.
    ///
    /// The message is opened with `message.created`; each
    /// [`push`](MessageWriter::push) emits `message.part`, and
    /// [`finish`](MessageWriter::finish) emits `message.completed`. Dropping
    /// the writer without finishing leaves the message open; it is flushed
    /// automatically when the run reaches a terminal state.
    pub fn begin_message(&self) -> MessageWriter<'_> {
        self.begin_message_as(self.role())
    }

    /// Begin a streamed message attributed to a specific role.
    pub fn begin_message_as(&self, role: Role) -> MessageWriter<'_> {
        let message =
            Message { role, parts: Vec::new(), created_at: Some(Utc::now()), completed_at: None };
        self.handle.emit(Event::MessageCreated { message: message.clone() });
        MessageWriter { handle: &self.handle, message, finished: false }
    }

    /// Pause the run and ask the client for more input.
    ///
    /// The run moves to [`RunStatus::Awaiting`](crate::types::RunStatus::Awaiting)
    /// with `await_request` set, and resolves when the client calls
    /// `POST /runs/{run_id}` with an [`AwaitResume`]. If the run is cancelled
    /// while awaiting, this returns an error, which the executor turns into a
    /// cancellation rather than a failure.
    pub async fn await_request(&self, request: AwaitRequest) -> Result<AwaitResume, Error> {
        self.handle.set_awaiting(request);
        let mut resume_rx = self.resume_rx.lock().await;
        let cancel = self.handle.cancel_token();
        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                Err(Error::server_error("run was cancelled while awaiting client input"))
            }
            resume = resume_rx.recv() => match resume {
                Some(resume) => {
                    self.handle.set_in_progress();
                    Ok(resume)
                }
                None => Err(Error::server_error("run can no longer be resumed")),
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

    /// A snapshot of the run as the server currently sees it.
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
    finished: bool,
}

impl MessageWriter<'_> {
    /// Emit one more part of the message.
    pub fn push(&mut self, part: MessagePart) -> &mut Self {
        self.message.parts.push(part.clone());
        self.handle.emit_part(part);
        self
    }

    /// Emit one more `text/plain` part.
    pub fn push_text(&mut self, text: impl Into<String>) -> &mut Self {
        self.push(MessagePart::text(text))
    }

    /// The parts emitted so far.
    pub fn parts(&self) -> &[MessagePart] {
        &self.message.parts
    }

    /// Complete the message, emitting `message.completed` and appending it to
    /// the run's output.
    pub fn finish(mut self) -> Message {
        self.finished = true;
        let mut message = std::mem::replace(
            &mut self.message,
            Message { role: Role::Agent, parts: Vec::new(), created_at: None, completed_at: None },
        );
        message.complete();
        self.handle.emit(Event::MessageCompleted { message: message.clone() });
        message
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
///         ctx.reply_text(ctx.input_text());
///         Ok(())
///     },
/// );
/// ```
pub fn agent_fn<F, Fut>(manifest: AgentManifest, run: F) -> FnAgent<F>
where
    F: Fn(RunContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<(), Error>> + Send + 'static,
{
    FnAgent { manifest, run }
}

/// An [`Agent`] backed by a closure. Built by [`agent_fn`].
#[derive(Debug, Clone)]
pub struct FnAgent<F> {
    manifest: AgentManifest,
    run: F,
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
}
