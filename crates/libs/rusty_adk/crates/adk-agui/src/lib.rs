//! Serve a Rust ADK agent over [AG-UI][agui], the Agent-User Interaction
//! protocol: the one `rusty_agui`'s React, Vue and Angular bindings, its
//! chat channels, its routines and `rusty_agent_gateway`'s `agui` route all
//! speak.
//!
//! [`AdkAgent`] wraps an ADK [`Runner`] in `rusty_agui`'s [`Agent`] trait, so
//! `rusty_agui::serve::AgentHandler` can serve it on `rusty_serve`:
//!
//! ```no_run
//! use adk_agents::LlmAgent;
//! use adk_agui::AdkAgent;
//! use adk_core::Services;
//! use adk_models::MockModel;
//! use adk_runner::Runner;
//! use adk_sessions::InMemorySessionService;
//! use rusty_agui::serve::AgentHandler;
//! use std::sync::Arc;
//!
//! let runtime = tokio::runtime::Runtime::new()?;
//! let agent = LlmAgent::builder("greeter")
//!     .model(Arc::new(MockModel::new().push_text("Hello!")))
//!     .description("Greets people.")
//!     .build()?
//!     .shared();
//! let runner = Runner::new("greeter_app", agent, Services::new(Arc::new(InMemorySessionService::new())));
//! let handler = AgentHandler::new(AdkAgent::new(Arc::new(runner), runtime.handle().clone()));
//! rusty_serve::Server::bind("127.0.0.1:8080".parse()?, handler)?.run()?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # What maps to what
//!
//! | AG-UI | ADK | Note |
//! |---|---|---|
//! | `threadId` | session id | one thread is one session, created on first contact |
//! | `forwardedProps.userId` | user id | the thread id when absent; see [`AdkAgent::with_user_resolver`] |
//! | the last user message | the new turn | ADK keeps the history; the client's copy is not replayed |
//! | `TEXT_MESSAGE_*` | a model text part | streamed as deltas when the run config streams |
//! | `TOOL_CALL_START`/`ARGS`/`END` | a `FunctionCall` part | the agent's own tools, reported as they are called |
//! | `TOOL_CALL_RESULT` | a `FunctionResponse` part | |
//! | `STATE_SNAPSHOT` | the session state | once at the end of a run that changed it; `temp:` keys left out |
//! | `RUN_ERROR` | an event with `error_code` or a failed stream | |
//! | a frontend tool call | a graph suspension | see below |
//!
//! Artifacts and thoughts are not forwarded: AG-UI has no event for a file,
//! and a reasoning trace is the model's, not the answer.
//!
//! # Human-in-the-loop
//!
//! When a graph node suspends through `resume_or_request_input`, the run
//! ends with a call to the frontend tool [`INPUT_TOOL`] whose id is the
//! interrupt id and whose arguments are the node's `hint` and `payload`.
//! A client that registers an action by that name renders the prompt; the
//! person's answer comes back as the tool message that answers the call,
//! and the next run on the thread resumes the graph at that node with the
//! answer as its payload. Neither side models the other's idea of waiting
//! for a person: AG-UI already had a call the client answers, ADK already
//! had a node that waits.
//!
//! # Threads
//!
//! `AgentHandler` runs the agent on its own thread, so [`AdkAgent`] drives
//! the runner's stream with `Handle::block_on` on the runtime it was given.
//! Hand it a handle from a multi-thread runtime, or a current-thread
//! runtime that nothing else is blocking.
//!
//! [agui]: https://docs.ag-ui.com

#![deny(missing_docs)]
#![warn(clippy::all)]

pub mod convert;

use adk_core::{AdkError, Content, Event as AdkEvent, RunConfig, Session};
use adk_graph::{PendingInterrupt, ResumeRequest, PENDING_STATE_KEY};
use adk_runner::Runner;
use futures::StreamExt;
use rusty_agui::serve::{Agent, Emitter};
use rusty_agui::{Error, EventKind, Message, Role, RunAgentInput};
use rusty_json::Value;
use std::sync::Arc;
use tokio::runtime::Handle;

use convert::{resume_payload, to_agui};

/// The frontend tool a graph suspension is reported as.
pub const INPUT_TOOL: &str = "request_input";

/// Decides which ADK user a run belongs to.
pub type UserResolver = Arc<dyn Fn(&RunAgentInput) -> String + Send + Sync>;

/// An ADK [`Runner`] behind AG-UI's [`Agent`] trait.
pub struct AdkAgent {
    runner: Arc<Runner>,
    handle: Handle,
    user_for: UserResolver,
    run_config: Option<RunConfig>,
}

impl AdkAgent {
    /// Wraps a runner; `handle` is the runtime its runs are driven on.
    pub fn new(runner: Arc<Runner>, handle: Handle) -> Self {
        Self {
            runner,
            handle,
            user_for: Arc::new(default_user),
            run_config: None,
        }
    }

