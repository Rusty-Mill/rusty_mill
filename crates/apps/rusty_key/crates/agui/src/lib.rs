//! Serve a Rusty Keys session over [AG-UI][agui], the Agent-User Interaction
//! protocol: the one `rusty_agui`'s React, Vue and Angular bindings, its chat
//! channels, its routines and `rusty_agent_gateway`'s `agui` route all speak.
//!
//! [`KeyAgent`] implements `rusty_agui`'s [`Agent`] trait the way the ACP
//! adapter implements the editor protocol: one [`Session`] per thread, built
//! from the config and model with an [`ApprovalGate`] at the end of its
//! policy chain, and each run is one turn of `send_streaming`.
//! `rusty_agui::serve::AgentHandler` serves it on `rusty_serve`:
//!
//! ```no_run
//! use rk_agui::KeyAgent;
//! use rk_constrain::ApprovalTrigger;
//! use rusty_agui::serve::AgentHandler;
//!
//! let runtime = tokio::runtime::Runtime::new()?;
//! let config = rk_config::Config::resolve(|key| std::env::var(key).ok())?;
//! let model = rk_kernel::fake::FakeLanguageModel::new(vec![]); // any aisdk model
//! let agent = KeyAgent::new(runtime.handle().clone(), config, model)
//!     .with_approval(vec![ApprovalTrigger::NewFilePath, ApprovalTrigger::BashFirstUse]);
//! rusty_serve::Server::bind("127.0.0.1:8080".parse()?, AgentHandler::new(agent))?.run()?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # What maps to what
//!
//! The names are the harness's own `rk://` contract (`rk_app::contract`):
//!
//! | AG-UI | `rk://` | Note |
//! |---|---|---|
//! | `threadId` | a `Session` | built on first contact, with the thread's whole history inside it |
//! | the last user message | the turn's prompt | the session keeps the transcript; the client's copy is not replayed |
//! | `TEXT_MESSAGE_*` | `token` | streamed as deltas of one message; the reply as one message when nothing streamed |
//! | `TOOL_CALL_START`/`ARGS`/`END`, `TOOL_CALL_RESULT` | `tool_event` | the turn's tool events, after the turn, with the outcome's status and payload |
//! | a frontend tool call | `approval_request` | see below |
//! | a frontend tool call | `plan_exit` | see below |
//! | `RUN_FINISHED` result | `turn_complete` | `{reply, verified, limits}`, the boundary `TurnResult` |
//! | `RUN_ERROR` | the boundary error | the turn's error message |
//!
//! `bash_output`, `entropy` and `consolidation` are not forwarded.
//!
//! # Human-in-the-loop
//!
//! Two gates, both frontend tool calls the client renders and answers, and
//! both answered on the next run of the thread (an AG-UI run is one request):
//!
//! - **Approval.** With [`KeyAgent::with_approval`], a tool call that matches
//!   a trigger stops the turn in the gate. The run ends with a call to
//!   [`APPROVE_TOOL`] carrying the tool, its arguments and the trigger; the
//!   tool message that answers it (`allow`, `always`, or anything else to
//!   block) is delivered to the gate and the same turn is relayed on. The
//!   turn waits in the runtime meanwhile.
//! - **Plan exit.** When the agent called `exit_plan_mode`, the turn ends with
//!   a call to [`PLAN_TOOL`] carrying the plan. The answer (`proceed`,
//!   `reject`, or `annotate <feedback>`) resolves it; feedback comes back as
//!   the run's result for the client to send as the next turn.
//!
//! [agui]: https://docs.ag-ui.com

#![deny(missing_docs)]
#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::Arc;

use aisdk::core::capabilities::{TextInputSupport, ToolCallSupport};
use aisdk::core::language_model::LanguageModel;
use rk_app::contract::TurnResult;
use rk_app::{Session, TurnOutcome};
use rk_config::Config;
use rk_constrain::{
    ApprovalGate, ApprovalRequest, ApprovalResponse, ApprovalTrigger, PlanDecision,
};
use rk_observe::{ToolEvent, ToolStatus};
use rusty_agui::serve::{Agent, Emitter};
use rusty_agui::{Error, EventKind, Message, Role, RunAgentInput};
use rusty_json::Value;
use serde_json::json;
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

