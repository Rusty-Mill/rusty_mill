//! Shared fixtures for the adapter tests.
#![allow(dead_code)]

use std::num::NonZeroU32;
use std::sync::Mutex;
use std::time::Duration;

use orch_core::board::{Author, Board, Confidence, EntryKind, NewEntry};
use orch_core::task::{Plan, Role, Task, TaskSpec};
use orch_core::{EntryId, GoalId, Ref, Text};
use orch_ollama::{CommandRunner, ExecError, Exit};

pub fn text(s: &str) -> Text {
    Text::new(s).expect("non-blank")
}

pub fn research_spec(refs: Vec<Ref>) -> TaskSpec {
    TaskSpec {
        role: Role::Research,
        instruction: text("Explain how Plan::start enforces no self-review."),
        acceptance: vec![text("cites the function"), text("one finding minimum")],
        refs,
        depends_on: vec![],
        max_calls: NonZeroU32::new(2).expect("non-zero"),
    }
}

/// A plan with one research card and a board with two human findings; the
/// card references only the first.
pub fn fixture() -> (Plan, Board, EntryId, EntryId) {
    let goal = GoalId::from_raw(1);
    let mut board = Board::new(goal);
    let referenced = board
        .append(finding("Plan::start rejects the author."))
        .expect("append");
    let unreferenced = board.append(finding("Unrelated note.")).expect("append");
    let mut plan = Plan::new(goal);
    plan.add(research_spec(vec![
        Ref::Entry(referenced),
        Ref::Path(text("crates/orch-core/src/task.rs")),
    ]))
    .expect("add");
    (plan, board, referenced, unreferenced)
}

pub fn task(plan: &Plan) -> &Task {
    plan.tasks().first().expect("one card")
}

fn finding(body: &str) -> NewEntry {
    NewEntry {
        task: None,
        author: Author::Human,
        kind: EntryKind::Finding {
            confidence: Confidence::High,
        },
        body: text(body),
        refs: vec![],
        supersedes: None,
    }
}

pub const REPLY_ONE: &str = r#"{"entries":[{"kind":"finding","confidence":"high","body":"Plan::start rejects a reviewer equal to the target's author.","refs":["E-1","path:crates/orch-core/src/task.rs"]}]}"#;

pub const REPLY_TWO: &str = r#"{"entries":[{"kind":"finding","confidence":"medium","body":"Board::live hides superseded entries but keeps them addressable by id.","refs":["E-1"]},{"kind":"question","body":"Should Local ever review Codex output?","refs":[]}]}"#;

/// One recorded invocation: argv, stdin bytes, timeout.
pub type Call = (Vec<String>, Vec<u8>, Duration);

/// Scripted process: records every call, returns the canned result.
pub struct FakeCommand {
    result: Result<Exit, ExecError>,
    pub calls: Mutex<Vec<Call>>,
}

impl FakeCommand {
    pub fn new(result: Result<Exit, ExecError>) -> Self {
        Self {
            result,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub fn ok(stdout: &str) -> Self {
        Self::new(Ok(exit(0, stdout, "")))
    }
}

pub fn exit(status: i32, stdout: &str, stderr: &str) -> Exit {
    Exit {
        status,
        stdout: stdout.as_bytes().to_vec(),
        stderr: stderr.as_bytes().to_vec(),
    }
}

impl CommandRunner for FakeCommand {
    fn run(&self, argv: &[String], stdin: &[u8], timeout: Duration) -> Result<Exit, ExecError> {
        self.calls
            .lock()
            .expect("lock")
            .push((argv.to_vec(), stdin.to_vec(), timeout));
        self.result.clone()
    }
}
