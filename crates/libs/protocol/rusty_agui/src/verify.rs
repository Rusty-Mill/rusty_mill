//! The protocol's ordering rules, and chunk expansion.
//!
//! A [`Verifier`] sits between a producer and a consumer. Each event it
//! accepts comes back as zero or more canonical events: a chunk event is
//! expanded into its start/content/end form (as the TypeScript SDK's
//! transform does), and everything is checked against the run's state. A
//! violation is an [`Error::Sequence`] naming the rule.
//!
//! Rules enforced:
//! - `RUN_STARTED` first, exactly once; nothing after `RUN_FINISHED` or
//!   `RUN_ERROR`. `RUN_ERROR` may arrive at any point of a running run.
//! - A text message, a tool call and a reasoning message are each
//!   opened before their content and closed with the same id, and no two
//!   of them are open at once.
//! - `STEP_FINISHED` names an open step; a run cannot finish with a
//!   message, tool call or step open.
//! - `TEXT_MESSAGE_CONTENT` and `TOOL_CALL_ARGS` deltas are non-empty.
//! - Open chunk streams close themselves on an id switch, on any other
//!   event, or at run end.

use crate::event::{Event, EventKind, EventMeta};
use crate::types::Role;
use crate::{Error, Result};

#[derive(Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    NotStarted,
    Running,
    Finished,
}

/// What is currently open and must be closed before anything else.
#[derive(Debug, PartialEq, Eq)]
enum Open {
    Text(String),
    Tool(String),
    Reasoning(String),
}

/// An open chunk stream, which the verifier itself closes.
#[derive(Debug, PartialEq, Eq)]
enum Chunk {
    Text(String),
    Tool(String),
    Reasoning(String),
}

/// A stateful checker for one run's event sequence.
#[derive(Debug, Default)]
pub struct Verifier {
    phase: Phase,
    open: Option<Open>,
    chunk: Option<Chunk>,
    steps: Vec<String>,
}

impl Verifier {
    /// A verifier for a run that has not started.
    pub fn new() -> Self {
        Self::default()
    }

    /// True once `RUN_FINISHED` or `RUN_ERROR` has been accepted.
    pub fn is_finished(&self) -> bool {
        self.phase == Phase::Finished
    }

    /// Accepts one event; returns the canonical events it stands for.
    pub fn push(&mut self, event: Event) -> Result<Vec<Event>> {
        let mut out = Vec::new();
        self.push_into(event, &mut out)?;
        Ok(out)
    }

    /// [`Verifier::push`] that appends to `out`.
    pub fn push_into(&mut self, event: Event, out: &mut Vec<Event>) -> Result<()> {
        let Event { kind, meta } = event;
        if let Some(expanded) = self.expand_chunk(&kind, &meta, out)? {
            for event in expanded {
                self.check(&event.kind)?;
                out.push(event);
            }
            return Ok(());
        }
        self.close_chunk(out)?;
        self.check(&kind)?;
        out.push(Event { kind, meta });
        Ok(())
    }

