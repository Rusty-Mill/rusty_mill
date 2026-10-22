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
/// The one task a title names, renamed (`{query, title}`) or given a new due
/// day (`{query, due}`: the day as said, read by the browser in its own zone).
pub const UPDATE_TASK: &str = "update_task";
/// Tasks of a status (open unless named), priority, tag and/or list, in a view
/// by due date: `{status?, priority?, tag?, list?, view}` (`all`, `today`,
/// `next7` or `overdue`). Browser-only: an MCP agent filters `list_tasks` itself.
pub const FILTER_TASKS: &str = "filter_tasks";
/// Habits are client documents with no API route, so these three run only in
/// the browser and have no MCP twin.
/// Today's habits and their progress: `{}`.
pub const LIST_HABITS: &str = "list_habits";
/// One more check-in today for the habit a name names, or none (`undo`): `{query, undo?}`.
pub const CHECK_IN_HABIT: &str = "check_in_habit";
/// A new daily habit: `{name}`.
pub const ADD_HABIT: &str = "add_habit";

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
“due this week”, “complete buy milk”, “reopen buy milk”, “delete buy milk”, \
“rename buy milk to buy oat milk”, “move buy milk to friday at 3pm”, “make buy milk high priority”, \
“move buy milk to list Work”, “repeat water plants every week”, “remind me about dentist 1 hour before”, “snooze dentist for 2 hours”, “tag dentist with health”, “what's high priority today?”, “tasks tagged work”, “what's in Work today?”, “done tasks in Work”, “lists”, “habits”, “check in meditate” or “add habit meditate”.";

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

/// Commands that change one task: their words and the field the part after " to " sets.
const EDITS: [(&[&str], &str); 3] = [
    (&["rename "], "title"),
    (&["move ", "reschedule ", "postpone "], "due"),
    (&["set priority of ", "priority of "], "priority"),
];

/// Priority words as people say them, and the one each stands for.
const PRIORITIES: [(&str, &str); 6] = [
    ("high", "high"),
    ("medium", "medium"),
    ("med", "medium"),
    ("low", "low"),
    ("none", "none"),
    ("no", "none"),
];

/// The priority `word` names (a trailing " priority" allowed), if it is one.
fn priority(word: &str) -> Option<&'static str> {
    let word = word.to_ascii_lowercase();
    let word = word.strip_suffix(" priority").unwrap_or(&word).trim();
    PRIORITIES
        .iter()
        .find(|(said, _)| *said == word)
        .map(|(_, means)| *means)
}

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
    // ASCII only, so a byte offset in `lower` is one in `text` (the words matched are ASCII).
    let lower = text.to_ascii_lowercase();
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
    if let Some(reply) = edit(text, lower)
        .or_else(|| priority_phrase(text, lower))
        .or_else(|| repeat_phrase(text, lower))
        .or_else(|| remind_phrase(text, lower))
        .or_else(|| snooze_phrase(text, lower))
        .or_else(|| habit_phrase(text, lower))
        .or_else(|| tag_phrase(text, lower))
        .or_else(|| filter_question(lower))
    {
        return Some(reply);
    }
    for verb in &VERBS {
        let Some(prefix) = verb.prefixes.iter().find(|p| lower.starts_with(*p)) else {
            continue;
        };
        let rest = unquote(&text[prefix.len()..]);
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

/// `text` without its surrounding space and quotes. Quotes come off before the
/// last trim, so `delete " "` asks rather than sends a blank name.
fn unquote(text: &str) -> &str {
    text.trim().trim_matches(['“', '”', '"']).trim()
}

/// Where an edit's task name ends and its new value begins.
enum Cut<'a> {
    At(&'a str, &'a str),
    Missing,
    /// More than one " to " could be the cut.
    Ambiguous,
}

/// Cut `rest` ("X to Y") without guessing. A quoted name ends at its closing
/// quote; otherwise only a " to " outside quotes can be the cut (`separators`),
/// a rename needs exactly one, and a move or a priority takes the last: a day
/// or a priority never holds " to ".
fn cut<'a>(rest: &'a str, field: &str) -> Cut<'a> {
    let rest = rest.trim();
    if let Some((open, close)) = [('“', '”'), ('"', '"')]
        .into_iter()
        .find(|(o, _)| rest.starts_with(*o))
    {
        let inner = &rest[open.len_utf8()..];
        let Some(end) = inner.find(close) else {
            return Cut::Missing;
        };
        let after = inner[end + close.len_utf8()..].trim_start();
        return match after.get(..3) {
            Some(word) if word.eq_ignore_ascii_case("to ") => {
                Cut::At(inner[..end].trim(), unquote(&after[3..]))
            }
            _ => Cut::Missing,
        };
    }
    let at = match *separators(rest).as_slice() {
        [at] => at,
        [.., last] if field != "title" => last,
        [] => return Cut::Missing,
        _ => return Cut::Ambiguous,
    };
    Cut::At(unquote(&rest[..at]), unquote(&rest[at + 4..]))
}

/// "rename X to Y" or "move X to WHEN": an `update_task` call, or what to ask
/// when a part is missing or the cut between them is unclear (see `cut`).
fn edit(text: &str, lower: &str) -> Option<Reply> {
    let (prefix, field) = EDITS.iter().find_map(|(prefixes, field)| {
        prefixes
            .iter()
            .find(|p| lower.starts_with(*p))
            .map(|p| (*p, *field))
    })?;
    if field == "due" {
        if let Some(reply) = list_move(text, prefix) {
            return Some(reply);
        }
    }
    let (query, value) = match cut(&text[prefix.len()..], field) {
        Cut::At(query, value) if !query.is_empty() && !value.is_empty() => (query, value),
        Cut::Ambiguous => {
            return Some(Reply::Text(
                "That has more than one “to”, so I can't tell where the name ends. \
                 Put the names in quotes, like “rename \"buy milk\" to \"talk to mum\"”."
                    .into(),
            ))
        }
        _ => {
            return Some(Reply::Text(match field {
                "title" => {
                    "Rename which task to what? Try “rename buy milk to buy oat milk”.".into()
                }
                "priority" => PRIORITY_HELP.into(),
                _ => "Move which task to when? Try “move buy milk to friday”.".into(),
            }))
        }
    };
    if field == "priority" {
        return Some(match priority(value) {
            Some(means) => update_call(query, field, means),
            None => Reply::Text("Priority is high, medium, low or none.".into()),
        });
    }
    Some(update_call(query, field, value))
}

const LIST_HELP: &str =
    "Move which task to which list? Try “move buy milk to list Work” or “move buy milk to the Work list”.";

/// The list a move's destination clause names (after its " to "): "list Y",
/// "the Y list", or "(the) inbox". `Some("")` for a bare "list", so the person
/// is asked; `None` when the clause is not a list (a day, say). An unquoted
/// name holding " to " is not taken: that " to " may be the real cut.
fn list_name(clause: &str) -> Option<&str> {
    let lower = clause.to_ascii_lowercase();
    match lower.as_str() {
        "inbox" | "the inbox" => return Some("Inbox"),
        "list" | "the list" => return Some(""),
        _ => {}
    }
    let name = if lower.starts_with("list ") {
        &clause["list ".len()..]
    } else if lower.starts_with("the ") && lower.ends_with(" list") {
        clause.get("the ".len()..clause.len() - " list".len())?
    } else {
        return None;
    };
    let name = name.trim();
    let quoted = [('“', '”'), ('"', '"')]
        .iter()
        .any(|(open, close)| name.len() > 1 && name.starts_with(*open) && name.ends_with(*close));
    if !quoted && name.to_ascii_lowercase().contains(" to ") {
        return None;
    }
    Some(unquote(name))
}