/// The frontend tool an approval request is reported as.
pub const APPROVE_TOOL: &str = "approve_tool";
/// The frontend tool a plan exit is reported as.
pub const PLAN_TOOL: &str = "plan_decide";

/// A Rusty Keys session per thread behind AG-UI's [`Agent`] trait.
pub struct KeyAgent<M> {
    handle: Handle,
    config: Config,
    model: M,
    triggers: Vec<ApprovalTrigger>,
    threads: HashMap<String, Thread<M>>,
}

/// One thread's session and what it is waiting on.
struct Thread<M> {
    session: Arc<Session<M>>,
    approvals: mpsc::Receiver<ApprovalRequest>,
    /// Frontend tool calls issued so far, for ids.
    calls: u32,
    /// A turn stopped in the approval gate.
    parked: Option<Parked>,
    /// The id of the plan-exit call awaiting a decision.
    plan_call: Option<String>,
}

/// A turn waiting on an approval.
struct Parked {
    call_id: String,
    turn: JoinHandle<anyhow::Result<TurnOutcome>>,
    tokens: mpsc::UnboundedReceiver<String>,
    respond: oneshot::Sender<ApprovalResponse>,
}

impl<M> KeyAgent<M>
where
    M: LanguageModel + TextInputSupport + ToolCallSupport + Clone + Send + Sync + 'static,
{
    /// Builds sessions from `config` and `model`; `handle` is the tokio runtime
    /// turns are driven on.
    pub fn new(handle: Handle, config: Config, model: M) -> Self {
        Self {
            handle,
            config,
            model,
            triggers: Vec::new(),
            threads: HashMap::new(),
        }
    }

    /// Gates the tool calls that match `triggers` on the person's approval.
    pub fn with_approval(mut self, triggers: Vec<ApprovalTrigger>) -> Self {
        self.triggers = triggers;
        self
    }

    /// The thread's session, built on first contact.
    fn thread(&mut self, thread_id: &str) -> Result<&mut Thread<M>, Error> {
        if !self.threads.contains_key(thread_id) {
            let (tx, approvals) = mpsc::channel(8);
            let gate = ApprovalGate::new(self.triggers.clone(), tx);
            let session =
                Session::new_with_policy(&self.config, self.model.clone(), Arc::new(gate))
                    .map_err(|error| Error::Agent(format!("session: {error}")))?;
            self.threads.insert(
                thread_id.to_string(),
                Thread {
                    session: Arc::new(session),
                    approvals,
                    calls: 0,
                    parked: None,
                    plan_call: None,
                },
            );
        }
        Ok(self.threads.get_mut(thread_id).expect("inserted above"))
    }

    async fn serve(
        &mut self,
        input: &RunAgentInput,
        out: &mut Emitter<'_>,
    ) -> Result<Option<Value>, Error> {
        let thread = self.thread(&input.thread_id)?;
        let turn = Turn::of(input, thread)?;
        let mut relay = Relay::new(out);
        let (handle, tokens) = match turn {
            Turn::Prompt(prompt) => {
                if thread.parked.is_some() {
                    return Err(Error::Agent(
                        "a turn is waiting on an approval; answer it first".into(),
                    ));
                }
                let (token_tx, tokens) = mpsc::unbounded_channel();
                let session = thread.session.clone();
                let handle = tokio::spawn(async move {
                    session
                        .send_streaming(&prompt, move |delta| {
                            let _ = token_tx.send(delta.to_string());
                        })
                        .await
                });
                (handle, tokens)
            }
            Turn::Approval(response) => {
                let parked = thread.parked.take().expect("checked by Turn::of");
                // A gate that stopped listening already denied the call.
                let _ = parked.respond.send(response);
                (parked.turn, parked.tokens)
            }
            Turn::Plan(decision) => {
                thread.plan_call = None;
                let feedback = thread.session.resolve_plan_exit(decision);
                let result = json!({ "plan": "resolved", "feedback": feedback });
                return to_agui(&result).map(Some);
            }
        };
        Self::pump(thread, handle, tokens, &mut relay).await
    }

    /// Relays the turn until it ends or stops in the gate.
    async fn pump(
        thread: &mut Thread<M>,
        mut turn: JoinHandle<anyhow::Result<TurnOutcome>>,
        mut tokens: mpsc::UnboundedReceiver<String>,
        relay: &mut Relay<'_, '_>,
    ) -> Result<Option<Value>, Error> {
        let outcome = loop {
            tokio::select! {
                biased;
                Some(request) = thread.approvals.recv() => {
                    relay.close_open()?;
                    thread.calls += 1;
                    let call_id = format!("approval-{}", thread.calls);
                    relay.frontend_call(&call_id, APPROVE_TOOL, &json!({
                        "tool": request.tool,
                        "args": request.args,
                        "trigger": format!("{:?}", request.trigger),
                    }))?;
                    thread.parked = Some(Parked { call_id, turn, tokens, respond: request.respond });
                    return Ok(None);
                }
                Some(delta) = tokens.recv() => relay.chunk(&delta)?,
                joined = &mut turn => {
                    while let Ok(delta) = tokens.try_recv() {
                        relay.chunk(&delta)?;
                    }
                    break joined
                        .map_err(|error| Error::Agent(format!("turn: {error}")))?
                        .map_err(|error| Error::Agent(error.to_string()))?;
                }
            }
        };
        relay.close_open()?;
        if !relay.streamed && !outcome.reply.is_empty() {
            relay.out.text(&outcome.reply)?;
        }
        for event in thread.session.last_tool_events() {
            relay.tool_event(&event)?;
        }
        if let Some(plan) = thread.session.plan_exit_pending() {
            thread.calls += 1;
            let call_id = format!("plan-{}", thread.calls);
            relay.frontend_call(&call_id, PLAN_TOOL, &json!({ "plan": plan }))?;
            thread.plan_call = Some(call_id);
        }
        to_agui(&TurnResult::from_outcome(&outcome)).map(Some)
    }
}

