//! The assistant behind `POST /api/agent`: an AG-UI agent the web UI's chat
//! panel talks to. The first consumer of `rusty_agui` in the workspace.
//!
//! It is deterministic and holds no store. Everything it knows comes in
//! the run input: the thread and the tools the UI offers. Each command is a
//! frontend tool call: the browser runs it against the same store it already
//! uses, answers with a tool message, and the next run reports the answer.
//! The tools are named and shaped as the MCP server's (`crate::mcp`), so an
//! agent sees one vocabulary wherever it meets rusty_tick, and an LLM-backed
//! agent can replace this one behind the same trait later.

use rusty_agui::{Agent, Emitter, FunctionCall, Message, RunAgentInput};
use rusty_json::{json, Value};

/// Add a task to the list the person has open: `{title}`.
pub const ADD_TASK: &str = "add_task";
/// Tasks whose title or notes match: `{query}`.
pub const SEARCH_TASKS: &str = "search_tasks";
/// Open tasks due `today`, in the `next7` days, or `overdue`: `{view}`.
pub const DUE_TASKS: &str = "due_tasks";
/// The one task a title names, closed: `{query}`.
pub const COMPLETE_TASK: &str = "complete_task";
/// The one done task a title names, opened again: `{query}`.
pub const REOPEN_TASK: &str = "reopen_task";
/// The one task a title names, to the Trash: `{query}`.
pub const DELETE_TASK: &str = "delete_task";
/// The lists, by name: `{}`.
pub const LIST_LISTS: &str = "list_lists";

/// What the assistant decided to do with a run.
#[derive(Debug, PartialEq)]
pub enum Reply {
    /// Say this.
    Text(String),
    /// Say `say`, ask the UI to run `tool` with `args`, then wait for its answer.
    Call {
        tool: &'static str,
        args: Value,
        say: String,
    },
}

const HELP: &str = "Try “add buy milk”, “find milk”, “what's due today?”, “what's overdue?”, \
“due this week”, “complete buy milk”, “reopen buy milk”, “delete buy milk” or “lists”.";

/// A command that names a task or a query: its words, the tool it runs, the
/// argument the rest of the message becomes, and what to ask when it is missing.
struct Verb {
    prefixes: &'static [&'static str],
    tool: &'static str,
    key: &'static str,
    ask: &'static str,
}

const VERBS: [Verb; 5] = [
    Verb {
        prefixes: &["add ", "new task ", "remind me to ", "todo ", "add"],
        tool: ADD_TASK,
        key: "title",
        ask: "What should I add?",
    },
    Verb {
        prefixes: &["find ", "search for ", "search ", "find", "search"],
        tool: SEARCH_TASKS,
        key: "query",
        ask: "What should I look for?",
    },
    Verb {
        prefixes: &["complete ", "finish ", "check off ", "done ", "complete"],
        tool: COMPLETE_TASK,
        key: "query",
        ask: "Which task should I complete?",
    },
    Verb {
        prefixes: &["reopen ", "uncheck ", "reopen"],
        tool: REOPEN_TASK,
        key: "query",
        ask: "Which task should I reopen?",
    },
    Verb {
        prefixes: &["delete ", "remove ", "trash ", "delete"],
        tool: DELETE_TASK,
        key: "query",
        ask: "Which task should I delete?",
    },
];

const LISTS: [&str; 5] = [
    "lists",
    "my lists",
    "show lists",
    "show my lists",
    "what lists do i have",
];

/// Decide on a reply from the thread alone.
pub fn decide(input: &RunAgentInput) -> Reply {
    if let Some(Message::Tool {
        content,
        tool_call_id,
        ..
    }) = input.messages.last()
    {
        return Reply::Text(report(called(input, tool_call_id), &content.text()));
    }
    let text = last_user_text(input).trim().to_string();
    let lower = text.to_lowercase();
    if text.is_empty() || lower == "help" || lower.contains("what can you do") {
        return Reply::Text(HELP.into());
    }
    let reply = command(&text, &lower)
        .unwrap_or_else(|| Reply::Text(format!("I don't know that one. {HELP}")));
    match reply {
        Reply::Call { tool, .. } if !input.tools.iter().any(|t| t.name == tool) => Reply::Text(
            format!("I can do that when the Tick web app offers its {tool} tool."),
        ),
        reply => reply,
    }
}