/// "move X to list Y", "move X to the Y list" or "move X to (the) inbox": a move
/// between lists. Only the destination clause can name the list: a quoted task
/// name ends at its closing quote, and an unquoted one at the last " to "
/// outside quotes whose rest is a whole list name, so "to list" inside a title
/// or a quoted list name is never taken for the cut. Anything else is not a
/// list move, and stays a move to a day.
fn list_move(text: &str, prefix: &str) -> Option<Reply> {
    let rest = text[prefix.len()..].trim();
    if let Some((open, close)) = [('“', '”'), ('"', '"')]
        .into_iter()
        .find(|(o, _)| rest.starts_with(*o))
    {
        let inner = &rest[open.len_utf8()..];
        let end = inner.find(close)?;
        let after = inner[end + close.len_utf8()..].trim_start();
        let clause = after.get(..3).filter(|w| w.eq_ignore_ascii_case("to "))?;
        let name = list_name(after[clause.len()..].trim())?;
        return Some(list_call(inner[..end].trim(), name));
    }
    if let Some(name) = rest
        .get(..3)
        .filter(|w| w.eq_ignore_ascii_case("to "))
        .and_then(|_| list_name(rest[3..].trim()))
    {
        return Some(list_call("", name));
    }
    separators(rest).into_iter().rev().find_map(|at| {
        let name = list_name(rest[at + " to ".len()..].trim())?;
        Some(list_call(unquote(&rest[..at]), name))
    })
}

/// Byte offsets of each " to " in `text` outside quotes: the only places a
/// command may be cut (see `outside_quotes`).
fn separators(text: &str) -> Vec<usize> {
    outside_quotes(text, " to ")
}

/// Byte offsets of each `word` (ASCII, any case) in `text` outside quotes
/// ("…" or “…”), so no quoted name is ever split. An unclosed quote runs to
/// the end, so it offers no cut and the person is asked.
fn outside_quotes(text: &str, word: &str) -> Vec<usize> {
    let mut quote: Option<char> = None;
    let mut out = Vec::new();
    for (at, ch) in text.char_indices() {
        match (quote, ch) {
            (None, '"') => quote = Some('"'),
            (None, '“') => quote = Some('”'),
            (Some(close), c) if c == close => quote = None,
            (None, _)
                if text
                    .get(at..at + word.len())
                    .is_some_and(|w| w.eq_ignore_ascii_case(word)) =>
            {
                out.push(at)
            }
            _ => {}
        }
    }
    out
}

fn list_call(query: &str, list: &str) -> Reply {
    if query.is_empty() || list.is_empty() {
        return Reply::Text(LIST_HELP.into());
    }
    update_call(query, "list", list)
}

const REMIND_HELP: &str =
    "Remind you about which task, and when? Try “remind me about dentist 1 hour before” \
or “stop reminding me about dentist”.";
const STOP_REMINDING: [&str; 3] = [
    "stop reminding me about ",
    "don't remind me about ",
    "don’t remind me about ",
];
/// When-phrases with no count that end a reminder request.
const REMIND_AT: [&str; 4] = [" at the time", " at time", " on time", " on the day"];

/// "remind me about X <when>" and "stop reminding me about X": an `update_task`
/// call with the when as said (the browser reads it against the task's due
/// date), or `none`. The when is a fixed ending ("at the time", "on the day")
/// or "<count> <unit> before", whose count and unit are the last two words, so
/// the name before them is never cut inside. ("remind me to …" still adds a task.)
fn remind_phrase(text: &str, lower: &str) -> Option<Reply> {
    if let Some(stop) = STOP_REMINDING.iter().find(|s| lower.starts_with(*s)) {
        return Some(remind_call(unquote(&text[stop.len()..]), "none"));
    }
    if lower == "remind me about" || STOP_REMINDING.iter().any(|s| lower == s.trim_end()) {
        return Some(Reply::Text(REMIND_HELP.into()));
    }
    let rest = text
        .get("remind me about ".len()..)
        .filter(|_| lower.starts_with("remind me about "))?
        .trim();
    let lower = rest.to_ascii_lowercase();
    let at = match REMIND_AT.iter().find(|e| lower.ends_with(*e)) {
        Some(ending) => Some(rest.len() - ending.len()),
        None => lower
            .strip_suffix(" before")
            .and_then(|body| body.rmatch_indices(' ').nth(1))
            .map(|(at, _)| at),
    };
    Some(match at {
        Some(at) => remind_call(unquote(&rest[..at]), rest[at..].trim()),
        None => Reply::Text(REMIND_HELP.into()),
    })
}

fn remind_call(query: &str, when: &str) -> Reply {
    if query.is_empty() || when.is_empty() {
        return Reply::Text(REMIND_HELP.into());
    }
    update_call(query, "remind", when)
}

/// Words a priority question may hold besides the priority and the view.
const FILTER_FILLER: [&str; 22] = [
    "what's", "what’s", "whats", "what", "is", "are", "show", "me", "my", "list", "all", "the",
    "tasks", "task", "which", "due", "with", "items", "anything", "any", "for", "in",
];
/// Status words, and the status each names ("won't do" is read as two words).
const FILTER_STATUSES: [(&str, &str); 7] = [
    ("open", "open"),
    ("done", "done"),
    ("completed", "done"),
    ("finished", "done"),
    ("closed", "done"),
    ("wontdo", "wontdo"),
    ("won't-do", "wontdo"),
];
const WONT: [&str; 3] = ["won't", "won’t", "wont"];

/// View words, and the view each names.
const FILTER_VIEWS: [(&str, &str); 6] = [
    ("overdue", "overdue"),
    ("week", "next7"),
    ("this", "next7"),
    ("next", "next7"),
    ("7", "next7"),
    ("today", "today"),
];

/// "high priority", "what's tagged work today?", "#errands", "what's in Work?",
/// "done tasks in Work", "overdue high priority tasks tagged work in Home": a
/// `filter_tasks` call.
/// Every other word must be filler or one view's words, so "add buy milk high
/// priority" is never read as a question. A tag is "tagged X", "tag X" or "#X";
/// a list is "in X", "in list X" or "in the X list"; quote either with spaces.
fn filter_question(lower: &str) -> Option<Reply> {
    let cleaned = lower
        .replace(['?', '.', '!', ','], " ")
        .replace("-priority", " priority");
    let words = tokens(&cleaned);
    let (mut level, mut tag, mut list, mut view) = (None, None, None, "all");
    let mut status = None;
    let mut i = 0;
    while let Some(&word) = words.get(i) {
        let next = words.get(i + 1).copied();
        i += 1;
        if next == Some("priority") && level.is_none() {
            if let Some(named) = priority(word) {
                level = Some(named);
                i += 1;
                continue;
            }
        }
        let named_status = match word {
            w if WONT.contains(&w) && next == Some("do") => {
                i += 1;
                Some("wontdo")
            }
            w => FILTER_STATUSES
                .iter()
                .find(|(said, _)| *said == w)
                .map(|(_, s)| *s),
        };
        if let Some(named) = named_status {
            if status.replace(named).is_some_and(|was| was != named) {
                return None;
            }
            continue;
        }
        if word == "in" {
            if let Some((name, after)) = list_after_in(&words, i) {
                if list.replace(name).is_some() {
                    return None;
                }
                i = after;
            }
            continue; // "in" before a view, as in "due in the next 7 days"
        }
        let named_tag = match word {
            "tagged" | "tag" => {
                i += 1;
                Some(next?.trim_start_matches('#'))
            }
            _ => word.strip_prefix('#'),
        };
        if let Some(named) = named_tag {
            if named.is_empty() || tag.replace(named).is_some() {
                return None;
            }
            continue;
        }
        match FILTER_VIEWS.iter().find(|(said, _)| *said == word) {
            Some((_, named)) if view == "all" || view == *named => view = named,
            Some(_) => return None,
            None if FILTER_FILLER.contains(&word) || word == "days" => {}
            None => return None,
        }
    }
    if status.is_none() && level.is_none() && tag.is_none() && list.is_none() {
        return None;
    }
    let mut args = Value::object();
    if let Some(status) = status {
        args.insert("status", status);
    }
    if let Some(level) = level {
        args.insert("priority", level);
    }
    if let Some(tag) = tag {
        args.insert("tag", tag);
    }
    if let Some(list) = list {
        args.insert("list", list);
    }
    args.insert("view", view);
    Some(Reply::Call {
        tool: FILTER_TASKS,
        args,
        say: "Checking…".into(),
    })
}