impl<M> Agent for KeyAgent<M>
where
    M: LanguageModel + TextInputSupport + ToolCallSupport + Clone + Send + Sync + 'static,
{
    fn run(
        &mut self,
        input: &RunAgentInput,
        out: &mut Emitter<'_>,
    ) -> Result<Option<Value>, Error> {
        let handle = self.handle.clone();
        handle.block_on(self.serve(input, out))
    }
}

fn to_agui<T: serde::Serialize>(value: &T) -> Result<Value, Error> {
    rusty_json::to_value(value).map_err(|error| Error::Agent(format!("value: {error}")))
}

/// What the run's last message asks for.
enum Turn {
    /// A new turn with this prompt.
    Prompt(String),
    /// The answer to the parked turn's approval request.
    Approval(ApprovalResponse),
    /// The decision on the pending plan exit.
    Plan(PlanDecision),
}

impl Turn {
    fn of<M>(input: &RunAgentInput, thread: &Thread<M>) -> Result<Self, Error> {
        match input.messages.last() {
            Some(Message::User { content, .. }) => {
                let prompt = content.text();
                if prompt.trim().is_empty() {
                    return Err(Error::Agent("the user message is empty".into()));
                }
                Ok(Turn::Prompt(prompt))
            }
            Some(Message::Tool {
                tool_call_id,
                content,
                ..
            }) => {
                let answer = content.text();
                if thread.parked.as_ref().is_some_and(|p| p.call_id == *tool_call_id) {
                    return Ok(Turn::Approval(approval_of(&answer)));
                }
                if thread.plan_call.as_deref() == Some(tool_call_id.as_str()) {
                    return Ok(Turn::Plan(plan_decision_of(&answer)));
                }
                Err(Error::Agent(format!(
                    "tool message {tool_call_id} answers no pending approval or plan exit"
                )))
            }
            _ => Err(Error::Agent(
                "the last message must be from the user, or a tool message answering a pending approval or plan exit".into(),
            )),
        }
    }
}

/// The person's answer to an approval request. Anything but an allow blocks.
fn approval_of(text: &str) -> ApprovalResponse {
    match text.trim().trim_matches('"').to_ascii_lowercase().as_str() {
        "allow" | "yes" | "approve" | "ok" => ApprovalResponse::Allow,
        "always" | "allow_always" | "allow always" => ApprovalResponse::AllowAlways,
        _ => ApprovalResponse::Block,
    }
}

