//! Chat channels for AG-UI agents.
//!
//! A channel turns a message from a chat service into a run of an AG-UI
//! agent and the run's reply into a message back. This crate holds the
//! mapping and nothing else: a [`Channel`] reads an inbound HTTP request
//! and describes the outbound one, and a [`Thread`] keeps one
//! conversation's history, builds each [`RunAgentInput`] and absorbs the
//! events that come back. No sockets, no clocks, no secrets read from the
//! environment: the runner that owns the I/O supplies them, so every rule
//! here is tested without a network. `examples/slack_bot.rs` is one such
//! runner.
//!
//! [`slack`] is the first adapter. Teams and SMS follow the same shape.

pub mod slack;

use std::fmt;

use rusty_agui::{Content, Event, EventKind, Message, Reducer, RunAgentInput, Verifier};
use rusty_json::Value;

/// What can go wrong between the chat service and the agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The request did not prove it came from the service.
    Signature(String),
    /// The request was signed but not readable as the service's payload.
    Payload(String),
    /// The agent's run ended badly: a `RUN_ERROR`, a broken stream, or an
    /// event out of order.
    Run(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Signature(why) => write!(f, "signature: {why}"),
            Error::Payload(why) => write!(f, "payload: {why}"),
            Error::Run(why) => write!(f, "run: {why}"),
        }
    }
}

impl std::error::Error for Error {}

/// Request headers, looked up by name without regard to case.
pub trait Headers {
    /// The first value for `name`.
    fn get(&self, name: &str) -> Option<&str>;
}

impl<const N: usize> Headers for [(&str, &str); N] {
    fn get(&self, name: &str) -> Option<&str> {
        self.iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| *v)
    }
}

impl Headers for Vec<(String, String)> {
    fn get(&self, name: &str) -> Option<&str> {
        self.iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// A message a person sent through a channel.
#[derive(Debug, Clone, PartialEq)]
pub struct Inbound {
    /// The conversation this belongs to, unique within the channel; the
    /// [`Thread`] key.
    pub conversation: String,
    /// Who sent it, as the service names them.
    pub sender: String,
    /// The text, cleaned of the service's markup.
    pub text: String,
    /// Where a reply goes; opaque to the runner, read by [`Channel::reply`].
    pub reply_to: Value,
}

/// What an inbound request turned out to be.
#[derive(Debug, Clone, PartialEq)]
pub enum Received {
    /// The service is checking the endpoint; answer with this challenge.
    Challenge(String),
    /// A message to run.
    Message(Inbound),
    /// Authentic, but nothing to do (a bot's own message, a retry, an
    /// event this channel does not handle); the reason is for the log.
    Ignored(&'static str),
}

/// An HTTP `POST` the runner sends to the service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outbound {
    /// Absolute URL.
    pub url: String,
    /// Headers, `Content-Type` included.
    pub headers: Vec<(String, String)>,
    /// The body.
    pub body: Vec<u8>,
}

/// A chat service, as the runner sees it. Sans-IO: `receive` reads a
/// request the runner accepted and `reply` describes one the runner sends.
pub trait Channel {
    /// A short name, used as the thread id prefix and in logs.
    fn name(&self) -> &'static str;

    /// Authenticate and read one inbound request. `now` is seconds since
    /// the epoch, for replay protection.
    fn receive(&self, headers: &dyn Headers, body: &[u8], now: u64) -> Result<Received, Error>;

    /// The request that sends `text` back where `to` came from.
    fn reply(&self, to: &Inbound, text: &str) -> Outbound;
}

/// What a run said back, in the channel's terms.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Reply {
    /// The agent's new assistant text, messages joined by blank lines.
    pub text: String,
    /// Set when the run ended in `RUN_ERROR`, a broken stream, or an event
    /// out of order; `text` holds whatever arrived before.
    pub error: Option<String>,
}

/// One conversation's history: the thread the agent sees on every run.
#[derive(Debug, Clone, PartialEq)]
pub struct Thread {
    /// The AG-UI thread id: `<channel>:<conversation>`.
    pub id: String,
    /// Every message so far, in order.
    pub messages: Vec<Message>,
}

impl Thread {
    /// An empty thread for `conversation` on `channel`.
    pub fn new(channel: &str, conversation: &str) -> Self {
        Thread {
            id: format!("{channel}:{conversation}"),
            messages: Vec::new(),
        }
    }

    /// Append the person's message and build the run that answers it.
    /// The run carries the whole thread, no tools (a channel has nothing
    /// to run on the person's side) and no context.
    pub fn run(&mut self, inbound: &Inbound) -> RunAgentInput {
        self.messages.push(Message::User {
            id: rusty_uuid::Uuid::new_v4().to_string(),
            content: Content::Text(inbound.text.clone()),
            name: Some(inbound.sender.clone()),
        });
        RunAgentInput::new(
            self.id.clone(),
            rusty_uuid::Uuid::new_v4().to_string(),
            self.messages.clone(),
        )
    }