/// The list named after an "in" whose next word is `words[at]` ("in Work", "in
/// list Work", "in the Work list"), and where the words after it start; `None`
/// when what follows is a view or filler ("in the next 7 days", "in my tasks").
fn list_after_in<'a>(words: &[&'a str], at: usize) -> Option<(&'a str, usize)> {
    let start = words[at..]
        .iter()
        .position(|w| !["the", "my", "list"].contains(w))
        .map(|skip| at + skip)?;
    let name = words[start];
    let view = FILTER_VIEWS.iter().any(|(said, _)| *said == name);
    if view || FILTER_FILLER.contains(&name) || name == "days" {
        return None;
    }
    let after = start + 1;
    Some((name, after + usize::from(words.get(after) == Some(&"list"))))
}

/// `text`'s words, a quoted run ("…" or “…”) being one word without its quotes.
fn tokens(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text.trim_start();
    while let Some(first) = rest.chars().next() {
        let close = match first {
            '"' => Some('"'),
            '“' => Some('”'),
            _ => None,
        };
        let (word, after) = match close {
            Some(close) => {
                let body = &rest[first.len_utf8()..];
                body.find(close).map_or((body, ""), |end| {
                    (&body[..end], &body[end + close.len_utf8()..])
                })
            }
            None => rest.split_at(rest.find(char::is_whitespace).unwrap_or(rest.len())),
        };
        out.push(word);
        rest = after.trim_start();
    }
    out
}

/// The heading and the nothing-found line for a filter's answer.
fn filter_heading(
    status: Option<&str>,
    level: Option<&str>,
    tag: Option<&str>,
    list: Option<&str>,
    view: &str,
) -> (String, String) {
    let which = match level {
        None => "tasks".to_string(),
        Some("none") => "tasks with no priority".to_string(),
        Some(level) => format!("{level}-priority tasks"),
    };
    let tagged = tag.map_or(String::new(), |t| format!(" tagged #{t}"))
        + &list.map_or(String::new(), |l| format!(" in {l}"));
    let (overdue, after) = match view {
        "today" => ("", " due today"),
        "next7" => ("", " due in the next 7 days"),
        "overdue" => ("overdue ", ""),
        _ => ("", ""),
    };
    // "Open" is said only where nothing else narrows the list ("Open tasks tagged #work").
    let state = match (status, view) {
        (Some("done"), _) => "done ",
        (Some("wontdo"), _) => "won't-do ",
        (_, "all") => "open ",
        _ => "",
    };
    let before = format!("{overdue}{state}");
    let heading = format!("{before}{which}{tagged}{after}:");
    let mut first = heading.chars();
    let heading = first
        .next()
        .map_or(String::new(), |c| c.to_uppercase().chain(first).collect());
    (heading, format!("No {before}{which}{tagged}{after}."))
}

const TAG_HELP: &str = "Tag which task, and with what? Try “tag dentist with health” \
or “remove tag health from dentist”.";

/// "tag X with|as TAGS" and "remove tag TAGS from X" (or "remove the TAG tag
/// from X"): an `update_task` call adding (`tag`) or removing (`untag`) the
/// tags as said, which the browser splits and checks. The split is at the last
/// separator outside quotes, so a quoted name may hold one. Checked before the
/// task verbs, so "remove tag …" never deletes a task.
fn tag_phrase(text: &str, lower: &str) -> Option<Reply> {
    if ["tag", "remove tag", "untag"].contains(&lower) {
        return Some(Reply::Text(TAG_HELP.into()));
    }
    if let Some(rest) = text
        .get("tag ".len()..)
        .filter(|_| lower.starts_with("tag "))
    {
        let at = [" with ", " as "]
            .iter()
            .filter_map(|sep| outside_quotes(rest, sep).last().map(|&at| (at, sep.len())))
            .max();
        return Some(match at {
            Some((at, len)) => tag_call(unquote(&rest[..at]), "tag", rest[at + len..].trim()),
            None => Reply::Text(TAG_HELP.into()),
        });
    }
    let prefix = ["remove tag ", "remove tags ", "remove the ", "untag "]
        .into_iter()
        .find(|p| lower.starts_with(p))?;
    let rest = &text[prefix.len()..];
    let from = outside_quotes(rest, " from ").last().copied();
    let said = from.map_or(rest.trim(), |at| rest[..at].trim());
    let said = strip_suffix_ci(said, " from").unwrap_or(said);
    let tags = match prefix {
        // "remove the milk" is still a delete; only "remove the … tag(s)" is ours.
        "remove the " => {
            strip_suffix_ci(said, " tag").or_else(|| strip_suffix_ci(said, " tags"))?
        }
        _ => said,
    };
    Some(match from {
        Some(at) => tag_call(unquote(&rest[at + " from ".len()..]), "untag", tags.trim()),
        None => Reply::Text(TAG_HELP.into()),
    })
}

/// `text` without `suffix`, matched ignoring ASCII case.
fn strip_suffix_ci<'a>(text: &'a str, suffix: &str) -> Option<&'a str> {
    let at = text.len().checked_sub(suffix.len())?;
    text.get(at..)?
        .eq_ignore_ascii_case(suffix)
        .then(|| &text[..at])
}

fn tag_call(query: &str, field: &str, tags: &str) -> Reply {
    if query.is_empty() || tags.is_empty() {
        return Reply::Text(TAG_HELP.into());
    }
    update_call(query, field, tags)
}

const HABIT_LISTS: [&str; 6] = [
    "habits",
    "my habits",
    "habits today",
    "show habits",
    "show my habits",
    "list habits",
];
/// Habit commands that name a habit: their words, the tool, its arguments, and what to ask.
const HABIT_VERBS: [(&[&str], &str, &str, &str); 3] = [
    (
        &[
            "undo check in for ",
            "undo check-in for ",
            "undo check in ",
            "undo check-in ",
        ],
        CHECK_IN_HABIT,
        "undo",
        "Undo which habit's check-in? Try “undo check in meditate”.",
    ),
    (
        &["check in for ", "check in to ", "check in "],
        CHECK_IN_HABIT,
        "",
        "Check in which habit? Try “check in meditate”.",
    ),
    (
        &["add habit ", "new habit "],
        ADD_HABIT,
        "",
        "What habit should I add? Try “add habit meditate”.",
    ),
];