/// The person's answer to a plan exit, as the desktop reads it.
fn plan_decision_of(text: &str) -> PlanDecision {
    match text.trim().trim_matches('"') {
        "" | "proceed" | "yes" | "approve" => PlanDecision::Proceed,
        other => match other.strip_prefix("annotate ") {
            Some(note) => PlanDecision::Annotate(note.trim().to_string()),
            None => PlanDecision::Reject,
        },
    }
}

/// Turns the turn's output into AG-UI events as it arrives.
struct Relay<'e, 'a> {
    out: &'e mut Emitter<'a>,
    /// The message token deltas are streaming into, if one is open.
    open: Option<String>,
    /// Whether any text was streamed, so the reply is not repeated.
    streamed: bool,
}

impl<'e, 'a> Relay<'e, 'a> {
    fn new(out: &'e mut Emitter<'a>) -> Self {
        Self {
            out,
            open: None,
            streamed: false,
        }
    }

    /// A token delta: text goes out as a delta of one open message.
    fn chunk(&mut self, delta: &str) -> Result<(), Error> {
        if delta.is_empty() {
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
            delta: delta.into(),
        })
    }

    /// Closes the streamed message, if one is open.
    fn close_open(&mut self) -> Result<(), Error> {
        if let Some(message_id) = self.open.take() {
            self.out.emit(EventKind::TextMessageEnd { message_id })?;
        }
        Ok(())
    }

    /// One of the turn's tool events: the call and its outcome.
    fn tool_event(&mut self, event: &ToolEvent) -> Result<(), Error> {
        let call_id = self.out.next_id();
        self.out.emit(EventKind::ToolCallStart {
            tool_call_id: call_id.clone(),
            tool_call_name: event.name.clone(),
            parent_message_id: None,
        })?;
        self.out.emit(EventKind::ToolCallArgs {
            tool_call_id: call_id.clone(),
            delta: event.args.to_string(),
        })?;
        self.out.emit(EventKind::ToolCallEnd {
            tool_call_id: call_id.clone(),
        })?;
        let content = match event.outcome.status {
            ToolStatus::Ok => event.outcome.payload.clone(),
            status => {
                json!({ "status": status.as_str(), "payload": event.outcome.payload }).to_string()
            }
        };
        let message_id = self.out.next_id();
        self.out.emit(EventKind::ToolCallResult {
            message_id,
            tool_call_id: call_id,
            content,
            role: Some(Role::Tool),
        })
    }

    /// A frontend tool call the client renders and answers.
    fn frontend_call(
        &mut self,
        call_id: &str,
        name: &str,
        args: &serde_json::Value,
    ) -> Result<(), Error> {
        self.out.emit(EventKind::ToolCallStart {
            tool_call_id: call_id.into(),
            tool_call_name: name.into(),
            parent_message_id: None,
        })?;
        self.out.emit(EventKind::ToolCallArgs {
            tool_call_id: call_id.into(),
            delta: args.to_string(),
        })?;
        self.out.emit(EventKind::ToolCallEnd {
            tool_call_id: call_id.into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_approval_answer_allows_always_or_blocks() {
        assert_eq!(approval_of("allow"), ApprovalResponse::Allow);
        assert_eq!(approval_of(" Yes "), ApprovalResponse::Allow);
        assert_eq!(approval_of("\"always\""), ApprovalResponse::AllowAlways);
        assert_eq!(approval_of("no"), ApprovalResponse::Block);
        assert_eq!(approval_of(""), ApprovalResponse::Block);
    }

    #[test]
    fn a_plan_answer_proceeds_rejects_or_annotates() {
        assert!(matches!(plan_decision_of("proceed"), PlanDecision::Proceed));
        assert!(matches!(plan_decision_of(""), PlanDecision::Proceed));
        assert!(matches!(plan_decision_of("reject"), PlanDecision::Reject));
        assert!(
            matches!(plan_decision_of("annotate add tests"), PlanDecision::Annotate(note) if note == "add tests")
        );
    }
}