    /// Fold a run's events into the thread and return what to send back.
    /// Events are verified (and chunks expanded) on the way in, so a
    /// stream from any source is held to the protocol's ordering rules.
    /// Stops at the first failure; the thread keeps what was folded
    /// before it, so the next run still sees the partial reply.
    pub fn absorb(&mut self, events: impl IntoIterator<Item = rusty_agui::Result<Event>>) -> Reply {
        let before: Vec<String> = self.messages.iter().map(message_id).collect();
        let mut reducer = Reducer::new();
        reducer.messages = std::mem::take(&mut self.messages);
        let mut verifier = Verifier::new();
        let error = events.into_iter().find_map(|event| {
            let canonical = match event.and_then(|e| verifier.push(e)) {
                Ok(canonical) => canonical,
                Err(e) => return Some(e.to_string()),
            };
            canonical.into_iter().find_map(|event| {
                if let EventKind::RunError { message, .. } = &event.kind {
                    return Some(message.clone());
                }
                reducer.apply(&event).err().map(|e| e.to_string())
            })
        });
        self.messages = reducer.messages;
        let text = self
            .messages
            .iter()
            .filter(|m| !before.contains(&message_id(m)))
            .filter_map(|m| match m {
                Message::Assistant {
                    content: Some(text),
                    ..
                } if !text.is_empty() => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        Reply { text, error }
    }
}

fn message_id(message: &Message) -> String {
    match message {
        Message::User { id, .. }
        | Message::Assistant { id, .. }
        | Message::System { id, .. }
        | Message::Developer { id, .. }
        | Message::Tool { id, .. }
        | Message::Activity { id, .. }
        | Message::Reasoning { id, .. } => id.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inbound(text: &str) -> Inbound {
        Inbound {
            conversation: "c1".into(),
            sender: "U1".into(),
            text: text.into(),
            reply_to: Value::Null,
        }
    }

    fn events(json: &[&str]) -> Vec<rusty_agui::Result<Event>> {
        json.iter().map(|e| Event::from_json(e)).collect()
    }

    #[test]
    fn a_run_carries_the_whole_thread_and_no_tools() {
        let mut thread = Thread::new("slack", "c1");
        let first = thread.run(&inbound("hello"));
        assert_eq!(first.thread_id, "slack:c1");
        assert_eq!(first.messages.len(), 1);
        assert!(first.tools.is_empty() && first.context.is_empty());

        let second = thread.run(&inbound("again"));
        assert_eq!(second.thread_id, first.thread_id);
        assert_ne!(second.run_id, first.run_id);
        assert_eq!(second.messages.len(), 2);
        assert!(
            matches!(&second.messages[1], Message::User { content: Content::Text(t), name: Some(n), .. } if t == "again" && n == "U1")
        );
    }

    #[test]
    fn absorb_returns_the_new_assistant_text_and_keeps_it() {
        let mut thread = Thread::new("slack", "c1");
        let _ = thread.run(&inbound("hello"));
        let reply = thread.absorb(events(&[
            r#"{"type":"RUN_STARTED","threadId":"slack:c1","runId":"r"}"#,
            r#"{"type":"TEXT_MESSAGE_START","messageId":"a1","role":"assistant"}"#,
            r#"{"type":"TEXT_MESSAGE_CONTENT","messageId":"a1","delta":"Hi "}"#,
            r#"{"type":"TEXT_MESSAGE_CONTENT","messageId":"a1","delta":"there"}"#,
            r#"{"type":"TEXT_MESSAGE_END","messageId":"a1"}"#,
            r#"{"type":"TEXT_MESSAGE_START","messageId":"a2","role":"assistant"}"#,
            r#"{"type":"TEXT_MESSAGE_CONTENT","messageId":"a2","delta":"More."}"#,
            r#"{"type":"TEXT_MESSAGE_END","messageId":"a2"}"#,
            r#"{"type":"RUN_FINISHED","threadId":"slack:c1","runId":"r"}"#,
        ]));
        assert_eq!(
            reply,
            Reply {
                text: "Hi there\n\nMore.".into(),
                error: None
            }
        );
        assert_eq!(
            thread.messages.len(),
            3,
            "the reply is part of the thread now"
        );

        // The next run's reply is only the next run's text.
        let _ = thread.run(&inbound("and?"));
        let reply = thread.absorb(events(&[
            r#"{"type":"RUN_STARTED","threadId":"slack:c1","runId":"r2"}"#,
            r#"{"type":"TEXT_MESSAGE_CHUNK","messageId":"a3","delta":"Only this."}"#,
            r#"{"type":"RUN_FINISHED","threadId":"slack:c1","runId":"r2"}"#,
        ]));
        assert_eq!(reply.text, "Only this.");
        assert_eq!(thread.messages.len(), 5);
    }

    #[test]
    fn absorb_reports_errors_and_keeps_the_partial_reply() {
        let mut thread = Thread::new("slack", "c1");
        let _ = thread.run(&inbound("hello"));
        let reply = thread.absorb(events(&[
            r#"{"type":"RUN_STARTED","threadId":"slack:c1","runId":"r"}"#,
            r#"{"type":"TEXT_MESSAGE_CHUNK","messageId":"a1","delta":"Partial"}"#,
            r#"{"type":"RUN_ERROR","message":"boom"}"#,
        ]));
        assert_eq!(
            reply,
            Reply {
                text: "Partial".into(),
                error: Some("boom".into())
            }
        );

        let mut stream = events(&[r#"{"type":"RUN_STARTED","threadId":"slack:c1","runId":"r"}"#]);
        stream.push(Err(rusty_agui::Error::Transport("closed".into())));
        let reply = thread.absorb(stream);
        assert_eq!(reply.text, "");
        assert!(reply.error.as_deref().is_some_and(|e| e.contains("closed")));
    }

    #[test]
    fn headers_are_case_insensitive() {
        let fixed = [("X-Slack-Signature", "v0=abc")];
        assert_eq!(Headers::get(&fixed, "x-slack-signature"), Some("v0=abc"));
        assert_eq!(Headers::get(&fixed, "other"), None);
        let owned = vec![("Content-Type".to_string(), "x".to_string())];
        assert_eq!(Headers::get(&owned, "content-type"), Some("x"));
    }
}
