//! Folds a verified event stream into what a client shows: the message
//! list and the shared state.

use crate::event::{Event, EventKind};
use crate::types::{Content, FunctionCall, Message, Role, ToolCall};
use crate::{Error, Result};
use rusty_json::Value;
use rusty_json_patch::{merge_patch, Patch};

/// The client-side view of a thread: messages and state.
///
/// Feed it canonical events (from a [`crate::Verifier`]); it does not
/// expand chunks or check ordering itself. Events it does not model
/// (lifecycle, steps, reasoning phases, subagents, raw, custom) are
/// accepted and ignored.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reducer {
    /// The thread, oldest first.
    pub messages: Vec<Message>,
    /// The shared state, `Null` until a snapshot arrives.
    pub state: Value,
}

impl Reducer {
    /// A reducer with no messages and null state.
    pub fn new() -> Self {
        Self::default()
    }

    /// A reducer seeded from a run input's messages and state, the way a
    /// client resumes a thread.
    pub fn from_input(input: &crate::RunAgentInput) -> Self {
        Reducer {
            messages: input.messages.clone(),
            state: input.state.clone(),
        }
    }

    /// Applies one event.
    pub fn apply(&mut self, event: &Event) -> Result<()> {
        use EventKind::*;
        match &event.kind {
            TextMessageStart { message_id, role } => {
                self.messages.push(new_message(message_id, *role))
            }
            TextMessageContent { message_id, delta } => self.append_text(message_id, delta)?,
            ToolCallStart {
                tool_call_id,
                tool_call_name,
                parent_message_id,
            } => self.start_tool_call(tool_call_id, tool_call_name, parent_message_id.as_deref()),
            ToolCallArgs {
                tool_call_id,
                delta,
            } => self.append_args(tool_call_id, delta)?,
            ToolCallResult {
                message_id,
                tool_call_id,
                content,
                ..
            } => self.messages.push(Message::Tool {
                id: message_id.clone(),
                content: Content::Text(content.clone()),
                tool_call_id: tool_call_id.clone(),
                error: None,
            }),
            StateSnapshot { snapshot } => self.state = snapshot.clone(),
            StateDelta { delta } => Patch::from_value(delta)?.apply(&mut self.state)?,
            MessagesSnapshot { messages } => self.messages = messages.clone(),
            ActivitySnapshot {
                message_id,
                activity_type,
                content,
                replace,
            } => {
                self.activity_snapshot(message_id, activity_type, content, replace.unwrap_or(true))
            }
            ActivityDelta {
                message_id, patch, ..
            } => {
                let Some(Message::Activity { content, .. }) = self.find_mut(message_id) else {
                    return Err(Error::Sequence(format!(
                        "ACTIVITY_DELTA for unknown activity {message_id:?}"
                    )));
                };
                Patch::from_value(patch)?.apply(content)?;
            }
            ReasoningMessageStart { message_id } => self.messages.push(Message::Reasoning {
                id: message_id.clone(),
                content: String::new(),
                encrypted_value: None,
            }),
            ReasoningMessageContent { message_id, delta } => self.append_text(message_id, delta)?,
            ReasoningEncryptedValue {
                entity_id,
                encrypted_value,
                ..
            } => {
                if let Some(Message::Reasoning {
                    encrypted_value: slot,
                    ..
                }) = self.find_mut(entity_id)
                {
                    *slot = Some(encrypted_value.clone());
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Applies every event in order, stopping at the first error.
    pub fn apply_all<'a>(&mut self, events: impl IntoIterator<Item = &'a Event>) -> Result<()> {
        events.into_iter().try_for_each(|e| self.apply(e))
    }

    /// The message with this id, if any.
    pub fn find(&self, id: &str) -> Option<&Message> {
        self.messages.iter().rev().find(|m| m.id() == id)
    }

    fn find_mut(&mut self, id: &str) -> Option<&mut Message> {
        self.messages.iter_mut().rev().find(|m| m.id() == id)
    }

    fn append_text(&mut self, id: &str, delta: &str) -> Result<()> {
        let Some(message) = self.find_mut(id) else {
            return Err(Error::Sequence(format!(
                "content for unknown message {id:?}"
            )));
        };
        match message {
            Message::Assistant { content, .. } => {
                content.get_or_insert_with(String::new).push_str(delta)
            }
            Message::System { content, .. }
            | Message::Developer { content, .. }
            | Message::Reasoning { content, .. } => content.push_str(delta),
            Message::User { content, .. } | Message::Tool { content, .. } => match content {
                Content::Text(text) => text.push_str(delta),
                Content::Parts(parts) => parts.push(crate::ContentPart::Text(delta.into())),
            },
            Message::Activity { .. } => {
                return Err(Error::Sequence(format!(
                    "text content for activity message {id:?}"
                )))
            }
        }
        Ok(())
    }

    /// Attaches a new tool call to its parent assistant message, the
    /// last assistant message, or a fresh one.
    fn start_tool_call(&mut self, id: &str, name: &str, parent: Option<&str>) {
        let call = ToolCall {
            id: id.into(),
            function: FunctionCall {
                name: name.into(),
                arguments: String::new(),
            },
        };
        let parent_index = match parent {
            Some(parent) => self.messages.iter().rposition(|m| m.id() == parent),
            None => self
                .messages
                .iter()
                .rposition(|m| matches!(m, Message::Assistant { .. })),
        };
        if let Some(Message::Assistant { tool_calls, .. }) =
            parent_index.and_then(|i| self.messages.get_mut(i))
        {
            tool_calls.push(call);
            return;
        }
        self.messages.push(Message::Assistant {
            id: parent.unwrap_or(id).into(),
            content: None,
            name: None,
            tool_calls: vec![call],
        });
    }

    fn append_args(&mut self, id: &str, delta: &str) -> Result<()> {
        let call = self.messages.iter_mut().rev().find_map(|m| match m {
            Message::Assistant { tool_calls, .. } => tool_calls.iter_mut().find(|c| c.id == id),
            _ => None,
        });
        match call {
            Some(call) => {
                call.function.arguments.push_str(delta);
                Ok(())
            }
            None => Err(Error::Sequence(format!(
                "args for unknown tool call {id:?}"
            ))),
        }
    }

    fn activity_snapshot(&mut self, id: &str, activity_type: &str, content: &Value, replace: bool) {
        if let Some(Message::Activity {
            activity_type: kind,
            content: existing,
            ..
        }) = self.find_mut(id)
        {
            *kind = activity_type.into();
            if replace {
                *existing = content.clone();
            } else {
                merge_patch(existing, content);
            }
            return;
        }
        self.messages.push(Message::Activity {
            id: id.into(),
            activity_type: activity_type.into(),
            content: content.clone(),
        });
    }
}

fn new_message(id: &str, role: Role) -> Message {
    let id = id.to_string();
    match role {
        Role::Assistant => Message::Assistant {
            id,
            content: Some(String::new()),
            name: None,
            tool_calls: Vec::new(),
        },
        Role::User => Message::User {
            id,
            content: Content::Text(String::new()),
            name: None,
        },
        Role::System => Message::System {
            id,
            content: String::new(),
            name: None,
        },
        Role::Developer => Message::Developer {
            id,
            content: String::new(),
            name: None,
        },
        Role::Tool => Message::Tool {
            id,
            content: Content::Text(String::new()),
            tool_call_id: String::new(),
            error: None,
        },
        Role::Reasoning => Message::Reasoning {
            id,
            content: String::new(),
            encrypted_value: None,
        },
        Role::Activity => Message::Activity {
            id,
            activity_type: String::new(),
            content: Value::object(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_json::json;
    use EventKind::*;

    fn apply(events: Vec<EventKind>) -> Reducer {
        let mut reducer = Reducer::new();
        for kind in events {
            reducer.apply(&kind.into()).unwrap();
        }
        reducer
    }

    #[test]
    fn text_streams_into_an_assistant_message() {
        let reducer = apply(vec![
            TextMessageStart {
                message_id: "m".into(),
                role: Role::Assistant,
            },
            TextMessageContent {
                message_id: "m".into(),
                delta: "Hel".into(),
            },
            TextMessageContent {
                message_id: "m".into(),
                delta: "lo".into(),
            },
            TextMessageEnd {
                message_id: "m".into(),
            },
        ]);
        assert_eq!(reducer.messages, vec![Message::assistant("m", "Hello")]);
    }

    #[test]
    fn tool_calls_attach_to_their_parent_and_results_follow() {
        let reducer = apply(vec![
            TextMessageStart {
                message_id: "m".into(),
                role: Role::Assistant,
            },
            TextMessageContent {
                message_id: "m".into(),
                delta: "Checking".into(),
            },
            TextMessageEnd {
                message_id: "m".into(),
            },
            ToolCallStart {
                tool_call_id: "c".into(),
                tool_call_name: "weather".into(),
                parent_message_id: Some("m".into()),
            },
            ToolCallArgs {
                tool_call_id: "c".into(),
                delta: "{\"city\":".into(),
            },
            ToolCallArgs {
                tool_call_id: "c".into(),
                delta: "\"Oslo\"}".into(),
            },
            ToolCallEnd {
                tool_call_id: "c".into(),
            },
            ToolCallResult {
                message_id: "t".into(),
                tool_call_id: "c".into(),
                content: "cold".into(),
                role: Some(Role::Tool),
            },
        ]);
        assert_eq!(reducer.messages.len(), 2);
        let Message::Assistant { tool_calls, .. } = &reducer.messages[0] else {
            panic!()
        };
        assert_eq!(tool_calls[0].function.arguments, "{\"city\":\"Oslo\"}");
        assert!(
            matches!(&reducer.messages[1], Message::Tool { tool_call_id, .. } if tool_call_id == "c")
        );
    }

    #[test]
    fn an_orphan_tool_call_gets_its_own_assistant_message() {
        let reducer = apply(vec![ToolCallStart {
            tool_call_id: "c".into(),
            tool_call_name: "f".into(),
            parent_message_id: None,
        }]);
        assert!(
            matches!(&reducer.messages[0], Message::Assistant { id, content: None, .. } if id == "c")
        );
    }

    #[test]
    fn state_snapshot_then_delta() {
        let reducer = apply(vec![
            StateSnapshot {
                snapshot: json!({"count": 1, "items": []}),
            },
            StateDelta {
                delta: json!([
                    {"op": "replace", "path": "/count", "value": 2},
                    {"op": "add", "path": "/items/-", "value": "x"}
                ]),
            },
        ]);
        assert_eq!(reducer.state, json!({"count": 2, "items": ["x"]}));
    }

    #[test]
    fn a_bad_delta_is_an_error_and_leaves_state_alone() {
        let mut reducer = apply(vec![StateSnapshot {
            snapshot: json!({"a": 1}),
        }]);
        let err = reducer
            .apply(
                &StateDelta {
                    delta: json!([{"op": "remove", "path": "/missing"}]),
                }
                .into(),
            )
            .unwrap_err();
        assert!(matches!(err, Error::Patch(_)));
        assert_eq!(reducer.state, json!({"a": 1}));
    }

    #[test]
    fn activities_snapshot_merge_and_patch() {
        let reducer = apply(vec![
            ActivitySnapshot {
                message_id: "a".into(),
                activity_type: "PLAN".into(),
                content: json!({"steps": ["one"]}),
                replace: None,
            },
            ActivitySnapshot {
                message_id: "a".into(),
                activity_type: "PLAN".into(),
                content: json!({"done": false}),
                replace: Some(false),
            },
            ActivityDelta {
                message_id: "a".into(),
                activity_type: "PLAN".into(),
                patch: json!([{"op": "replace", "path": "/done", "value": true}]),
            },
        ]);
        assert_eq!(
            reducer.messages,
            vec![Message::Activity {
                id: "a".into(),
                activity_type: "PLAN".into(),
                content: json!({"steps": ["one"], "done": true}),
            }]
        );
    }

    #[test]
    fn messages_snapshot_replaces_and_reasoning_accumulates() {
        let reducer = apply(vec![
            MessagesSnapshot {
                messages: vec![Message::user("u", "hi")],
            },
            ReasoningMessageStart {
                message_id: "r".into(),
            },
            ReasoningMessageContent {
                message_id: "r".into(),
                delta: "think".into(),
            },
            ReasoningMessageEnd {
                message_id: "r".into(),
            },
            ReasoningEncryptedValue {
                subtype: "message".into(),
                entity_id: "r".into(),
                encrypted_value: "blob".into(),
            },
        ]);
        assert_eq!(
            reducer.messages,
            vec![
                Message::user("u", "hi"),
                Message::Reasoning {
                    id: "r".into(),
                    content: "think".into(),
                    encrypted_value: Some("blob".into()),
                }
            ]
        );
    }

    /// Whole runs with their expected reduction, shared with the TypeScript
    /// core's tests.
    #[test]
    fn run_fixture_reduces_identically() {
        let cases = rusty_json::Value::parse(include_str!("../fixtures/runs.json")).unwrap();
        for case in cases.as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let input = crate::RunAgentInput::from_value(&case["input"]).unwrap();
            let mut reducer = Reducer::from_input(&input);
            let mut verifier = crate::Verifier::new();
            for raw in case["events"].as_array().unwrap() {
                for event in verifier.push(Event::from_value(raw).unwrap()).unwrap() {
                    reducer.apply(&event).unwrap();
                }
            }
            let messages: Vec<Value> = reducer.messages.iter().map(Message::to_value).collect();
            assert_eq!(Value::Array(messages), case["messages"], "{name}: messages");
            assert_eq!(reducer.state, case["state"], "{name}: state");
        }
    }
}
