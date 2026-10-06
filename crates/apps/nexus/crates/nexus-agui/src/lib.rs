//! Serve a Nexus agent session over [AG-UI][agui], the Agent-User
//! Interaction protocol: the one `rusty_agui`'s React, Vue and Angular
//! bindings, its chat channels, its routines and `rusty_agent_gateway`'s
//! `agui` route all speak.
//!
//! [`NexusAgent`] implements `rusty_agui`'s [`Agent`] trait over the
//! ai-runtime (ADR 0028): each AG-UI run submits one agent session through
//! `com.nexus.ai.runtime::submit` and relays the typed `AiEvent` stream the
//! runtime republishes on the bus. `rusty_agui::serve::AgentHandler` serves
//! it on `rusty_serve`:
//!
//! ```ignore
//! // Needs a forge and the full bootstrap; shown rather than run.
//! use nexus_agui::{KernelRuntime, NexusAgent};
//! use rusty_agui::serve::AgentHandler;
//! use std::sync::Arc;
//!
//! let runtime = tokio::runtime::Runtime::new()?;
//! let nexus = runtime.block_on(async {
//!     nexus_bootstrap::build_cli_runtime("/path/to/forge".into())
//! })?;
//! let agent = NexusAgent::new(
//!     Arc::new(KernelRuntime::new(Arc::new(nexus.context))),
//!     runtime.handle().clone(),
//! );
//! rusty_serve::Server::bind("127.0.0.1:8080".parse()?, AgentHandler::new(agent))?.run()?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # What maps to what
//!
//! | AG-UI | Nexus | Note |
//! |---|---|---|
//! | the last user message | `session_run`'s `goal` | one run is one session; see below |
//! | `forwardedProps.archetype` | `session_run`'s `archetype` | optional |
//! | `TEXT_MESSAGE_*` | `TokenChunk` | streamed as deltas of one message |
//! | `TOOL_CALL_START`/`ARGS`/`END` | `ToolCalled` | the agent's own tools, with the runtime's arguments preview |
//! | `TOOL_CALL_RESULT` | `ToolResult` | the summary; an error as `{"error": …}` |
//! | a frontend tool call | `RoundProposed` | see below |
//! | `RUN_FINISHED` result | `Finished.outcome` | the session's id, outcome and tokens used |
//! | `RUN_ERROR` | `Failed`, `Cancelled`, a closed stream | |
//!
//! When no token streamed, the finished session's last round text is sent
//! as the message, so a non-streaming provider still answers.
//!
//! # Human-in-the-loop
//!
//! With [`NexusAgent::with_approval`], the session's rounds are gated: the
//! agent publishes `round_proposed` and waits for `round_decide`. The run
//! ends with a call to the frontend tool [`DECIDE_TOOL`] whose id names
//! the session and round and whose arguments carry the round's narration.
//! A client that registers an action by that name renders the proposal;
//! the person's answer comes back as the tool message that answers the
//! call, and the next run on the thread delivers it through `round_decide`
//! (`approve`, `yes` or `approve_all` approves every call; a JSON object is
//! passed through as the decision; any other text aborts with that reason)
//! and keeps relaying the same session until it finishes or proposes the
//! next round. The session waits in the runtime's worker meanwhile, for as
//! long as its approval timeout allows.
//!
//! Without it (the default), rounds are auto-approved and a run is one
//! request: the runtime's gate belongs to a UI that can answer it.
//!
//! # One run, one session
//!
//! The runtime's `submit` starts a session; it does not resume one, and a
//! resumed Nexus session is a fork with a new id. So each user turn is a
//! new session whose goal is that message, and the thread's earlier turns
//! are not replayed. Nexus's own session memory is what carries context
//! between sessions.
//!
//! [agui]: https://docs.ag-ui.com

#![deny(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod runtime;

use std::collections::HashMap;
use std::sync::Arc;

use futures::StreamExt;
use nexus_ai_runtime::events::AiEvent;
use rusty_agui::serve::{Agent, Emitter};
use rusty_agui::{Error, EventKind, Message, Role, RunAgentInput};
use rusty_json::Value;
use serde_json::{json, Value as Json};
use tokio::runtime::Handle;

pub use runtime::{KernelRuntime, Runtime};