    /// Turns a chunk event into canonical events, opening or switching
    /// the chunk stream as needed. `None` when `kind` is not a chunk.
    fn expand_chunk(
        &mut self,
        kind: &EventKind,
        meta: &EventMeta,
        out: &mut Vec<Event>,
    ) -> Result<Option<Vec<Event>>> {
        let with = |kind: EventKind| Event {
            kind,
            meta: meta.clone(),
        };
        let mut events = Vec::new();
        match kind {
            EventKind::TextMessageChunk {
                message_id,
                role,
                delta,
            } => {
                let id = self.chunk_id(message_id.as_deref(), "TEXT_MESSAGE_CHUNK")?;
                let fresh = !matches!(&self.chunk, Some(Chunk::Text(open)) if *open == id);
                if fresh {
                    self.close_chunk(out)?;
                    self.chunk = Some(Chunk::Text(id.clone()));
                    events.push(with(EventKind::TextMessageStart {
                        message_id: id.clone(),
                        role: role.unwrap_or(Role::Assistant),
                    }));
                }
                if let Some(delta) = delta.as_deref().filter(|d| !d.is_empty()) {
                    events.push(with(EventKind::TextMessageContent {
                        message_id: id,
                        delta: delta.into(),
                    }));
                }
            }
            EventKind::ToolCallChunk {
                tool_call_id,
                tool_call_name,
                parent_message_id,
                delta,
            } => {
                let id = self.chunk_id(tool_call_id.as_deref(), "TOOL_CALL_CHUNK")?;
                let fresh = !matches!(&self.chunk, Some(Chunk::Tool(open)) if *open == id);
                if fresh {
                    let Some(name) = tool_call_name else {
                        return Err(Error::Sequence(
                            "the first TOOL_CALL_CHUNK of a call needs toolCallName".into(),
                        ));
                    };
                    self.close_chunk(out)?;
                    self.chunk = Some(Chunk::Tool(id.clone()));
                    events.push(with(EventKind::ToolCallStart {
                        tool_call_id: id.clone(),
                        tool_call_name: name.clone(),
                        parent_message_id: parent_message_id.clone(),
                    }));
                }
                if let Some(delta) = delta.as_deref().filter(|d| !d.is_empty()) {
                    events.push(with(EventKind::ToolCallArgs {
                        tool_call_id: id,
                        delta: delta.into(),
                    }));
                }
            }
            EventKind::ReasoningMessageChunk { message_id, delta } => {
                let id = self.chunk_id(message_id.as_deref(), "REASONING_MESSAGE_CHUNK")?;
                let fresh = !matches!(&self.chunk, Some(Chunk::Reasoning(open)) if *open == id);
                if fresh {
                    self.close_chunk(out)?;
                    self.chunk = Some(Chunk::Reasoning(id.clone()));
                    events.push(with(EventKind::ReasoningMessageStart {
                        message_id: id.clone(),
                    }));
                }
                match delta.as_deref() {
                    // An empty delta closes the reasoning message.
                    Some("") => {
                        self.chunk = None;
                        events.push(with(EventKind::ReasoningMessageEnd { message_id: id }));
                    }
                    Some(delta) => events.push(with(EventKind::ReasoningMessageContent {
                        message_id: id,
                        delta: delta.into(),
                    })),
                    None => {}
                }
            }
            _ => return Ok(None),
        }
        Ok(Some(events))
    }

    /// The id a chunk refers to: its own, or the open stream's.
    fn chunk_id(&self, given: Option<&str>, what: &str) -> Result<String> {
        if let Some(id) = given {
            return Ok(id.into());
        }
        match &self.chunk {
            Some(Chunk::Text(id) | Chunk::Tool(id) | Chunk::Reasoning(id)) => Ok(id.clone()),
            None => Err(Error::Sequence(format!(
                "{what} without an id and no chunk stream open"
            ))),
        }
    }

    /// Emits the end event for an open chunk stream, if any.
    fn close_chunk(&mut self, out: &mut Vec<Event>) -> Result<()> {
        let kind = match self.chunk.take() {
            None => return Ok(()),
            Some(Chunk::Text(message_id)) => EventKind::TextMessageEnd { message_id },
            Some(Chunk::Tool(tool_call_id)) => EventKind::ToolCallEnd { tool_call_id },
            Some(Chunk::Reasoning(message_id)) => EventKind::ReasoningMessageEnd { message_id },
        };
        self.check(&kind)?;
        out.push(kind.into());
        Ok(())
    }

