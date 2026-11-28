//! The claim-log anchor pass (#1551) over a scripted `bbp verify`: what it writes on a
//! linked card, what it passes as `--expect`, and that a rewrite is recorded once.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use rusty_mill_board::anchor::{ACTOR, Verified};
use rusty_mill_board::api::Shared;
use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::domain::ExecutionLink;
use rusty_mill_board::inbox::{Action, DryRun, Escalation, Escalations, InboxError, Outcome};
use rusty_mill_board::store::{MemStore, Store};
use uuid::Uuid;

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

type Answer = Result<Result<Verified, String>, InboxError>;

/// Answers `verify` from a script and records each `(task, expect)` it was asked.
#[derive(Default)]
struct Fake {
    answers: Mutex<VecDeque<Answer>>,
    asked: Mutex<Vec<(String, Option<String>)>>,
}

impl Escalations for Fake {
    fn open(&self, _tasks: &[String]) -> Result<Vec<Escalation>, InboxError> {
        Ok(Vec::new())
    }
    fn preview(&self, _e: &Escalation, _a: &Action) -> Result<DryRun, InboxError> {
        Err(InboxError::Unsupported("not in this test"))
    }
    fn act(
        &self,
        _e: &Escalation,
        _a: &Action,
        _by: &str,
        _decision: Option<&str>,
    ) -> Result<Outcome, InboxError> {
        Err(InboxError::Unsupported("not in this test"))
    }
    fn applied(&self, _task: &str, _decision: &str) -> Result<Option<u64>, InboxError> {
        Ok(None)
    }
    fn verify(&self, task: &str, expect: Option<&str>) -> Answer {
        self.asked
            .lock()
            .unwrap()
            .push((task.to_owned(), expect.map(str::to_owned)));
        self.answers
            .lock()
            .unwrap()
            .pop_front()
            .expect("a scripted answer")
    }
}

fn head(lines: u64, head: &str) -> Answer {
    Ok(Ok(Verified {
        lines,
        unchained: 0,
        head: head.to_owned(),
    }))
}

fn link(task: &str) -> ExecutionLink {
    ExecutionLink {
        execution: format!("x-{task}"),
        goal: "g".into(),
        bbp_task: task.into(),
        attempt: "a".into(),
        assignment: "orch".into(),
        generation: 1,
    }
}

/// A board with one card linked to task T1, answering `verify` from `answers`.
fn setup(answers: Vec<Answer>) -> (Shared, Arc<Fake>, Uuid) {
    let fake = Arc::new(Fake {
        answers: Mutex::new(answers.into()),
        ..Fake::default()
    });
    let board: Shared = Board::new(
        Box::new(MemStore::default()) as Box<dyn Store>,
        Roles::default(),
    )
    .with_escalations(Arc::clone(&fake) as Arc<dyn Escalations>);
    let nano = board.roles().actor("nano", "human").unwrap();
    let card = board.add(&nano, "run task T1", "", vec![]).unwrap();
    board.bind_execution(card.id, link("T1"), &nano).unwrap();
    (board, fake, card.id)
}

fn anchors(board: &Shared, id: Uuid) -> Vec<String> {
    board
        .get(id)
        .unwrap()
        .comments
        .into_iter()
        .filter(|c| c.by == ACTOR)
        .map(|c| c.text)
        .collect()
}

#[test]
fn a_head_is_anchored_then_checked_against() {
    let (board, fake, id) = setup(vec![head(3, A), head(3, A), head(5, B)]);
    assert_eq!(board.anchor_tick().unwrap(), 1);
    assert_eq!(board.anchor_tick().unwrap(), 0);
    assert_eq!(board.anchor_tick().unwrap(), 1);
    assert_eq!(
        anchors(&board, id),
        vec![
            format!("bbp-anchor v1 task=T1 lines=3 head={A}"),
            format!("bbp-anchor v1 task=T1 lines=5 head={B}"),
        ]
    );
    let expects: Vec<Option<String>> = fake
        .asked
        .lock()
        .unwrap()
        .iter()
        .map(|a| a.1.clone())
        .collect();
    assert_eq!(expects, vec![None, Some(A.into()), Some(A.into())]);
}

#[test]
fn a_rewrite_is_recorded_once_and_the_old_head_stays_expected() {
    let gone = || Ok(Err("--expect is not a line of this log".to_owned()));
    let (board, fake, id) = setup(vec![head(3, A), gone(), gone()]);
    board.anchor_tick().unwrap();
    assert_eq!(board.anchor_tick().unwrap(), 1);
    assert_eq!(board.anchor_tick().unwrap(), 0);
    assert_eq!(
        anchors(&board, id).last().unwrap(),
        "bbp-anchor v1 task=T1 FAILED: --expect is not a line of this log"
    );
    assert_eq!(fake.asked.lock().unwrap()[2].1.as_deref(), Some(A));
}

#[test]
fn a_host_that_cannot_be_asked_writes_nothing() {
    let (board, _fake, id) = setup(vec![Err(InboxError::Host("no such file".into()))]);
    assert_eq!(board.anchor_tick().unwrap(), 0);
    assert!(anchors(&board, id).is_empty());
}

#[test]
fn a_board_without_a_bbp_host_does_nothing() {
    let board: Shared = Board::new(
        Box::new(MemStore::default()) as Box<dyn Store>,
        Roles::default(),
    );
    assert_eq!(board.anchor_tick().unwrap(), 0);
}

#[test]
fn a_timed_out_verify_skips_only_its_task_and_the_pass_goes_on() {
    let stuck = Err(InboxError::Host(
        "`bbp verify` timed out after 120s; killed".into(),
    ));
    let (board, fake, first) = setup(vec![stuck, head(3, A)]);
    let nano = board.roles().actor("nano", "human").unwrap();
    let second = board.add(&nano, "run task T2", "", vec![]).unwrap().id;
    board.bind_execution(second, link("T2"), &nano).unwrap();
    assert_eq!(
        board.anchor_tick().unwrap(),
        1,
        "the other task is still anchored"
    );
    assert_eq!(fake.asked.lock().unwrap().len(), 2);
    let written: Vec<String> = [first, second]
        .into_iter()
        .flat_map(|id| anchors(&board, id))
        .collect();
    assert_eq!(written.len(), 1);
    assert!(
        written[0].contains(&format!("head={A}")),
        "a timeout is never FAILED: {written:?}"
    );
}