/// The frontend tool a round proposal is reported as.
pub const DECIDE_TOOL: &str = "round_decide";

/// Default approval timeout handed to `session_run` when rounds are gated:
/// how long a session waits in the worker for the person.
pub const DEFAULT_APPROVAL_TIMEOUT_SECS: u64 = 1800;

/// A Nexus agent session behind AG-UI's [`Agent`] trait.
pub struct NexusAgent<R: Runtime> {
    runtime: Arc<R>,
    handle: Handle,
    approval: Option<u64>,
    /// Sessions waiting on a round decision, by the thread they run in.
    pending: HashMap<String, Pending>,
}

/// A session parked on a proposed round.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Pending {
    task_id: uuid::Uuid,
    session_id: String,
    round: u32,
}

impl Pending {
    /// The frontend tool call's id: names the session and the round.
    fn call_id(&self) -> String {
        format!("{}:{}", self.session_id, self.round)
    }
}

impl<R: Runtime> NexusAgent<R> {
    /// Wraps a runtime; `handle` is the tokio runtime its runs are driven on.
    pub fn new(runtime: Arc<R>, handle: Handle) -> Self {
        Self {
            runtime,
            handle,
            approval: None,
            pending: HashMap::new(),
        }
    }

    /// Gates every round on the person's decision (see the crate docs);
    /// `timeout_secs` is how long a session waits for one.
    #[must_use]
    pub fn with_approval(mut self, timeout_secs: u64) -> Self {
        self.approval = Some(timeout_secs);
        self
    }

    /// The `session_run` arguments for a user turn.
    fn session_args(&self, goal: &str, session_id: &str, input: &RunAgentInput) -> Json {
        let mut args = json!({
            "goal": goal,
            "session_id": session_id,
            "auto_approve": self.approval.is_none(),
        });
        if let Some(timeout) = self.approval {
            args["approval_timeout_secs"] = json!(timeout);
        }
        if let Some(archetype) = input
            .forwarded_props
            .get("archetype")
            .and_then(Value::as_str)
        {
            args["archetype"] = json!(archetype);
        }
        args
    }

    async fn serve(
        &mut self,
        input: &RunAgentInput,
        out: &mut Emitter<'_>,
    ) -> Result<Option<Value>, Error> {
        let turn = Turn::of(input, self.pending.get(&input.thread_id))?;
        let mut events = self.runtime.subscribe();
        let (task_id, session_id) = match turn {
            Turn::Goal(goal) => {
                let session_id = uuid::Uuid::new_v4().to_string();
                let args = self.session_args(&goal, &session_id, input);
                let task_id = self.runtime.submit(args).await.map_err(Error::Agent)?;
                (task_id, session_id)
            }
            Turn::Decision(decision) => {
                let pending = self
                    .pending
                    .remove(&input.thread_id)
                    .ok_or_else(|| Error::Agent("no session is waiting on this thread".into()))?;
                self.runtime
                    .decide(&pending.session_id, decision)
                    .await
                    .map_err(Error::Agent)?;
                (pending.task_id, pending.session_id)
            }
        };

        let mut relay = Relay::new(out, task_id, session_id.clone());
        while let Some(event) = events.next().await {
            match relay.forward(&event)? {
                Flow::Continue => {}
                Flow::Proposed(round) => {
                    self.pending.insert(
                        input.thread_id.clone(),
                        Pending {
                            task_id,
                            session_id,
                            round,
                        },
                    );
                    return Ok(None);
                }
                Flow::Finished(result) => return Ok(Some(result)),
            }
        }
        Err(Error::Agent(
            "the runtime's event stream closed before the session ended".into(),
        ))
    }
}

impl<R: Runtime> Agent for NexusAgent<R> {
    fn run(
        &mut self,
        input: &RunAgentInput,
        out: &mut Emitter<'_>,
    ) -> Result<Option<Value>, Error> {
        let handle = self.handle.clone();
        handle.block_on(self.serve(input, out))
    }
}

/// What the run's last message asks for.
enum Turn {
    /// A new session with this goal.
    Goal(String),
    /// The decision on the round the thread's session proposed.
    Decision(Json),
}

