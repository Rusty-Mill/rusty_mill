//! Test doubles and fixtures shared by every CLI adapter's tests.
//!
//! Behind the `fake` feature. [`FakeCommand`] records each call and returns
//! a canned result; the board fixtures give one Research card referencing
//! one of two human findings.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::num::NonZeroU32;
use std::sync::Mutex;
use std::time::Duration;

use crate::{CommandRunner, ExecError, Exit};
use orch_core::board::{Author, Board, Confidence, EntryKind, NewEntry};
use orch_core::task::{Plan, Role, Task, TaskSpec};
use orch_core::{EntryId, GoalId, Ref, Text};

/// A `Text` from a literal that is known to be non-blank.
pub fn text(s: &str) -> Text {
    Text::new(s).expect("non-blank")
}

/// A Research card with two acceptance criteria and the given refs.
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

/// The single card of a [`fixture`] plan.
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

/// ADR-0004 example reply 1: one high-confidence finding with two refs.
pub const REPLY_ONE: &str = r#"{"entries":[{"kind":"finding","confidence":"high","body":"Plan::start rejects a reviewer equal to the target's author.","refs":["E-1","path:crates/orch-core/src/task.rs"]}]}"#;

/// ADR-0004 example reply 2: a medium finding and an open question.
pub const REPLY_TWO: &str = r#"{"entries":[{"kind":"finding","confidence":"medium","body":"Board::live hides superseded entries but keeps them addressable by id.","refs":["E-1"]},{"kind":"question","body":"Should Local ever review Codex output?","refs":[]}]}"#;

/// One recorded invocation: argv, stdin bytes, timeout.
pub type Call = (Vec<String>, Vec<u8>, Duration);

/// Scripted process: records every call, returns the canned result.
pub struct FakeCommand {
    result: Result<Exit, ExecError>,
    /// Every `(argv, stdin, timeout)` the adapter asked for, in order.
    pub calls: Mutex<Vec<Call>>,
    /// The `remove_env` list passed with each call, in the same order.
    pub scrubbed: Mutex<Vec<Vec<String>>>,
}

impl FakeCommand {
    /// A fake that returns `result` on every call.
    pub fn new(result: Result<Exit, ExecError>) -> Self {
        Self {
            result,
            calls: Mutex::new(Vec::new()),
            scrubbed: Mutex::new(Vec::new()),
        }
    }

    /// A fake that exits 0 with `stdout` and empty stderr.
    pub fn ok(stdout: &str) -> Self {
        Self::new(Ok(exit(0, stdout, "")))
    }
}

/// An [`Exit`] from string stdout and stderr.
pub fn exit(status: i32, stdout: &str, stderr: &str) -> Exit {
    Exit {
        status,
        stdout: stdout.as_bytes().to_vec(),
        stderr: stderr.as_bytes().to_vec(),
    }
}

impl CommandRunner for FakeCommand {
    fn run_scrubbed(
        &self,
        argv: &[String],
        stdin: &[u8],
        timeout: Duration,
        remove_env: &[&str],
    ) -> Result<Exit, ExecError> {
        let mut calls = self.calls.lock().map_err(|_| poisoned())?;
        let mut scrubbed = self.scrubbed.lock().map_err(|_| poisoned())?;
        calls.push((argv.to_vec(), stdin.to_vec(), timeout));
        scrubbed.push(remove_env.iter().map(|s| (*s).to_owned()).collect());
        self.result.clone()
    }
}

fn poisoned() -> ExecError {
    ExecError::Io("fake command: lock poisoned".to_owned())
}