/// "habits", "check in X", "undo check in X" and "add habit X". Checked before
/// the task verbs, so "add habit …" never adds a task.
fn habit_phrase(text: &str, lower: &str) -> Option<Reply> {
    if HABIT_LISTS.contains(&lower.trim_end_matches(['?', '.', '!'])) {
        return Some(Reply::Call {
            tool: LIST_HABITS,
            args: json!({}),
            say: "Reading your habits…".into(),
        });
    }
    let bare = HABIT_VERBS
        .iter()
        .find(|(prefixes, ..)| prefixes.iter().any(|p| lower == p.trim_end()));
    if let Some((.., ask)) = bare {
        return Some(Reply::Text((*ask).into()));
    }
    let (prefix, tool, flag, ask) =
        HABIT_VERBS.iter().find_map(|(prefixes, tool, flag, ask)| {
            let prefix = prefixes.iter().find(|p| lower.starts_with(*p))?;
            Some((prefix, *tool, *flag, *ask))
        })?;
    let rest = unquote(&text[prefix.len()..]);
    if rest.is_empty() {
        return Some(Reply::Text(ask.into()));
    }
    let (args, say) = match (tool, flag) {
        (ADD_HABIT, _) => (
            json!({ "name": rest }),
            format!("Adding the habit “{rest}”…"),
        ),
        (_, "undo") => (
            json!({ "query": rest, "undo": "true" }),
            format!("Looking up “{rest}”…"),
        ),
        _ => (json!({ "query": rest }), format!("Looking up “{rest}”…")),
    };
    Some(Reply::Call { tool, args, say })
}

const SNOOZE_HELP: &str = "Snooze which task, and for how long? Try “snooze dentist for 2 hours” \
or “snooze bins until friday”.";
const SNOOZE_UNITS: [&str; 10] = [
    "minute", "minutes", "min", "mins", "hour", "hours", "day", "days", "week", "weeks",
];

/// "snooze X for <count> <unit>" or "snooze X until <when>": an `update_task`
/// call with the snooze as said, which the browser reads against now and the
/// task's due date. The count and unit are the last two words, and "until"
/// the last one outside quotes, so the name is never cut inside.
fn snooze_phrase(text: &str, lower: &str) -> Option<Reply> {
    if lower == "snooze" {
        return Some(Reply::Text(SNOOZE_HELP.into()));
    }
    let rest = text
        .get("snooze ".len()..)
        .filter(|_| lower.starts_with("snooze "))?
        .trim();
    if let Some(&at) = outside_quotes(rest, " until ").last() {
        return Some(snooze_call(unquote(&rest[..at]), rest[at..].trim()));
    }
    let duration = rest.rsplit_once(' ').and_then(|(head, unit)| {
        let (name, _count) = head.trim_end().rsplit_once(' ')?;
        SNOOZE_UNITS
            .contains(&unit.to_ascii_lowercase().as_str())
            .then_some(name)
    });
    let Some(name) = duration else {
        return Some(Reply::Text(SNOOZE_HELP.into()));
    };
    let snooze = rest[name.len()..].trim();
    let name = match name.len().checked_sub(" for".len()) {
        Some(at) if name[at..].eq_ignore_ascii_case(" for") => &name[..at],
        _ if name.eq_ignore_ascii_case("for") => "",
        _ => name,
    };
    Some(snooze_call(unquote(name), snooze))
}

fn snooze_call(query: &str, snooze: &str) -> Reply {
    if query.is_empty() || snooze.is_empty() {
        return Reply::Text(SNOOZE_HELP.into());
    }
    update_call(query, "snooze", snooze)
}

const REPEAT_HELP: &str =
    "Repeat which task, and how often? Try “repeat water plants every week” or “stop repeating standup”.";
const STOP_REPEATING: [&str; 4] = [
    "stop repeating ",
    "don't repeat ",
    "don’t repeat ",
    "do not repeat ",
];
/// How-often words that end a "repeat X …" without an "every".
const REPEAT_ENDINGS: [&str; 7] = [
    " daily",
    " weekly",
    " monthly",
    " yearly",
    " annually",
    " on weekdays",
    " weekdays",
];

/// "repeat X every …" / "repeat X daily", "make|set X (to) repeat …", and "stop
/// repeating X": an `update_task` call with the how-often as said (the browser
/// reads it), or `never`. The name ends at the last " every " or " repeat "
/// outside quotes, or at a fixed how-often word, so it is never cut inside.
fn repeat_phrase(text: &str, lower: &str) -> Option<Reply> {
    if let Some(stop) = STOP_REPEATING.iter().find(|s| lower.starts_with(*s)) {
        return Some(repeat_call(unquote(&text[stop.len()..]), "never"));
    }
    if lower == "repeat" || STOP_REPEATING.iter().any(|s| lower == s.trim_end()) {
        return Some(Reply::Text(REPEAT_HELP.into()));
    }
    if lower.starts_with("repeat ") {
        let rest = text["repeat ".len()..].trim();
        let ending = || {
            let lower = rest.to_ascii_lowercase();
            REPEAT_ENDINGS
                .iter()
                .find(|e| lower.ends_with(*e))
                .map(|e| rest.len() - e.len())
        };
        return Some(
            match outside_quotes(rest, " every ")
                .last()
                .copied()
                .or_else(ending)
            {
                Some(at) => repeat_call(unquote(&rest[..at]), rest[at..].trim()),
                None => Reply::Text(REPEAT_HELP.into()),
            },
        );
    }
    let prefix = ["make ", "set "]
        .into_iter()
        .find(|p| lower.starts_with(p))?;
    let rest = text[prefix.len()..].trim();
    let at = *outside_quotes(rest, " repeat ").last()?;
    let name = rest[..at].trim_end();
    let name = match name.get(name.len().saturating_sub(3)..) {
        Some(tail) if tail.eq_ignore_ascii_case(" to") => &name[..name.len() - 3],
        _ => name,
    };
    Some(repeat_call(
        unquote(name),
        rest[at + " repeat ".len()..].trim(),
    ))
}

fn repeat_call(query: &str, rule: &str) -> Reply {
    if query.is_empty() || rule.is_empty() {
        return Reply::Text(REPEAT_HELP.into());
    }
    update_call(query, "repeat", rule)
}

const PRIORITY_HELP: &str = "Which task, and what priority? Try “make buy milk high priority”.";

/// "make|mark|set X <word> priority": the fixed ending says where the name stops.
fn priority_phrase(text: &str, lower: &str) -> Option<Reply> {
    let prefix = ["make ", "mark ", "set "]
        .into_iter()
        .find(|p| lower.starts_with(p))?;
    let (word, means) = PRIORITIES
        .iter()
        .find(|(said, _)| lower.ends_with(&format!(" {said} priority")))?;
    let end = text.len() - (word.len() + " priority".len() + 1);
    let query = text.get(prefix.len()..end).map_or("", unquote);
    if query.is_empty() {
        return Some(Reply::Text(PRIORITY_HELP.into()));
    }
    Some(update_call(query, "priority", means))
}