impl Turn {
    fn of(input: &RunAgentInput, pending: Option<&Pending>) -> Result<Self, Error> {
        match input.messages.last() {
            Some(Message::User { content, .. }) => {
                let goal = content.text();
                if goal.trim().is_empty() {
                    return Err(Error::Agent("the user message is empty".into()));
                }
                Ok(Turn::Goal(goal))
            }
            Some(Message::Tool {
                tool_call_id,
                content,
                ..
            }) => match pending {
                Some(pending) if pending.call_id() == *tool_call_id => {
                    Ok(Turn::Decision(decision_of(&content.text())))
                }
                _ => Err(Error::Agent(format!(
                    "tool message {tool_call_id} answers no proposed round"
                ))),
            },
            _ => Err(Error::Agent(
                "the last message must be from the user, or a tool message answering a proposed round".into(),
            )),
        }
    }
}

/// The person's answer as a `round_decide` body: a JSON object as it is,
/// an approval word as `approve_all`, anything else as an abort with reason.
fn decision_of(text: &str) -> Json {
    if let Ok(Json::Object(map)) = serde_json::from_str::<Json>(text) {
        return Json::Object(map);
    }
    let word = text.trim().trim_matches('"').to_ascii_lowercase();
    match word.as_str() {
        "approve" | "approve_all" | "approved" | "yes" | "ok" => json!({ "kind": "approve_all" }),
        _ => json!({ "kind": "abort", "reason": text.trim() }),
    }
}

/// How the relay says a run should go on.
enum Flow {
    Continue,
    /// The session proposed this round and waits.
    Proposed(u32),
    /// The session ended with this result.
    Finished(Value),
}

/// Turns one task's `AiEvent`s into AG-UI events as they arrive.
struct Relay<'e, 'a> {
    out: &'e mut Emitter<'a>,
    task_id: uuid::Uuid,
    session_id: String,
    /// The message token chunks are streaming into, if one is open.
    open: Option<String>,
    /// Whether any text was streamed, so the final text is not repeated.
    streamed: bool,
}

impl<'e, 'a> Relay<'e, 'a> {
    fn new(out: &'e mut Emitter<'a>, task_id: uuid::Uuid, session_id: String) -> Self {
        Self {
            out,
            task_id,
            session_id,
            open: None,
            streamed: false,
        }
    }

    fn forward(&mut self, event: &AiEvent) -> Result<Flow, Error> {
        if event.task_id() != self.task_id {
            return Ok(Flow::Continue);
        }
        match event {
            AiEvent::TokenChunk { text, .. } => self.chunk(text).map(|()| Flow::Continue),
            AiEvent::ToolCalled {
                tool_use_id,
                name,
                args_preview,
                ..
            } => {
                let parent = self.close_open()?;
                self.out.emit(EventKind::ToolCallStart {
                    tool_call_id: tool_use_id.clone(),
                    tool_call_name: name.clone(),
                    parent_message_id: parent,
                })?;
                self.out.emit(EventKind::ToolCallArgs {
                    tool_call_id: tool_use_id.clone(),
                    delta: args_json(args_preview),
                })?;
                self.out.emit(EventKind::ToolCallEnd {
                    tool_call_id: tool_use_id.clone(),
                })?;
                Ok(Flow::Continue)
            }
            AiEvent::ToolResult {
                tool_use_id,
                is_error,
                summary,
                ..
            } => {
                let message_id = self.out.next_id();
                let content = if *is_error {
                    json!({ "error": summary }).to_string()
                } else {
                    summary.clone()
                };
                self.out.emit(EventKind::ToolCallResult {
                    message_id,
                    tool_call_id: tool_use_id.clone(),
                    content,
                    role: Some(Role::Tool),
                })?;
                Ok(Flow::Continue)
            }
            AiEvent::RoundProposed {
                round, narration, ..
            } => {
                self.close_open()?;
                if !narration.is_empty() {
                    self.out.text(narration)?;
                    self.streamed = true;
                }
                let id = format!("{}:{round}", self.session_id);
                self.out.emit(EventKind::ToolCallStart {
                    tool_call_id: id.clone(),
                    tool_call_name: DECIDE_TOOL.into(),
                    parent_message_id: None,
                })?;
                self.out.emit(EventKind::ToolCallArgs {
                    tool_call_id: id.clone(),
                    delta: json!({ "round": round, "narration": narration }).to_string(),
                })?;
                self.out.emit(EventKind::ToolCallEnd { tool_call_id: id })?;
                Ok(Flow::Proposed(*round))
            }
            AiEvent::Finished { outcome, .. } => {
                self.close_open()?;
                if !self.streamed {
                    if let Some(text) = last_round_text(outcome) {
                        self.out.text(text)?;
                    }
                }
                let result = json!({
                    "sessionId": self.session_id,
                    "outcome": outcome.get("outcome").cloned().unwrap_or(Json::Null),
                    "tokensUsed": outcome.get("tokens_used").cloned().unwrap_or(json!(0)),
                });
                let result = rusty_json::to_value(&result)
                    .map_err(|error| Error::Agent(format!("result: {error}")))?;
                Ok(Flow::Finished(result))
            }
            AiEvent::Failed { error, .. } => {
                self.close_open()?;
                Err(Error::Agent(error.clone()))
            }
            AiEvent::Cancelled { by, .. } => {
                self.close_open()?;
                Err(Error::Agent(format!("cancelled by {by}")))
            }
            AiEvent::Submitted { .. }
            | AiEvent::Started { .. }
            | AiEvent::RoundDecided { .. }
            | AiEvent::Paused { .. }
            | AiEvent::Resumed { .. } => Ok(Flow::Continue),
        }
    }