/// The command `text` (lower-cased as `lower`) asks for, if any.
fn command(text: &str, lower: &str) -> Option<Reply> {
    for verb in &VERBS {
        let Some(prefix) = verb.prefixes.iter().find(|p| lower.starts_with(*p)) else {
            continue;
        };
        // Quotes off before the last trim, so `delete " "` asks rather than sends a blank name.
        let rest = text[prefix.len()..]
            .trim()
            .trim_matches(['“', '”', '"'])
            .trim();
        if rest.is_empty() {
            return Some(Reply::Text(verb.ask.into()));
        }
        let say = match verb.tool {
            ADD_TASK => format!("Adding “{rest}”…"),
            SEARCH_TASKS => format!("Looking for “{rest}”…"),
            _ => format!("Looking up “{rest}”…"),
        };
        let mut args = Value::object();
        args.insert(verb.key, rest);
        return Some(Reply::Call {
            tool: verb.tool,
            args,
            say,
        });
    }
    let bare = lower.trim_end_matches(['?', '.', '!']);
    if LISTS.contains(&bare) {
        return Some(Reply::Call {
            tool: LIST_LISTS,
            args: json!({}),
            say: "Reading your lists…".into(),
        });
    }
    let view = if lower.contains("overdue") {
        "overdue"
    } else if lower.contains("week") || lower.contains("next 7") {
        "next7"
    } else if lower.contains("due") || lower.contains("today") {
        "today"
    } else {
        return None;
    };
    Some(Reply::Call {
        tool: DUE_TASKS,
        args: json!({ "view": view }),
        say: "Checking…".into(),
    })
}

/// The call the tool message `tool_call_id` answers.
fn called<'a>(input: &'a RunAgentInput, tool_call_id: &str) -> Option<&'a FunctionCall> {
    input.messages.iter().rev().find_map(|m| match m {
        Message::Assistant { tool_calls, .. } => tool_calls
            .iter()
            .find(|c| c.id == tool_call_id)
            .map(|c| &c.function),
        _ => None,
    })
}

/// What to say about the UI's answer `result` to `call`.
fn report(call: Option<&FunctionCall>, result: &str) -> String {
    let parsed = Value::parse(result).unwrap_or(Value::Null);
    let name = call.map_or("", |c| c.name.as_str());
    let args = call
        .and_then(|c| Value::parse(&c.arguments).ok())
        .unwrap_or(Value::Null);
    let arg = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    if let Some(error) = parsed.get("error").and_then(Value::as_str) {
        return format!("I couldn't do that: {error}");
    }
    if let Some(matches) = strings(&parsed, "matches") {
        return format!(
            "Several tasks match “{}”: {}. Say a little more of the one you mean.",
            arg("query"),
            matches.join(", ")
        );
    }
    let title = parsed.get("title").and_then(Value::as_str).unwrap_or("it");
    match name {
        ADD_TASK => format!("Added “{title}”."),
        COMPLETE_TASK => format!("Completed “{title}”."),
        REOPEN_TASK => format!("Reopened “{title}”."),
        DELETE_TASK => format!("Moved “{title}” to the Trash."),
        LIST_LISTS => bullets(
            "Your lists:",
            "You have no lists.",
            strings(&parsed, "lists"),
        ),
        SEARCH_TASKS => bullets(
            &format!("Tasks matching “{}”:", arg("query")),
            &format!("Nothing matches “{}”.", arg("query")),
            strings(&parsed, "tasks"),
        ),
        DUE_TASKS => {
            let (heading, none) = match arg("view").as_str() {
                "overdue" => ("Overdue:", "Nothing is overdue."),
                "next7" => (
                    "Due in the next 7 days:",
                    "Nothing is due in the next 7 days.",
                ),
                _ => ("Due today:", "Nothing is due today."),
            };
            bullets(heading, none, strings(&parsed, "tasks"))
        }
        _ => "Done.".into(),
    }
}

fn strings(value: &Value, key: &str) -> Option<Vec<String>> {
    let items = value.get(key)?.as_array()?;
    Some(
        items
            .iter()
            .filter_map(Value::as_str)
            .map(String::from)
            .collect(),
    )
}