    /// Sets how a run maps to an ADK user id. The default reads
    /// `forwardedProps.userId` and falls back to the thread id, which keeps
    /// one thread's `user:` state out of another's.
    pub fn with_user_resolver<F>(mut self, resolver: F) -> Self
    where
        F: Fn(&RunAgentInput) -> String + Send + Sync + 'static,
    {
        self.user_for = Arc::new(resolver);
        self
    }

    /// Applies a fixed [`RunConfig`] to every run.
    pub fn with_run_config(mut self, config: RunConfig) -> Self {
        self.run_config = Some(config);
        self
    }

    /// The runner this agent drives.
    pub fn runner(&self) -> &Arc<Runner> {
        &self.runner
    }

    /// Loads the session for this thread, creating it on first contact.
    async fn session_for(&self, user_id: &str, thread_id: &str) -> Result<Session, AdkError> {
        let sessions = &self.runner.services().session;
        if let Some(session) = sessions
            .get_session(self.runner.app_name(), user_id, thread_id)
            .await?
        {
            return Ok(session);
        }
        sessions
            .create_session(
                self.runner.app_name(),
                user_id,
                None,
                Some(thread_id.to_string()),
            )
            .await
    }

    async fn serve(
        &self,
        input: &RunAgentInput,
        out: &mut Emitter<'_>,
    ) -> Result<Option<Value>, Error> {
        let user_id = (self.user_for)(input);
        let session = self
            .session_for(&user_id, &input.thread_id)
            .await
            .map_err(adk_error)?;
        let turn = Turn::of(input, pending_interrupt(&session))?;
        let mut stream = match turn {
            Turn::Resume(resume) => {
                self.runner
                    .resume(&user_id, &input.thread_id, resume, self.run_config.clone())
            }
            Turn::Message(content) => {
                self.runner
                    .run(&user_id, &input.thread_id, content, self.run_config.clone())
            }
        };

        let mut relay = Relay::new(out);
        while let Some(event) = stream.next().await {
            let event = event.map_err(adk_error)?;
            relay.forward(&event)?;
            if event.request_input.is_some() {
                // The graph persisted its resume point and ended the run.
                break;
            }
        }
        drop(stream);
        relay.finish()?;
        let (result, state_changed) = (relay.result, relay.state_changed);

        if state_changed {
            let session = self
                .runner
                .session(&user_id, &input.thread_id)
                .await
                .map_err(adk_error)?;
            if let Some(session) = session {
                out.state(state_snapshot(&session)?)?;
            }
        }
        Ok(result)
    }
}

impl Agent for AdkAgent {
    fn run(
        &mut self,
        input: &RunAgentInput,
        out: &mut Emitter<'_>,
    ) -> Result<Option<Value>, Error> {
        let handle = self.handle.clone();
        handle.block_on(self.serve(input, out))
    }
}

fn default_user(input: &RunAgentInput) -> String {
    input
        .forwarded_props
        .get("userId")
        .and_then(Value::as_str)
        .unwrap_or(&input.thread_id)
        .to_string()
}

fn adk_error(error: AdkError) -> Error {
    Error::Agent(error.to_string())
}

/// The interrupt a previous run left pending on this session, if any. The
/// graph engine persists it under [`PENDING_STATE_KEY`]; this reads ADK's
/// own record rather than keeping a second one.
fn pending_interrupt(session: &Session) -> Option<PendingInterrupt> {
    let raw = session.state.get(PENDING_STATE_KEY)?;
    if raw.is_null() {
        return None;
    }
    serde_json::from_value(raw.clone()).ok()
}

/// The session's committed state as the client's shared state. `temp:`
/// keys are the invocation's own and the graph's resume point is ADK's.
fn state_snapshot(session: &Session) -> Result<Value, Error> {
    let map: serde_json::Map<String, serde_json::Value> = session
        .state
        .to_map()
        .into_iter()
        .filter(|(key, _)| !key.starts_with("temp:") && key != PENDING_STATE_KEY)
        .collect();
    to_agui(&serde_json::Value::Object(map))
}

/// What the run's last message asks for.
enum Turn {
    /// A new user turn.
    Message(Content),
    /// The answer to a suspended node.
    Resume(ResumeRequest),
}

impl Turn {
    fn of(input: &RunAgentInput, pending: Option<PendingInterrupt>) -> Result<Self, Error> {
        match input.messages.last() {
            Some(Message::User { content, .. }) => Ok(Turn::Message(Content::user_text(content.text()))),
            Some(Message::Tool {
                tool_call_id,
                content,
                ..
            }) => match pending {
                Some(pending) if pending.interrupt_id == *tool_call_id => Ok(Turn::Resume(
                    ResumeRequest::new(pending.interrupt_id, resume_payload(&content.text())),
                )),
                _ => Err(Error::Agent(format!(
                    "tool message {tool_call_id} answers no pending interrupt"
                ))),
            },
            _ => Err(Error::Agent(
                "the last message must be from the user, or a tool message answering a pending interrupt".into(),
            )),
        }
    }
}