    /// The ordering rules for one canonical event.
    fn check(&mut self, kind: &EventKind) -> Result<()> {
        use EventKind::*;
        match self.phase {
            Phase::NotStarted if !matches!(kind, RunStarted { .. }) => {
                return Err(Error::Sequence(format!(
                    "{} before RUN_STARTED",
                    kind.type_name()
                )));
            }
            Phase::Finished => {
                return Err(Error::Sequence(format!(
                    "{} after the run finished",
                    kind.type_name()
                )));
            }
            _ => {}
        }
        match kind {
            RunStarted { .. } => {
                if self.phase == Phase::Running {
                    return Err(Error::Sequence("RUN_STARTED twice".into()));
                }
                self.phase = Phase::Running;
            }
            RunFinished { .. } => {
                if let Some(open) = &self.open {
                    return Err(Error::Sequence(format!(
                        "RUN_FINISHED with {} open",
                        describe(open)
                    )));
                }
                if let Some(step) = self.steps.last() {
                    return Err(Error::Sequence(format!(
                        "RUN_FINISHED with step {step:?} open"
                    )));
                }
                self.phase = Phase::Finished;
            }
            RunError { .. } => self.phase = Phase::Finished,
            StepStarted { step_name } => self.steps.push(step_name.clone()),
            StepFinished { step_name } => match self.steps.iter().rposition(|s| s == step_name) {
                Some(i) => {
                    self.steps.remove(i);
                }
                None => {
                    return Err(Error::Sequence(format!(
                        "STEP_FINISHED for step {step_name:?} that is not open"
                    )));
                }
            },
            TextMessageStart { message_id, .. } => {
                self.open_new(Open::Text(message_id.clone()))?;
            }
            TextMessageContent { message_id, delta } => {
                if delta.is_empty() {
                    return Err(Error::Sequence(
                        "TEXT_MESSAGE_CONTENT with empty delta".into(),
                    ));
                }
                self.expect_open(&Open::Text(message_id.clone()), "TEXT_MESSAGE_CONTENT")?;
            }
            TextMessageEnd { message_id } => {
                self.expect_open(&Open::Text(message_id.clone()), "TEXT_MESSAGE_END")?;
                self.open = None;
            }
            ToolCallStart { tool_call_id, .. } => {
                self.open_new(Open::Tool(tool_call_id.clone()))?;
            }
            ToolCallArgs {
                tool_call_id,
                delta,
            } => {
                if delta.is_empty() {
                    return Err(Error::Sequence("TOOL_CALL_ARGS with empty delta".into()));
                }
                self.expect_open(&Open::Tool(tool_call_id.clone()), "TOOL_CALL_ARGS")?;
            }
            ToolCallEnd { tool_call_id } => {
                self.expect_open(&Open::Tool(tool_call_id.clone()), "TOOL_CALL_END")?;
                self.open = None;
            }
            ReasoningMessageStart { message_id } => {
                self.open_new(Open::Reasoning(message_id.clone()))?;
            }
            ReasoningMessageContent { message_id, .. } => {
                self.expect_open(
                    &Open::Reasoning(message_id.clone()),
                    "REASONING_MESSAGE_CONTENT",
                )?;
            }
            ReasoningMessageEnd { message_id } => {
                self.expect_open(
                    &Open::Reasoning(message_id.clone()),
                    "REASONING_MESSAGE_END",
                )?;
                self.open = None;
            }
            TextMessageChunk { .. } | ToolCallChunk { .. } | ReasoningMessageChunk { .. } => {
                unreachable!("chunks are expanded before checking")
            }
            // Everything else may appear anywhere inside a run, but not
            // while a message or tool call is streaming.
            _ => {
                if let Some(open) = &self.open {
                    return Err(Error::Sequence(format!(
                        "{} while {} is open",
                        kind.type_name(),
                        describe(open)
                    )));
                }
            }
        }
        Ok(())
    }

    fn open_new(&mut self, open: Open) -> Result<()> {
        if let Some(current) = &self.open {
            return Err(Error::Sequence(format!(
                "cannot open {} while {} is open",
                describe(&open),
                describe(current)
            )));
        }
        self.open = Some(open);
        Ok(())
    }

    fn expect_open(&self, wanted: &Open, event: &str) -> Result<()> {
        match &self.open {
            Some(current) if current == wanted => Ok(()),
            Some(current) => Err(Error::Sequence(format!(
                "{event} for {} but {} is open",
                describe(wanted),
                describe(current)
            ))),
            None => Err(Error::Sequence(format!(
                "{event} for {} that is not open",
                describe(wanted)
            ))),
        }
    }
}