/// An `update_task` call setting `field` of the task `query` names.
fn update_call(query: &str, field: &str, value: &str) -> Reply {
    let mut args = json!({ "query": query });
    args.insert(field, value);
    Reply::Call {
        tool: UPDATE_TASK,
        args,
        say: format!("Looking up “{query}”…"),
    }
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
            "Several {} match “{}”: {}. Say a little more of the one you mean.",
            if name.contains("habit") {
                "habits"
            } else {
                "tasks"
            },
            arg("query"),
            matches.join(", ")
        );
    }
    let title = parsed.get("title").and_then(Value::as_str).unwrap_or("it");
    match name {
        ADD_TASK => format!("Added “{title}”."),
        ADD_HABIT => format!("Added the daily habit “{title}”."),
        CHECK_IN_HABIT => {
            let progress = parsed.get("progress").and_then(Value::as_str).unwrap_or("");
            match (
                parsed.get("undone").is_some(),
                parsed.get("already").is_some(),
            ) {
                (true, _) => format!("Cleared today's check-ins for “{title}”."),
                (_, true) => format!("“{title}” is already done today ({progress})."),
                _ => format!("Checked in “{title}”: {progress} today."),
            }
        }
        FILTER_TASKS => {
            let (status, level, tag) = (arg("status"), arg("priority"), arg("tag"));
            // The browser answers with the list's own name, as the person named it.
            let list = parsed
                .get("list")
                .and_then(Value::as_str)
                .map_or_else(|| arg("list"), str::to_string);
            let some = |v: &str| Some(v.to_string()).filter(|v| !v.is_empty());
            let (heading, none) = filter_heading(
                some(&status).as_deref(),
                some(&level).as_deref(),
                some(&tag).as_deref(),
                some(&list).as_deref(),
                &arg("view"),
            );
            bullets(&heading, &none, strings(&parsed, "tasks"))
        }
        LIST_HABITS => bullets(
            "Today's habits:",
            "No habits are due today.",
            strings(&parsed, "habits"),
        ),
        COMPLETE_TASK => format!("Completed “{title}”."),
        REOPEN_TASK => format!("Reopened “{title}”."),
        DELETE_TASK => format!("Moved “{title}” to the Trash."),
        UPDATE_TASK if parsed.get("remind").is_some() => {
            match parsed.get("remind").and_then(Value::as_str) {
                Some("none") => format!("No more reminders for “{title}”."),
                Some(when) => format!("I'll remind you about “{title}” {when}."),
                None => "Done.".into(),
            }
        }
        UPDATE_TASK if parsed.get("tagged").is_some() => {
            let tags = parsed.get("tagged").and_then(Value::as_str).unwrap_or("");
            format!("Tagged “{title}” {tags}.")
        }
        UPDATE_TASK if parsed.get("untagged").is_some() => {
            let tags = parsed.get("untagged").and_then(Value::as_str).unwrap_or("");
            format!("Removed {tags} from “{title}”.")
        }
        UPDATE_TASK if parsed.get("snooze").is_some() => {
            match parsed.get("snooze").and_then(Value::as_str) {
                Some(until) => format!("Snoozed “{title}” until {until}."),
                None => "Done.".into(),
            }
        }
        UPDATE_TASK if parsed.get("repeat").is_some() => {
            match (
                parsed.get("repeat").and_then(Value::as_str),
                parsed.get("due").and_then(Value::as_str),
            ) {
                (Some("never"), _) => format!("“{title}” no longer repeats."),
                (Some(rule), Some(due)) => format!("“{title}” now repeats {rule}, from {due}."),
                (Some(rule), None) => format!("“{title}” now repeats {rule}."),
                (None, _) => "Done.".into(),
            }
        }
        UPDATE_TASK => match (
            parsed
                .get("due")
                .or_else(|| parsed.get("list"))
                .and_then(Value::as_str),
            parsed.get("priority").and_then(Value::as_str),
        ) {
            (Some(to), _) => format!("Moved “{title}” to {to}."),
            (_, Some("none")) => format!("Cleared the priority of “{title}”."),
            (_, Some(level)) => format!("Gave “{title}” {level} priority."),
            _ => format!("Renamed it “{title}”."),
        },
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

    const ALL: [&str; 12] = [
        FILTER_TASKS,
        LIST_HABITS,
        CHECK_IN_HABIT,
        ADD_HABIT,
        ADD_TASK,
        UPDATE_TASK,
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
        let query = args.get("query").and_then(Value::as_str);
        Some(match (tool, rest) {
            (UPDATE_TASK, _) => format!("Looking up “{}”…", query?),
            (ADD_TASK, Some(r)) => format!("Adding “{r}”…"),
            (SEARCH_TASKS, Some(r)) => format!("Looking for “{r}”…"),
            (LIST_LISTS, _) => "Reading your lists…".into(),
            (LIST_HABITS, _) => "Reading your habits…".into(),
            (ADD_HABIT, _) => format!(
                "Adding the habit “{}”…",
                args.get("name").and_then(Value::as_str)?
            ),
            (DUE_TASKS | FILTER_TASKS, _) => "Checking…".into(),
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
    fn splits_a_rename_only_where_it_is_sure() {
        // Two " to ", unquoted: either could be the cut, so it asks.
        assert!(matches!(
            decide(&input("rename Buy milk to Talk to mum")),
            Reply::Text(t) if t.contains("quotes")
        ));
        // Quotes mark where each name ends, " to " inside them included.
        assert_eq!(
            decide(&input("rename \"Buy milk\" to \"Talk to mum\"")),
            call(
                UPDATE_TASK,
                json!({"query": "Buy milk", "title": "Talk to mum"})
            )
        );
        assert_eq!(
            decide(&input("rename “talk to mum” to call mum")),
            call(
                UPDATE_TASK,
                json!({"query": "talk to mum", "title": "call mum"})
            )
        );
        assert_eq!(
            decide(&input("rename Buy milk to “Talk to mum”")),
            call(
                UPDATE_TASK,
                json!({"query": "Buy milk", "title": "Talk to mum"})
            )
        );
        // A day never holds " to ", so a move still cuts at the last one.
        assert_eq!(
            decide(&input("move talk to mum to friday")),
            call(
                UPDATE_TASK,
                json!({"query": "talk to mum", "due": "friday"})
            )
        );
        for unclosed in ["rename “buy milk to oats", "rename \"buy\" milk to oats"] {
            assert!(
                matches!(decide(&input(unclosed)), Reply::Text(t) if t.starts_with("Rename which task")),
                "{unclosed}"
            );
        }
    }

    #[test]
    fn turns_list_moves_into_update_task() {
        assert_eq!(
            decide(&input("move buy milk to list Work")),
            call(UPDATE_TASK, json!({"query": "buy milk", "list": "Work"}))
        );
        assert_eq!(
            decide(&input("Move add to list groceries to the Shopping list")),
            call(
                UPDATE_TASK,
                json!({"query": "add to list groceries", "list": "Shopping"})
            )
        );
        assert_eq!(
            decide(&input("move talk to mum to list “Things to do”")),
            call(
                UPDATE_TASK,
                json!({"query": "talk to mum", "list": "Things to do"})
            )
        );
        assert_eq!(
            decide(&input("move buy milk to the inbox")),
            call(UPDATE_TASK, json!({"query": "buy milk", "list": "Inbox"}))
        );
        // "to list" inside the task's own title is not a destination.
        assert_eq!(
            decide(&input("move \"Add to list groceries\" to friday")),
            call(
                UPDATE_TASK,
                json!({"query": "Add to list groceries", "due": "friday"})
            )
        );
        assert_eq!(
            decide(&input("move add to list groceries to friday")),
            call(
                UPDATE_TASK,
                json!({"query": "add to list groceries", "due": "friday"})
            )
        );
        assert_eq!(
            decide(&input("move “Add to list groceries” to list Shopping")),
            call(
                UPDATE_TASK,
                json!({"query": "Add to list groceries", "list": "Shopping"})
            )
        );
        assert_eq!(
            decide(&input("move add to list groceries to list Shopping")),
            call(
                UPDATE_TASK,
                json!({"query": "add to list groceries", "list": "Shopping"})
            )
        );
        // A " to " inside a quoted list name is never the cut.
        assert_eq!(
            decide(&input("move Buy milk to list \"Things to list Work\"")),
            call(
                UPDATE_TASK,
                json!({"query": "Buy milk", "list": "Things to list Work"})
            )
        );
        assert_eq!(
            decide(&input("move Buy milk to the “Things to do” list")),
            call(
                UPDATE_TASK,
                json!({"query": "Buy milk", "list": "Things to do"})
            )
        );
        // A day and time, or a time alone, is still a move in time: the browser reads it.
        assert_eq!(
            decide(&input("move pay rent to friday at 3pm")),
            call(
                UPDATE_TASK,
                json!({"query": "pay rent", "due": "friday at 3pm"})
            )
        );
        assert_eq!(
            decide(&input("move talk to mum to 9:30")),
            call(UPDATE_TASK, json!({"query": "talk to mum", "due": "9:30"}))
        );
        // Still a day: it does not end in a list.
        assert_eq!(
            decide(&input("move buy milk to friday")),
            call(UPDATE_TASK, json!({"query": "buy milk", "due": "friday"}))
        );
        for missing in [
            "move to list Work",
            "move buy milk to list ",
            "move buy milk to the list",
        ] {
            assert!(
                matches!(decide(&input(missing)), Reply::Text(t) if t.starts_with("Move which task to which list")),
                "{missing}"
            );
        }
        assert_eq!(
            answered(
                "move milk to list Work",
                r#"{"title":"Buy milk","list":"Work"}"#
            ),
            Reply::Text("Moved “Buy milk” to Work.".into())
        );
    }

    #[test]
    fn turns_status_questions_into_filter_tasks() {
        let by = |st: &str, v: &str| call(FILTER_TASKS, json!({"status": st, "view": v}));
        assert_eq!(decide(&input("done tasks")), by("done", "all"));
        assert_eq!(
            decide(&input("show me completed tasks due this week")),
            by("done", "next7")
        );
        assert_eq!(decide(&input("won't do tasks")), by("wontdo", "all"));
        assert_eq!(decide(&input("open tasks due today")), by("open", "today"));
        assert_eq!(
            decide(&input("finished high priority tasks in work")),
            call(
                FILTER_TASKS,
                json!({"status": "done", "priority": "high", "list": "work", "view": "all"})
            )
        );
        assert_eq!(
            decide(&input("done milk")),
            call(COMPLETE_TASK, json!({"query": "milk"}))
        );
        assert_eq!(
            decide(&input("complete tasks")),
            call(COMPLETE_TASK, json!({"query": "tasks"}))
        );
        for not_ours in ["done open tasks", "won't", "what won't do tasks are there"] {
            assert!(
                !matches!(
                    decide(&input(not_ours)),
                    Reply::Call {
                        tool: FILTER_TASKS,
                        ..
                    }
                ),
                "{not_ours}"
            );
        }
        assert_eq!(
            answered(
                "done tasks in work",
                r#"{"tasks":["Report"],"list":"Work"}"#
            ),
            Reply::Text("Done tasks in Work:\n- Report".into())
        );
        assert_eq!(
            answered("overdue won't do tasks", r#"{"tasks":[]}"#),
            Reply::Text("No overdue won't-do tasks.".into())
        );
        assert_eq!(
            answered("open tasks tagged work", r#"{"tasks":["Report"]}"#),
            Reply::Text("Open tasks tagged #work:\n- Report".into())
        );
    }

    #[test]
    fn turns_list_questions_into_filter_tasks() {
        let listed = |l: &str, v: &str| call(FILTER_TASKS, json!({"list": l, "view": v}));
        assert_eq!(decide(&input("what's in work?")), listed("work", "all"));
        assert_eq!(
            decide(&input("tasks in list Work due today")),
            listed("work", "today")
        );
        assert_eq!(
            decide(&input("overdue tasks in the inbox list")),
            listed("inbox", "overdue")
        );
        assert_eq!(
            decide(&input("what's in “home stuff” this week")),
            listed("home stuff", "next7")
        );
        assert_eq!(
            decide(&input("high priority tasks tagged urgent in work today")),
            call(
                FILTER_TASKS,
                json!({"priority": "high", "tag": "urgent", "list": "work", "view": "today"})
            )
        );
        assert_eq!(
            decide(&input("high priority due in the next 7 days")),
            call(FILTER_TASKS, json!({"priority": "high", "view": "next7"}))
        );
        assert_eq!(
            decide(&input("move milk to list Work")),
            call(UPDATE_TASK, json!({"query": "milk", "list": "Work"}))
        );
        for not_ours in [
            "in",
            "what's in my tasks",
            "in work in home",
            "in work today overdue",
            "add milk in work",
        ] {
            assert!(
                !matches!(
                    decide(&input(not_ours)),
                    Reply::Call {
                        tool: FILTER_TASKS,
                        ..
                    }
                ),
                "{not_ours}"
            );
        }
        assert_eq!(
            answered(
                "what's in work today",
                r#"{"tasks":["Report"],"list":"Work"}"#
            ),
            Reply::Text("Tasks in Work due today:\n- Report".into())
        );
        assert_eq!(
            answered("high priority in work", r#"{"tasks":[],"list":"Work"}"#),
            Reply::Text("No open high-priority tasks in Work.".into())
        );
    }

    #[test]
    fn turns_tag_questions_into_filter_tasks() {
        let tagged = |t: &str, v: &str| call(FILTER_TASKS, json!({"tag": t, "view": v}));
        assert_eq!(decide(&input("tasks tagged work")), tagged("work", "all"));
        assert_eq!(decide(&input("#errands")), tagged("errands", "all"));
        assert_eq!(
            decide(&input("What's tagged #work today?")),
            tagged("work", "today")
        );
        assert_eq!(
            decide(&input("overdue tasks with tag “deep work”")),
            tagged("deep work", "overdue")
        );
        assert_eq!(
            decide(&input("high priority tasks tagged work due this week")),
            call(
                FILTER_TASKS,
                json!({"priority": "high", "tag": "work", "view": "next7"})
            )
        );
        assert_eq!(
            decide(&input("tag milk with work")),
            call(UPDATE_TASK, json!({"query": "milk", "tag": "work"}))
        );
        assert_eq!(
            decide(&input("find #work")),
            call(SEARCH_TASKS, json!({"query": "#work"}))
        );
        for not_ours in [
            "tagged",
            "#",
            "#work #home",
            "tagged work and home",
            "add #work report",
        ] {
            assert!(
                !matches!(
                    decide(&input(not_ours)),
                    Reply::Call {
                        tool: FILTER_TASKS,
                        ..
                    }
                ),
                "{not_ours}"
            );
        }
        assert_eq!(
            answered("tasks tagged work", r#"{"tasks":["Report"]}"#),
            Reply::Text("Open tasks tagged #work:\n- Report".into())
        );
        assert_eq!(
            answered("high priority tagged work today", r#"{"tasks":[]}"#),
            Reply::Text("No high-priority tasks tagged #work due today.".into())
        );
    }

    #[test]
    fn turns_priority_questions_into_filter_tasks() {
        let filter = |p: &str, v: &str| call(FILTER_TASKS, json!({"priority": p, "view": v}));
        assert_eq!(decide(&input("high priority")), filter("high", "all"));
        assert_eq!(
            decide(&input("What's high priority today?")),
            filter("high", "today")
        );
        assert_eq!(
            decide(&input("show me my high-priority tasks")),
            filter("high", "all")
        );
        assert_eq!(
            decide(&input("overdue low priority tasks")),
            filter("low", "overdue")
        );
        assert_eq!(
            decide(&input("med priority due this week")),
            filter("medium", "next7")
        );
        assert_eq!(
            decide(&input("no priority tasks due in the next 7 days")),
            filter("none", "next7")
        );
        assert_eq!(
            decide(&input("make milk high priority")),
            call(UPDATE_TASK, json!({"query": "milk", "priority": "high"}))
        );
        assert_eq!(
            decide(&input("add buy milk high priority")),
            call(ADD_TASK, json!({"title": "buy milk high priority"}))
        );
        assert_eq!(
            decide(&input("what's due today?")),
            call(DUE_TASKS, json!({"view": "today"}))
        );
        for not_ours in ["high priority overdue today", "priority", "top priority"] {
            assert!(
                !matches!(
                    decide(&input(not_ours)),
                    Reply::Call {
                        tool: FILTER_TASKS,
                        ..
                    }
                ),
                "{not_ours}"
            );
        }
        assert_eq!(
            answered(
                "high priority today",
                r#"{"tasks":["Pay rent","Call bank"]}"#
            ),
            Reply::Text("High-priority tasks due today:\n- Pay rent\n- Call bank".into())
        );
        assert_eq!(
            answered("overdue low priority", r#"{"tasks":[]}"#),
            Reply::Text("No overdue low-priority tasks.".into())
        );
        assert_eq!(
            answered("no priority tasks", r#"{"tasks":["Read"]}"#),
            Reply::Text("Open tasks with no priority:\n- Read".into())
        );
    }

    #[test]
    fn turns_tag_phrases_into_update_task() {
        let tag = |q: &str, t: &str| call(UPDATE_TASK, json!({"query": q, "tag": t}));
        let untag = |q: &str, t: &str| call(UPDATE_TASK, json!({"query": q, "untag": t}));
        assert_eq!(
            decide(&input("tag dentist with health")),
            tag("dentist", "health")
        );
        assert_eq!(
            decide(&input("Tag pay rent as home, money")),
            tag("pay rent", "home, money")
        );
        assert_eq!(
            decide(&input("tag “cook with kids” with family")),
            tag("cook with kids", "family")
        );
        assert_eq!(
            decide(&input("tag dentist with #health and errands")),
            tag("dentist", "#health and errands")
        );
        assert_eq!(
            decide(&input("remove tag health from dentist")),
            untag("dentist", "health")
        );
        assert_eq!(
            decide(&input("remove the work tag from “email from bob”")),
            untag("email from bob", "work")
        );
        assert_eq!(
            decide(&input("untag work from email")),
            untag("email", "work")
        );
        assert_eq!(
            decide(&input("remove milk")),
            call(DELETE_TASK, json!({"query": "milk"}))
        );
        assert_eq!(
            decide(&input("remove the milk")),
            call(DELETE_TASK, json!({"query": "the milk"}))
        );
        for missing in [
            "tag",
            "tag dentist",
            "tag with health",
            "tag dentist with",
            "remove tag health from",
            "remove tag from dentist",
        ] {
            assert!(
                matches!(decide(&input(missing)), Reply::Text(t) if t.starts_with("Tag which task")),
                "{missing}"
            );
        }
        assert_eq!(
            answered(
                "tag dentist with health",
                r##"{"title":"Dentist","tagged":"#health"}"##
            ),
            Reply::Text("Tagged “Dentist” #health.".into())
        );
        assert_eq!(
            answered(
                "remove tag health from dentist",
                r##"{"title":"Dentist","untagged":"#health"}"##
            ),
            Reply::Text("Removed #health from “Dentist”.".into())
        );
    }

    #[test]
    fn turns_habit_phrases_into_habit_tools() {
        assert_eq!(decide(&input("habits")), call(LIST_HABITS, json!({})));
        assert_eq!(
            decide(&input("Show my habits?")),
            call(LIST_HABITS, json!({}))
        );
        assert_eq!(
            decide(&input("check in meditate")),
            call(CHECK_IN_HABIT, json!({"query": "meditate"}))
        );
        assert_eq!(
            decide(&input("Check in to “read 20 pages”")),
            call(CHECK_IN_HABIT, json!({"query": "read 20 pages"}))
        );
        assert_eq!(
            decide(&input("undo check-in for meditate")),
            call(CHECK_IN_HABIT, json!({"query": "meditate", "undo": "true"}))
        );
        assert_eq!(
            decide(&input("add habit drink water")),
            call(ADD_HABIT, json!({"name": "drink water"}))
        );
        assert_eq!(
            decide(&input("add drink water")),
            call(ADD_TASK, json!({"title": "drink water"}))
        );
        assert_eq!(
            decide(&input("check off milk")),
            call(COMPLETE_TASK, json!({"query": "milk"}))
        );
        for (missing, ask) in [
            ("check in", "Check in which"),
            ("check in “ ”", "Check in which"),
            ("undo check in", "Undo which"),
            ("add habit", "What habit"),
        ] {
            assert!(
                matches!(decide(&input(missing)), Reply::Text(t) if t.starts_with(ask)),
                "{missing}"
            );
        }
        assert_eq!(
            answered(
                "check in meditate",
                r#"{"title":"Meditate","progress":"1/1"}"#
            ),
            Reply::Text("Checked in “Meditate”: 1/1 today.".into())
        );
        assert_eq!(
            answered(
                "check in meditate",
                r#"{"title":"Meditate","progress":"1/1","already":"yes"}"#
            ),
            Reply::Text("“Meditate” is already done today (1/1).".into())
        );
        assert_eq!(
            answered(
                "undo check in meditate",
                r#"{"title":"Meditate","undone":"yes"}"#
            ),
            Reply::Text("Cleared today's check-ins for “Meditate”.".into())
        );
        assert_eq!(
            answered("check in read", r#"{"matches":["Read","Read news"]}"#),
            Reply::Text("Several habits match “read”: Read, Read news. Say a little more of the one you mean.".into())
        );
        assert_eq!(
            answered("habits", r#"{"habits":["Meditate (0/1)","Water (3/8)"]}"#),
            Reply::Text("Today's habits:\n- Meditate (0/1)\n- Water (3/8)".into())
        );
        assert_eq!(
            answered("add habit meditate", r#"{"title":"Meditate"}"#),
            Reply::Text("Added the daily habit “Meditate”.".into())
        );
    }

    #[test]
    fn turns_snooze_phrases_into_update_task() {
        let snooze = |q: &str, d: &str| call(UPDATE_TASK, json!({"query": q, "snooze": d}));
        assert_eq!(
            decide(&input("snooze dentist for 2 hours")),
            snooze("dentist", "2 hours")
        );
        assert_eq!(
            decide(&input("Snooze pay rent FOR a day")),
            snooze("pay rent", "a day")
        );
        assert_eq!(
            decide(&input("snooze bins 30 minutes")),
            snooze("bins", "30 minutes")
        );
        assert_eq!(
            decide(&input("snooze “wait for 2 days” for 1 week")),
            snooze("wait for 2 days", "1 week")
        );
        assert_eq!(
            decide(&input("snooze bins until friday at 9")),
            snooze("bins", "until friday at 9")
        );
        assert_eq!(
            decide(&input("snooze “until done” until tomorrow")),
            snooze("until done", "until tomorrow")
        );
        for missing in [
            "snooze",
            "snooze dentist",
            "snooze for 2 hours",
            "snooze dentist for ages",
            "snooze until friday",
            "snooze dentist until",
        ] {
            assert!(
                matches!(decide(&input(missing)), Reply::Text(t) if t.starts_with("Snooze which task")),
                "{missing}"
            );
        }
        assert_eq!(
            answered(
                "snooze dentist for 2 hours",
                r#"{"title":"Dentist","snooze":"Thu 22 Oct 21:30"}"#
            ),
            Reply::Text("Snoozed “Dentist” until Thu 22 Oct 21:30.".into())
        );
    }

    #[test]
    fn turns_reminder_phrases_into_update_task() {
        let remind = |q: &str, r: &str| call(UPDATE_TASK, json!({"query": q, "remind": r}));
        assert_eq!(
            decide(&input("remind me about dentist 1 hour before")),
            remind("dentist", "1 hour before")
        );
        assert_eq!(
            decide(&input("Remind me about pay rent 2 days before")),
            remind("pay rent", "2 days before")
        );
        assert_eq!(
            decide(&input("remind me about “call in 5 days” a day before")),
            remind("call in 5 days", "a day before")
        );
        assert_eq!(
            decide(&input("remind me about standup at the time")),
            remind("standup", "at the time")
        );
        assert_eq!(
            decide(&input("remind me about bins on the day")),
            remind("bins", "on the day")
        );
        assert_eq!(
            decide(&input("stop reminding me about standup")),
            remind("standup", "none")
        );
        // "remind me to …" still adds a task.
        assert_eq!(
            decide(&input("remind me to call mum")),
            call(ADD_TASK, json!({"title": "call mum"}))
        );
        for missing in [
            "remind me about dentist",
            "remind me about 1 hour before",
            "remind me about",
            "stop reminding me about",
        ] {
            assert!(
                matches!(decide(&input(missing)), Reply::Text(t) if t.starts_with("Remind you about which task")),
                "{missing}"
            );
        }
        assert_eq!(
            answered(
                "remind me about dentist 1 hour before",
                r#"{"title":"Dentist","remind":"1 hour before"}"#
            ),
            Reply::Text("I'll remind you about “Dentist” 1 hour before.".into())
        );
        assert_eq!(
            answered(
                "stop reminding me about dentist",
                r#"{"title":"Dentist","remind":"none"}"#
            ),
            Reply::Text("No more reminders for “Dentist”.".into())
        );
    }

    #[test]
    fn turns_repeat_phrases_into_update_task() {
        let repeat = |q: &str, r: &str| call(UPDATE_TASK, json!({"query": q, "repeat": r}));
        assert_eq!(
            decide(&input("repeat water plants every 2 weeks")),
            repeat("water plants", "every 2 weeks")
        );
        assert_eq!(
            decide(&input("Repeat standup every monday and friday")),
            repeat("standup", "every monday and friday")
        );
        assert_eq!(
            decide(&input("repeat “every other thing” weekly")),
            repeat("every other thing", "weekly")
        );
        assert_eq!(
            decide(&input("repeat pay rent monthly")),
            repeat("pay rent", "monthly")
        );
        assert_eq!(
            decide(&input("repeat gym on weekdays")),
            repeat("gym", "on weekdays")
        );
        assert_eq!(
            decide(&input("make take pills repeat daily")),
            repeat("take pills", "daily")
        );
        assert_eq!(
            decide(&input("set backup to repeat every week")),
            repeat("backup", "every week")
        );
        assert_eq!(
            decide(&input("stop repeating standup")),
            repeat("standup", "never")
        );
        assert_eq!(
            decide(&input("don't repeat “pay rent”")),
            repeat("pay rent", "never")
        );
        for missing in [
            "repeat",
            "repeat every week",
            "repeat gym",
            "stop repeating ",
        ] {
            assert!(
                matches!(decide(&input(missing)), Reply::Text(t) if t.starts_with("Repeat which task")),
                "{missing}"
            );
        }
        assert_eq!(
            answered(
                "repeat gym weekly",
                r#"{"title":"Gym","repeat":"every week"}"#
            ),
            Reply::Text("“Gym” now repeats every week.".into())
        );
        assert_eq!(
            answered(
                "repeat gym weekly",
                r#"{"title":"Gym","repeat":"every week","due":"Thu, 22 Oct"}"#
            ),
            Reply::Text("“Gym” now repeats every week, from Thu, 22 Oct.".into())
        );
        assert_eq!(
            answered("stop repeating gym", r#"{"title":"Gym","repeat":"never"}"#),
            Reply::Text("“Gym” no longer repeats.".into())
        );
    }

    #[test]
    fn turns_priority_phrases_into_update_task() {
        assert_eq!(
            decide(&input("make buy milk high priority")),
            call(
                UPDATE_TASK,
                json!({"query": "buy milk", "priority": "high"})
            )
        );
        assert_eq!(
            decide(&input("Mark “talk to mum” low priority")),
            call(
                UPDATE_TASK,
                json!({"query": "talk to mum", "priority": "low"})
            )
        );
        assert_eq!(
            decide(&input("set pay rent no priority")),
            call(
                UPDATE_TASK,
                json!({"query": "pay rent", "priority": "none"})
            )
        );
        assert_eq!(
            decide(&input("set priority of talk to mum to Medium")),
            call(
                UPDATE_TASK,
                json!({"query": "talk to mum", "priority": "medium"})
            )
        );
        assert!(matches!(
            decide(&input("set priority of buy milk to urgent")),
            Reply::Text(t) if t.starts_with("Priority is")
        ));
        assert!(matches!(
            decide(&input("make high priority")),
            Reply::Text(t) if t.starts_with("Which task")
        ));
        assert_eq!(
            answered(
                "make milk high priority",
                r#"{"title":"Buy milk","priority":"high"}"#
            ),
            Reply::Text("Gave “Buy milk” high priority.".into())
        );
        assert_eq!(
            answered(
                "make milk no priority",
                r#"{"title":"Buy milk","priority":"none"}"#
            ),
            Reply::Text("Cleared the priority of “Buy milk”.".into())
        );
    }

    #[test]
    fn turns_rename_and_move_into_update_task() {
        assert_eq!(
            decide(&input("rename buy milk to Buy oat milk")),
            call(
                UPDATE_TASK,
                json!({"query": "buy milk", "title": "Buy oat milk"})
            )
        );
        assert_eq!(
            decide(&input("Move “talk to mum” to Friday")),
            call(
                UPDATE_TASK,
                json!({"query": "talk to mum", "due": "Friday"})
            )
        );
        assert_eq!(
            decide(&input("reschedule pay rent to in 3 days")),
            call(
                UPDATE_TASK,
                json!({"query": "pay rent", "due": "in 3 days"})
            )
        );
        for missing in ["rename buy milk", "rename  to x", "rename x to \" \""] {
            assert!(
                matches!(decide(&input(missing)), Reply::Text(t) if t.starts_with("Rename which task")),
                "{missing}"
            );
        }
        assert!(
            matches!(decide(&input("move buy milk")), Reply::Text(t) if t.starts_with("Move which task"))
        );
        // A character whose lower case is longer must not shift where the name is cut.
        assert_eq!(
            decide(&input("rename \u{212A}ettle to Kettle")),
            call(
                UPDATE_TASK,
                json!({"query": "\u{212A}ettle", "title": "Kettle"})
            )
        );
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
            answered("rename milk to Oat milk", r#"{"title":"Oat milk"}"#),
            Reply::Text("Renamed it “Oat milk”.".into())
        );
        assert_eq!(
            answered(
                "move milk to friday",
                r#"{"title":"Buy milk","due":"Fri, 23 Oct"}"#
            ),
            Reply::Text("Moved “Buy milk” to Fri, 23 Oct.".into())
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
