//! `bbp tokens`: model tokens per turn, read from the harness output the
//! moderator already keeps (`agents/<task>/turn-N.log`). The core cannot see
//! tokens; a launcher that runs `claude -p ... --output-format stream-json`
//! prints them. A turn whose harness was killed has no final `result` line, so
//! its tokens are the sum of the assistant messages it printed (a lower bound,
//! marked `complete: false`).

use crate::moderator::log_path;
use rusty_bbp::*;
use rusty_serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

/// Tokens as the harness reports them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    #[rusty_serde(default)]
    pub input_tokens: u64,
    #[rusty_serde(default)]
    pub output_tokens: u64,
    #[rusty_serde(default)]
    pub cache_creation_input_tokens: u64,
    #[rusty_serde(default)]
    pub cache_read_input_tokens: u64,
}

impl Usage {
    /// Everything the harness consumed. The parts stay in the output so the
    /// owner can weigh cache reads differently.
    pub fn total(&self) -> u64 {
        self.input_tokens
            + self.output_tokens
            + self.cache_creation_input_tokens
            + self.cache_read_input_tokens
    }

    fn plus(self, o: Usage) -> Usage {
        Usage {
            input_tokens: self.input_tokens + o.input_tokens,
            output_tokens: self.output_tokens + o.output_tokens,
            cache_creation_input_tokens: self.cache_creation_input_tokens
                + o.cache_creation_input_tokens,
            cache_read_input_tokens: self.cache_read_input_tokens + o.cache_read_input_tokens,
        }
    }
}

#[derive(Deserialize)]
struct Message {
    #[rusty_serde(default)]
    id: Option<String>,
    #[rusty_serde(default)]
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Line {
    #[rusty_serde(rename = "type")]
    kind: String,
    #[rusty_serde(default)]
    usage: Option<Usage>,
    #[rusty_serde(default)]
    message: Option<Message>,
}

/// The tokens one harness printed and whether its final `result` line was
/// there. Lines that are not JSON, or carry no usage, are skipped. Assistant
/// messages are counted once per message id (a message is printed once per
/// content block).
pub fn parse(stream: &str) -> (Usage, bool) {
    let mut seen: HashSet<String> = HashSet::new();
    let mut assistant = Usage::default();
    for line in stream.lines() {
        let Ok(l) = rusty_serde::json::from_str::<Line>(line) else {
            continue;
        };
        match (l.kind.as_str(), l.usage, l.message) {
            ("result", Some(u), _) => return (u, true),
            ("assistant", _, Some(Message { id, usage: Some(u) })) => {
                if id.is_none_or(|id| seen.insert(id)) {
                    assistant = assistant.plus(u);
                }
            }
            _ => {}
        }
    }
    (assistant, false)
}

/// One turn's tokens.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TurnTokens {
    pub turn: u64,
    pub role: String,
    /// The turn was revoked: its work was thrown away.
    pub revoked: bool,
    /// The harness printed a final `result`; false is a lower bound.
    pub complete: bool,
    pub usage: Usage,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TaskTokens {
    pub turns: Vec<TurnTokens>,
    /// Every turn, revoked ones included.
    pub total: u64,
    pub revoked: u64,
    /// Turns that were granted but left no harness output (no launcher, or it
    /// never started): their cost is unknown, not zero.
    pub without_output: u64,
}

/// Per-turn tokens of `task`, from its log and the harness output under `dir`.
pub fn of(dir: &Path, task: &TaskId, events: &[Event]) -> TaskTokens {
    let revoked: HashSet<TurnId> = events
        .iter()
        .filter_map(|e| match e {
            Event::TurnEnded {
                turn,
                cause: TurnEnd::Revoked,
            } => Some(*turn),
            _ => None,
        })
        .collect();
    let mut out = TaskTokens {
        turns: Vec::new(),
        total: 0,
        revoked: 0,
        without_output: 0,
    };
    for e in events {
        let Event::TurnGranted { role, turn, .. } = e else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(log_path(dir, task, *turn)) else {
            out.without_output += 1;
            continue;
        };
        let (usage, complete) = parse(&text);
        let revoked = revoked.contains(turn);
        out.total += usage.total();
        if revoked {
            out.revoked += usage.total();
        }
        out.turns.push(TurnTokens {
            turn: turn.0,
            role: role.0.clone(),
            revoked,
            complete,
            usage,
        });
    }
    out
}

/// [`of`] as one line of JSON.
pub fn json(dir: &Path, task: &TaskId) -> Result<String, String> {
    let d = crate::open_driver(dir, task)?;
    let events = d.store.events(task, 0).map_err(|e| format!("{e:?}"))?;
    rusty_serde::json::to_string(&of(dir, task, &events)).map_err(|e| e.to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const RESULT: &str = r#"{"type":"result","subtype":"success","usage":{"input_tokens":10,"output_tokens":5,"cache_creation_input_tokens":100,"cache_read_input_tokens":1000,"server_tool_use":{"web_search_requests":0}},"total_cost_usd":0.1}"#;

    fn assistant(id: &str, input: u64, output: u64) -> String {
        format!(
            r#"{{"type":"assistant","message":{{"id":"{id}","usage":{{"input_tokens":{input},"output_tokens":{output}}}}}}}"#
        )
    }

    #[test]
    fn the_result_line_is_authoritative() {
        let s = format!("{}\nnot json\n{RESULT}\n", assistant("m1", 1, 1));
        let (u, complete) = parse(&s);
        assert!(complete);
        assert_eq!((u.input_tokens, u.output_tokens, u.total()), (10, 5, 1115));
    }

    #[test]
    fn a_killed_harness_is_summed_from_assistant_messages_once_per_id() {
        // The same message printed twice (two content blocks) counts once.
        let s = [
            assistant("m1", 3, 2),
            assistant("m1", 3, 2),
            assistant("m2", 4, 1),
        ]
        .join("\n");
        let (u, complete) = parse(&s);
        assert!(!complete);
        assert_eq!((u.input_tokens, u.output_tokens), (7, 3));
    }

    #[test]
    fn output_with_no_usage_is_zero_and_incomplete() {
        assert_eq!(
            parse("hello\n{\"type\":\"system\"}\n"),
            (Usage::default(), false)
        );
    }

    #[test]
    fn a_task_sums_every_turn_and_marks_revoked_and_missing_output() {
        let dir = std::env::temp_dir().join(format!("bbp-tokens-{}", std::process::id()));
        let task = TaskId("T".into());
        let t = |n| TurnId(n);
        let granted = |n, role: &str| Event::TurnGranted {
            role: RoleId(role.into()),
            turn: t(n),
            kind: TurnKind::Default,
            deadline: Time(0),
            return_to: None,
            slice: None,
        };
        for (n, body) in [(1, RESULT.to_owned()), (2, assistant("m", 7, 3))] {
            let p = log_path(&dir, &task, t(n));
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        }
        let events = [
            granted(1, "planner"),
            granted(2, "coder2"),
            granted(3, "tester"), // no output
            Event::TurnEnded {
                turn: t(2),
                cause: TurnEnd::Revoked,
            },
        ];
        let got = of(&dir, &task, &events);
        assert_eq!(got.total, 1115 + 10);
        assert_eq!(got.revoked, 10);
        assert_eq!(got.without_output, 1);
        assert_eq!(
            got.turns
                .iter()
                .map(|x| (x.role.as_str(), x.revoked, x.complete))
                .collect::<Vec<_>>(),
            [("planner", false, true), ("coder2", true, false)]
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