fn describe(open: &Open) -> String {
    match open {
        Open::Text(id) => format!("text message {id:?}"),
        Open::Tool(id) => format!("tool call {id:?}"),
        Open::Reasoning(id) => format!("reasoning message {id:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use EventKind::*;

    fn started() -> Event {
        RunStarted {
            thread_id: "t".into(),
            run_id: "r".into(),
            parent_run_id: None,
            input: None,
        }
        .into()
    }

    fn finished() -> Event {
        RunFinished {
            thread_id: "t".into(),
            run_id: "r".into(),
            result: None,
            outcome: None,
        }
        .into()
    }

    fn text(id: &str, delta: &str) -> Event {
        TextMessageContent {
            message_id: id.into(),
            delta: delta.into(),
        }
        .into()
    }

    fn kinds(events: &[Event]) -> Vec<&'static str> {
        events.iter().map(|e| e.kind.type_name()).collect()
    }

    fn run(events: Vec<Event>) -> Result<Vec<Event>> {
        let mut verifier = Verifier::new();
        let mut out = Vec::new();
        for event in events {
            verifier.push_into(event, &mut out)?;
        }
        Ok(out)
    }

    #[test]
    fn a_well_formed_run_passes_through_unchanged() {
        let events = vec![
            started(),
            StepStarted {
                step_name: "s".into(),
            }
            .into(),
            TextMessageStart {
                message_id: "m".into(),
                role: Role::Assistant,
            }
            .into(),
            text("m", "hi"),
            TextMessageEnd {
                message_id: "m".into(),
            }
            .into(),
            ToolCallStart {
                tool_call_id: "c".into(),
                tool_call_name: "f".into(),
                parent_message_id: Some("m".into()),
            }
            .into(),
            ToolCallArgs {
                tool_call_id: "c".into(),
                delta: "{}".into(),
            }
            .into(),
            ToolCallEnd {
                tool_call_id: "c".into(),
            }
            .into(),
            StateSnapshot {
                snapshot: rusty_json::json!({}),
            }
            .into(),
            StepFinished {
                step_name: "s".into(),
            }
            .into(),
            finished(),
        ];
        let out = run(events.clone()).unwrap();
        assert_eq!(out, events);
    }

    #[test]
    fn rule_violations_are_named() {
        let cases: Vec<(Vec<Event>, &str)> = vec![
            (vec![text("m", "x")], "before RUN_STARTED"),
            (vec![started(), started()], "twice"),
            (
                vec![started(), finished(), finished()],
                "after the run finished",
            ),
            (vec![started(), text("m", "x")], "not open"),
            (
                vec![
                    started(),
                    TextMessageStart {
                        message_id: "m".into(),
                        role: Role::Assistant,
                    }
                    .into(),
                    text("m", ""),
                ],
                "empty delta",
            ),
            (
                vec![
                    started(),
                    TextMessageStart {
                        message_id: "m".into(),
                        role: Role::Assistant,
                    }
                    .into(),
                    text("other", "x"),
                ],
                "but text message \"m\" is open",
            ),
            (
                vec![
                    started(),
                    TextMessageStart {
                        message_id: "m".into(),
                        role: Role::Assistant,
                    }
                    .into(),
                    ToolCallStart {
                        tool_call_id: "c".into(),
                        tool_call_name: "f".into(),
                        parent_message_id: None,
                    }
                    .into(),
                ],
                "cannot open tool call",
            ),
            (
                vec![
                    started(),
                    TextMessageStart {
                        message_id: "m".into(),
                        role: Role::Assistant,
                    }
                    .into(),
                    finished(),
                ],
                "RUN_FINISHED with text message",
            ),
            (
                vec![
                    started(),
                    StepStarted {
                        step_name: "s".into(),
                    }
                    .into(),
                    finished(),
                ],
                "step \"s\" open",
            ),
            (
                vec![
                    started(),
                    StepFinished {
                        step_name: "s".into(),
                    }
                    .into(),
                ],
                "not open",
            ),
            (
                vec![
                    started(),
                    TextMessageStart {
                        message_id: "m".into(),
                        role: Role::Assistant,
                    }
                    .into(),
                    StateSnapshot {
                        snapshot: rusty_json::Value::Null,
                    }
                    .into(),
                ],
                "STATE_SNAPSHOT while text message",
            ),
        ];
        for (events, expected) in cases {
            match run(events) {
                Err(Error::Sequence(msg)) => {
                    assert!(msg.contains(expected), "{msg:?} vs {expected:?}")
                }
                other => panic!("expected a sequence error containing {expected:?}, got {other:?}"),
            }
        }
    }

    #[test]
    fn run_error_closes_the_run_from_anywhere() {
        let out = run(vec![
            started(),
            TextMessageStart {
                message_id: "m".into(),
                role: Role::Assistant,
            }
            .into(),
            RunError {
                message: "boom".into(),
                code: None,
            }
            .into(),
        ])
        .unwrap();
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn text_chunks_expand_and_close_on_switch_and_at_end() {
        let out = run(vec![
            started(),
            TextMessageChunk {
                message_id: Some("a".into()),
                role: None,
                delta: Some("he".into()),
            }
            .into(),
            TextMessageChunk {
                message_id: None,
                role: None,
                delta: Some("llo".into()),
            }
            .into(),
            TextMessageChunk {
                message_id: Some("b".into()),
                role: Some(Role::User),
                delta: None,
            }
            .into(),
            finished(),
        ])
        .unwrap();
        assert_eq!(
            kinds(&out),
            [
                "RUN_STARTED",
                "TEXT_MESSAGE_START",
                "TEXT_MESSAGE_CONTENT",
                "TEXT_MESSAGE_CONTENT",
                "TEXT_MESSAGE_END",
                "TEXT_MESSAGE_START",
                "TEXT_MESSAGE_END",
                "RUN_FINISHED",
            ]
        );
        assert_eq!(
            out[1].kind,
            TextMessageStart {
                message_id: "a".into(),
                role: Role::Assistant
            }
        );
        assert_eq!(
            out[5].kind,
            TextMessageStart {
                message_id: "b".into(),
                role: Role::User
            }
        );
    }

    #[test]
    fn tool_and_reasoning_chunks_expand() {
        let out = run(vec![
            started(),
            ToolCallChunk {
                tool_call_id: Some("c".into()),
                tool_call_name: Some("f".into()),
                parent_message_id: None,
                delta: Some("{\"a\"".into()),
            }
            .into(),
            ToolCallChunk {
                tool_call_id: None,
                tool_call_name: None,
                parent_message_id: None,
                delta: Some(":1}".into()),
            }
            .into(),
            ReasoningMessageChunk {
                message_id: Some("r".into()),
                delta: Some("think".into()),
            }
            .into(),
            ReasoningMessageChunk {
                message_id: None,
                delta: Some("".into()),
            }
            .into(),
            finished(),
        ])
        .unwrap();
        assert_eq!(
            kinds(&out),
            [
                "RUN_STARTED",
                "TOOL_CALL_START",
                "TOOL_CALL_ARGS",
                "TOOL_CALL_ARGS",
                "TOOL_CALL_END",
                "REASONING_MESSAGE_START",
                "REASONING_MESSAGE_CONTENT",
                "REASONING_MESSAGE_END",
                "RUN_FINISHED",
            ]
        );
    }

    #[test]
    fn chunk_errors() {
        assert!(matches!(
            run(vec![
                started(),
                TextMessageChunk {
                    message_id: None,
                    role: None,
                    delta: Some("x".into())
                }
                .into()
            ]),
            Err(Error::Sequence(_))
        ));
        assert!(matches!(
            run(vec![
                started(),
                ToolCallChunk {
                    tool_call_id: Some("c".into()),
                    tool_call_name: None,
                    parent_message_id: None,
                    delta: None
                }
                .into()
            ]),
            Err(Error::Sequence(_))
        ));
    }

    /// Chunk expansion cases shared with the TypeScript core's tests.
    #[test]
    fn chunk_fixture_expands_identically() {
        let cases = rusty_json::Value::parse(include_str!("../fixtures/chunks.json")).unwrap();
        for case in cases.as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let input: Vec<Event> = case["input"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| Event::from_value(v).unwrap())
                .collect();
            let expected: Vec<Event> = case["expected"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| Event::from_value(v).unwrap())
                .collect();
            assert_eq!(run(input).unwrap(), expected, "{name}");
        }
    }
}