    /// A token chunk: text goes out as a delta of one open message.
    fn chunk(&mut self, text: &str) -> Result<(), Error> {
        if text.is_empty() {
            return Ok(());
        }
        let message_id = if let Some(id) = &self.open {
            id.clone()
        } else {
            let id = self.out.next_id();
            self.out.emit(EventKind::TextMessageStart {
                message_id: id.clone(),
                role: Role::Assistant,
            })?;
            self.open = Some(id.clone());
            id
        };
        self.streamed = true;
        self.out.emit(EventKind::TextMessageContent {
            message_id,
            delta: text.into(),
        })
    }

    /// Closes the streamed message, if one is open, and returns its id.
    fn close_open(&mut self) -> Result<Option<String>, Error> {
        let Some(message_id) = self.open.take() else {
            return Ok(None);
        };
        self.out.emit(EventKind::TextMessageEnd {
            message_id: message_id.clone(),
        })?;
        Ok(Some(message_id))
    }
}

/// The runtime's arguments preview as a JSON text: as it is when it parses,
/// quoted otherwise, so the client always gets JSON.
fn args_json(preview: &str) -> String {
    if serde_json::from_str::<Json>(preview).is_ok() {
        return preview.to_string();
    }
    json!({ "preview": preview }).to_string()
}

/// The text of a finished session's last round.
fn last_round_text(session: &Json) -> Option<&str> {
    session
        .get("rounds")?
        .as_array()?
        .last()?
        .get("text")?
        .as_str()
        .filter(|text| !text.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_decision_is_approval_an_object_or_an_abort() {
        assert_eq!(decision_of("approve"), json!({ "kind": "approve_all" }));
        assert_eq!(decision_of(" Yes "), json!({ "kind": "approve_all" }));
        assert_eq!(decision_of("\"ok\""), json!({ "kind": "approve_all" }));
        assert_eq!(
            decision_of(r#"{"kind":"partial","entries":[]}"#),
            json!({ "kind": "partial", "entries": [] })
        );
        assert_eq!(
            decision_of("not today"),
            json!({ "kind": "abort", "reason": "not today" })
        );
    }

    #[test]
    fn the_arguments_preview_is_always_json() {
        assert_eq!(args_json(r#"{"path":"a.md"}"#), r#"{"path":"a.md"}"#);
        assert_eq!(args_json("path=a.md…"), r#"{"preview":"path=a.md…"}"#);
    }

    #[test]
    fn the_last_round_text_is_the_answer() {
        let session =
            json!({ "rounds": [{ "round": 1, "text": "first" }, { "round": 2, "text": "last" }] });
        assert_eq!(last_round_text(&session), Some("last"));
        assert_eq!(last_round_text(&json!({ "rounds": [] })), None);
        assert_eq!(
            last_round_text(&json!({ "rounds": [{ "text": "" }] })),
            None
        );
    }
}
