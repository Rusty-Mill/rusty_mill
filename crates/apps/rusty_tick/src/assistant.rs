//! The assistant behind `POST /api/agent`: an AG-UI agent the web UI's chat
//! panel talks to. The first consumer of `rusty_agui` in the workspace.
//!
//! It is deterministic and holds no store. Everything it knows comes in
//! the run input: the thread, the context the UI exposes (the current view
//! and its tasks), and the tools the UI offers. Adding a task is a
//! frontend tool call: the browser runs it against the same API it already
//! uses, answers with a tool message, and the next run confirms. That
//! keeps this agent sans-IO and testable as a pure function, and lets an
//! LLM-backed agent replace it behind the same trait later.

use rusty_agui::{Agent, Emitter, Message, RunAgentInput};
use rusty_json::{json, Value};

/// The tool the web UI offers for adding a task.
pub const CREATE_TASK: &str = "create_task";

/// What the assistant decided to do with a run.
#[derive(Debug, PartialEq, Eq)]
pub enum Reply {
    /// Say this.
    Text(String),
    /// Ask the UI to create a task, then wait for its answer.
    CreateTask {
        /// The task's title.
        title: String,
    },
}

const HELP: &str = "I can add tasks to the current list: try “add buy milk”. \
Ask “what's due?” and I'll read the view you have open.";

/// Decide on a reply from the thread alone.
pub fn decide(input: &RunAgentInput) -> Reply {
    if let Some(Message::Tool { content, .. }) = input.messages.last() {
        return confirm(&content.text());
    }
    let text = last_user_text(input).trim().to_string();
    let lower = text.to_lowercase();
    if text.is_empty() || lower == "help" || lower.contains("what can you do") {
        return Reply::Text(HELP.into());
    }
    if let Some(title) = add_title(&text) {
        if title.is_empty() {
            return Reply::Text("What should I add?".into());
        }
        if !input.tools.iter().any(|t| t.name == CREATE_TASK) {
            return Reply::Text(
                "I can add tasks when the Tick web app offers its create_task tool.".into(),
            );
        }
        return Reply::CreateTask {
            title: title.into(),
        };
    }
    if lower.contains("due") || lower.contains("today") || lower.contains("what's in") {
        return Reply::Text(match context(input, "tasks in view") {
            Some(tasks) if !tasks.trim().is_empty() => {
                format!("In the view you have open:\n{tasks}")
            }
            Some(_) => "The view you have open has no tasks.".into(),
            None => "Open a task view and ask again; I read what you have open.".into(),
        });
    }
    Reply::Text(format!("I only know how to add tasks for now. {HELP}"))
}

/// The confirmation after the UI answered a `create_task` call.
fn confirm(result: &str) -> Reply {
    let parsed = Value::parse(result).unwrap_or(Value::Null);
    if let Some(error) = parsed.get("error").and_then(Value::as_str) {
        return Reply::Text(format!("I could not add the task: {error}"));
    }
    match parsed.get("title").and_then(Value::as_str) {
        Some(title) => Reply::Text(format!("Added “{title}”.")),
        None => Reply::Text("Done.".into()),
    }
}

fn add_title(text: &str) -> Option<&str> {
    const PREFIXES: [&str; 5] = ["add ", "new task ", "remind me to ", "todo ", "add"];
    let lower = text.to_lowercase();
    PREFIXES
        .iter()
        .find(|p| lower.starts_with(*p))
        .map(|p| text[p.len()..].trim())
}

fn last_user_text(input: &RunAgentInput) -> String {
    input
        .messages
        .iter()
        .rev()
        .find_map(|m| match m {
            Message::User { content, .. } => Some(content.text()),
            _ => None,
        })
        .unwrap_or_default()
}

fn context<'a>(input: &'a RunAgentInput, description: &str) -> Option<&'a str> {
    input
        .context
        .iter()
        .find(|c| c.description == description)
        .map(|c| c.value.as_str())
}

/// The agent `rusty_tick` serves. Stateless; one instance serves every run.
pub struct Assistant;

impl Agent for Assistant {
    fn run(
        &mut self,
        input: &RunAgentInput,
        out: &mut Emitter<'_>,
    ) -> rusty_agui::Result<Option<Value>> {
        match decide(input) {
            Reply::Text(text) => {
                out.text(&text)?;
                Ok(None)
            }
            Reply::CreateTask { title } => {
                let message_id = out.text(&format!("Adding “{title}”…"))?;
                let tool_call_id = out.next_id();
                out.tool_call(
                    &tool_call_id,
                    CREATE_TASK,
                    &json!({ "title": title.as_str() }).to_json_string(),
                    Some(message_id),
                )?;
                Ok(None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_agui::{Content, Context, Tool};

    fn input(text: &str) -> RunAgentInput {
        RunAgentInput::new("t", "r", vec![Message::user("u", text)])
    }

    fn with_tool(mut input: RunAgentInput) -> RunAgentInput {
        input.tools.push(Tool {
            name: CREATE_TASK.into(),
            description: "Add a task".into(),
            parameters: json!({"type": "object"}),
        });
        input
    }

    #[test]
    fn adds_a_task_through_the_offered_tool() {
        assert_eq!(
            decide(&with_tool(input("add Buy milk"))),
            Reply::CreateTask {
                title: "Buy milk".into()
            }
        );
        assert_eq!(
            decide(&with_tool(input("Remind me to call mum"))),
            Reply::CreateTask {
                title: "call mum".into()
            }
        );
        assert_eq!(
            decide(&with_tool(input("add"))),
            Reply::Text("What should I add?".into())
        );
        assert!(
            matches!(decide(&input("add Buy milk")), Reply::Text(t) if t.contains("create_task"))
        );
    }

    #[test]
    fn confirms_from_the_tool_message() {
        let mut thread = with_tool(input("add Buy milk"));
        thread.messages.push(Message::Tool {
            id: "tm".into(),
            content: Content::Text(r#"{"title":"Buy milk","id":"x"}"#.into()),
            tool_call_id: "c".into(),
            error: None,
        });
        assert_eq!(decide(&thread), Reply::Text("Added “Buy milk”.".into()));

        thread.messages.push(Message::Tool {
            id: "tm2".into(),
            content: Content::Text(r#"{"error":"offline"}"#.into()),
            tool_call_id: "c2".into(),
            error: None,
        });
        assert_eq!(
            decide(&thread),
            Reply::Text("I could not add the task: offline".into())
        );
    }

    #[test]
    fn reads_the_view_from_context_and_helps_otherwise() {
        let mut due = input("what's due today?");
        due.context.push(Context {
            description: "tasks in view".into(),
            value: "- Pay rent (today)".into(),
        });
        assert_eq!(
            decide(&due),
            Reply::Text("In the view you have open:\n- Pay rent (today)".into())
        );
        assert!(
            matches!(decide(&input("what's due?")), Reply::Text(t) if t.starts_with("Open a task view"))
        );
        assert!(matches!(decide(&input("help")), Reply::Text(t) if t == HELP));
        assert!(
            matches!(decide(&input("sing a song")), Reply::Text(t) if t.starts_with("I only know"))
        );
    }
}