/// Turns ADK events into AG-UI events as they arrive.
struct Relay<'e, 'a> {
    out: &'e mut Emitter<'a>,
    /// The message partial chunks are streaming into, if one is open.
    open: Option<String>,
    /// The output of the last final response.
    result: Option<Value>,
    state_changed: bool,
}

impl<'e, 'a> Relay<'e, 'a> {
    fn new(out: &'e mut Emitter<'a>) -> Self {
        Self {
            out,
            open: None,
            result: None,
            state_changed: false,
        }
    }

    fn forward(&mut self, event: &AdkEvent) -> Result<(), Error> {
        // The runner echoes the user's own turn back.
        if event.author == "user" {
            return Ok(());
        }
        if let Some(message) = event.error_message.as_ref().or(event.error_code.as_ref()) {
            return Err(Error::Agent(message.clone()));
        }
        if event.is_partial() {
            return self.chunk(event);
        }
        // A complete event after streamed chunks repeats their text.
        let streamed = self.close_open()?;
        let text = event
            .content
            .as_ref()
            .map(Content::text)
            .unwrap_or_default();
        let message_id = match (streamed, text.is_empty()) {
            (false, false) => Some(self.out.text(&text)?),
            _ => None,
        };
        for call in event.function_calls() {
            let id = call.id.clone().unwrap_or_else(|| self.out.next_id());
            self.out.emit(EventKind::ToolCallStart {
                tool_call_id: id.clone(),
                tool_call_name: call.name.clone(),
                parent_message_id: message_id.clone(),
            })?;
            let args = serde_json::Value::Object(call.args.clone()).to_string();
            self.out.emit(EventKind::ToolCallArgs {
                tool_call_id: id.clone(),
                delta: args,
            })?;
            self.out.emit(EventKind::ToolCallEnd { tool_call_id: id })?;
        }
        for response in event.function_responses() {
            let tool_call_id = response.id.clone().unwrap_or_else(|| self.out.next_id());
            let message_id = self.out.next_id();
            self.out.emit(EventKind::ToolCallResult {
                message_id,
                tool_call_id,
                content: response.response.to_string(),
                role: Some(Role::Tool),
            })?;
        }
        if let Some(request) = &event.request_input {
            let mut args = serde_json::Map::new();
            args.insert(
                "hint".into(),
                serde_json::Value::String(request.hint.clone()),
            );
            if let Some(payload) = &request.payload {
                args.insert("payload".into(), payload.clone());
            }
            let id = request.interrupt_id.clone();
            self.out.emit(EventKind::ToolCallStart {
                tool_call_id: id.clone(),
                tool_call_name: INPUT_TOOL.into(),
                parent_message_id: None,
            })?;
            self.out.emit(EventKind::ToolCallArgs {
                tool_call_id: id.clone(),
                delta: serde_json::Value::Object(args).to_string(),
            })?;
            self.out.emit(EventKind::ToolCallEnd { tool_call_id: id })?;
        }
        if event
            .actions
            .state_delta
            .keys()
            .any(|key| !key.starts_with("temp:"))
        {
            self.state_changed = true;
        }
        if event.is_final_response() {
            if let Some(output) = &event.output {
                self.result = Some(to_agui(output)?);
            }
        }
        Ok(())
    }

    /// A token-level chunk: text goes out as a delta of one open message.
    fn chunk(&mut self, event: &AdkEvent) -> Result<(), Error> {
        let text = event
            .content
            .as_ref()
            .map(Content::text)
            .unwrap_or_default();
        if text.is_empty() {
            return Ok(());
        }
        let message_id = match &self.open {
            Some(id) => id.clone(),
            None => {
                let id = self.out.next_id();
                self.out.emit(EventKind::TextMessageStart {
                    message_id: id.clone(),
                    role: Role::Assistant,
                })?;
                self.open = Some(id.clone());
                id
            }
        };
        self.out.emit(EventKind::TextMessageContent {
            message_id,
            delta: text,
        })
    }

    /// Closes the streamed message, if one is open; says whether one was.
    fn close_open(&mut self) -> Result<bool, Error> {
        let Some(message_id) = self.open.take() else {
            return Ok(false);
        };
        self.out.emit(EventKind::TextMessageEnd { message_id })?;
        Ok(true)
    }

    fn finish(&mut self) -> Result<(), Error> {
        self.close_open().map(|_| ())
    }
}