fn bullets(heading: &str, none: &str, items: Option<Vec<String>>) -> String {
    match items {
        Some(items) if !items.is_empty() => format!(
            "{heading}\n{}",
            items
                .iter()
                .map(|i| format!("- {i}"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
        _ => none.into(),
    }
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
            Reply::Call { tool, args, say } => {
                let message_id = out.text(&say)?;
                let tool_call_id = out.next_id();
                out.tool_call(
                    &tool_call_id,
                    tool,
                    &args.to_json_string(),
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
    use rusty_agui::{Content, Tool, ToolCall};

    const ALL: [&str; 7] = [
        ADD_TASK,
        SEARCH_TASKS,
        DUE_TASKS,
        COMPLETE_TASK,
        REOPEN_TASK,
        DELETE_TASK,
        LIST_LISTS,
    ];

    fn input(text: &str) -> RunAgentInput {
        let mut input = RunAgentInput::new("t", "r", vec![Message::user("u", text)]);
        input.tools = ALL
            .iter()
            .map(|name| Tool {
                name: (*name).into(),
                description: String::new(),
                parameters: json!({"type": "object"}),
            })
            .collect();
        input
    }

    fn call(tool: &'static str, args: Value) -> Reply {
        let say = command_say(tool, &args).unwrap_or_default();
        Reply::Call { tool, args, say }
    }

    /// What `command` says before a call, for building expected replies.
    fn command_say(tool: &str, args: &Value) -> Option<String> {
        let rest = args
            .get("title")
            .or_else(|| args.get("query"))
            .and_then(Value::as_str);
        Some(match (tool, rest) {
            (ADD_TASK, Some(r)) => format!("Adding “{r}”…"),
            (SEARCH_TASKS, Some(r)) => format!("Looking for “{r}”…"),
            (LIST_LISTS, _) => "Reading your lists…".into(),
            (DUE_TASKS, _) => "Checking…".into(),
            (_, Some(r)) => format!("Looking up “{r}”…"),
            _ => return None,
        })
    }

    /// `text`, answered by the UI with `result` to the call it made.
    fn answered(text: &str, result: &str) -> Reply {
        let mut thread = input(text);
        let Reply::Call { tool, args, .. } = decide(&thread) else {
            return Reply::Text("no call".into());
        };
        thread.messages.push(Message::Assistant {
            id: "a".into(),
            content: None,
            name: None,
            tool_calls: vec![ToolCall {
                id: "c".into(),
                function: FunctionCall {
                    name: tool.into(),
                    arguments: args.to_json_string(),
                },
            }],
        });
        thread.messages.push(Message::Tool {
            id: "tm".into(),
            content: Content::Text(result.into()),
            tool_call_id: "c".into(),
            error: None,
        });
        decide(&thread)
    }

    #[test]
    fn turns_each_command_into_its_tool() {
        assert_eq!(
            decide(&input("add Buy milk")),
            call(ADD_TASK, json!({"title": "Buy milk"}))
        );
        assert_eq!(
            decide(&input("Remind me to call mum")),
            call(ADD_TASK, json!({"title": "call mum"}))
        );
        assert_eq!(
            decide(&input("find milk")),
            call(SEARCH_TASKS, json!({"query": "milk"}))
        );
        assert_eq!(
            decide(&input("complete “Buy milk”")),
            call(COMPLETE_TASK, json!({"query": "Buy milk"}))
        );
        assert_eq!(
            decide(&input("reopen buy milk")),
            call(REOPEN_TASK, json!({"query": "buy milk"}))
        );
        assert_eq!(
            decide(&input("Delete buy milk")),
            call(DELETE_TASK, json!({"query": "buy milk"}))
        );
        assert_eq!(decide(&input("my lists?")), call(LIST_LISTS, json!({})));
        assert_eq!(
            decide(&input("what's overdue?")),
            call(DUE_TASKS, json!({"view": "overdue"}))
        );
        assert_eq!(
            decide(&input("what's due this week")),
            call(DUE_TASKS, json!({"view": "next7"}))
        );
        assert_eq!(
            decide(&input("what's due today?")),
            call(DUE_TASKS, json!({"view": "today"}))
        );
    }

    #[test]
    fn asks_for_what_is_missing_and_helps_otherwise() {
        assert_eq!(
            decide(&input("add")),
            Reply::Text("What should I add?".into())
        );
        assert_eq!(
            decide(&input("complete ")),
            Reply::Text("Which task should I complete?".into())
        );
        assert_eq!(
            decide(&input("delete \" \"")),
            Reply::Text("Which task should I delete?".into())
        );
        assert_eq!(
            decide(&input("complete “  ”")),
            Reply::Text("Which task should I complete?".into())
        );
        assert!(matches!(decide(&input("help")), Reply::Text(t) if t == HELP));
        assert!(
            matches!(decide(&input("sing a song")), Reply::Text(t) if t.starts_with("I don't know"))
        );
        let mut bare = input("add Buy milk");
        bare.tools.clear();
        assert!(matches!(decide(&bare), Reply::Text(t) if t.contains("add_task")));
    }

    #[test]
    fn reports_each_tools_answer() {
        assert_eq!(
            answered("add Buy milk", r#"{"title":"Buy milk","id":"x"}"#),
            Reply::Text("Added “Buy milk”.".into())
        );
        assert_eq!(
            answered("complete milk", r#"{"title":"Buy milk"}"#),
            Reply::Text("Completed “Buy milk”.".into())
        );
        assert_eq!(
            answered("delete milk", r#"{"title":"Buy milk"}"#),
            Reply::Text("Moved “Buy milk” to the Trash.".into())
        );
        assert_eq!(
            answered("complete milk", r#"{"matches":["Buy milk","Milk the cow"]}"#),
            Reply::Text("Several tasks match “milk”: Buy milk, Milk the cow. Say a little more of the one you mean.".into())
        );
        assert_eq!(
            answered("reopen milk", r#"{"error":"No done task matches “milk”."}"#),
            Reply::Text("I couldn't do that: No done task matches “milk”.".into())
        );
        assert_eq!(
            answered(
                "what's overdue?",
                r#"{"tasks":["Pay rent (due 21/10/2026)"]}"#
            ),
            Reply::Text("Overdue:\n- Pay rent (due 21/10/2026)".into())
        );
        assert_eq!(
            answered("what's due today?", r#"{"tasks":[]}"#),
            Reply::Text("Nothing is due today.".into())
        );
        assert_eq!(
            answered("find milk", r#"{"tasks":[]}"#),
            Reply::Text("Nothing matches “milk”.".into())
        );
        assert_eq!(
            answered("lists", r#"{"lists":["Inbox","Work"]}"#),
            Reply::Text("Your lists:\n- Inbox\n- Work".into())
        );
    }
}
