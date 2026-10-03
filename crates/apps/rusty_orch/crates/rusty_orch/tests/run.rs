//! The loop around the dispatcher, driven by the scripted fake.

use std::io;
use std::time::{Duration, Instant};

use orch_core::board::{Confidence, EntryKind};
use orch_core::task::Status;
use orch_dispatch::fake::{FakeAgent, Reply};
use orch_dispatch::{Ceiling, DispatchError};
use rusty_orch::input::parse;
use rusty_orch::report;
use rusty_orch::run::{execute, Console, Ended};

/// A console that hands out scripted answers and records what it saw.
#[derive(Default)]
struct Scripted {
    answers: Vec<Option<String>>,
    notes: Vec<String>,
    asked: Vec<String>,
}

impl Scripted {
    fn answering(answers: &[Option<&str>]) -> Self {
        Self {
            answers: answers.iter().rev().map(|a| a.map(str::to_owned)).collect(),
            ..Self::default()
        }
    }
}

impl Console for Scripted {
    fn note(&mut self, line: &str) {
        self.notes.push(line.to_owned());
    }

    fn ask(&mut self, question: &str) -> io::Result<Option<String>> {
        self.asked.push(question.to_owned());
        Ok(self.answers.pop().flatten())
    }
}

fn spec(max_calls: u32, tasks: &str) -> rusty_orch::input::Spec {
    parse(&format!(
        r#"{{"goal":"g","done_when":["d"],"out_of_scope":[],"wall_clock_secs":60,"max_calls":{max_calls},"stop":"checkpoint","tasks":{tasks}}}"#
    ))
    .expect("spec")
}

const RESEARCH: &str =
    r#"[{"role":"research","instruction":"i","acceptance":["a"],"max_calls":2}]"#;

fn finding() -> EntryKind {
    EntryKind::Finding {
        confidence: Confidence::High,
    }
}

fn frozen() -> impl Fn() -> Instant {
    let t = Instant::now();
    move || t
}

#[test]
fn a_plan_runs_to_finished_and_reports_both_ways() {
    let fake = FakeAgent::new([Reply::Write(vec![finding()])]);
    let mut console = Scripted::default();
    let s = execute(spec(3, RESEARCH), fake, &mut console, frozen(), None).expect("run");

    assert_eq!(s.ended, Ended::Finished);
    assert_eq!(s.ledger.calls(), 1);
    assert_eq!(s.board.live().count(), 1);
    assert_eq!(console.notes, vec!["run: finished after 1 call(s)"]);
    assert!(console.asked.is_empty());

    let text = report::text(&s);
    assert!(text.contains("ended: finished"), "{text}");
    assert!(
        text.contains("T-1 research  done     codex -> E-1"),
        "{text}"
    );
    assert!(text.contains("E-1 [finding] codex: fake Finding"), "{text}");

    let json = report::json(&s);
    assert_eq!(
        json.get("ended")
            .and_then(|e| e.get("kind"))
            .and_then(|k| k.as_str()),
        Some("finished")
    );
    assert_eq!(json.get("calls").and_then(|c| c.as_u64()), Some(1));
    let task = &json.get("tasks").and_then(|t| t.as_array()).expect("tasks")[0];
    assert_eq!(task.get("status").and_then(|s| s.as_str()), Some("done"));
    assert_eq!(task.get("agent").and_then(|s| s.as_str()), Some("codex"));
    let entry = &json
        .get("entries")
        .and_then(|t| t.as_array())
        .expect("entries")[0];
    assert_eq!(entry.get("kind").and_then(|s| s.as_str()), Some("finding"));
    assert_eq!(
        entry.get("confidence").and_then(|s| s.as_str()),
        Some("high")
    );
    assert_eq!(
        entry.get("superseded").and_then(|s| s.as_bool()),
        Some(false)
    );
}

#[test]
fn a_question_blocks_when_nobody_answers() {
    let fake = FakeAgent::new([Reply::Write(vec![EntryKind::Question])]);
    let mut console = Scripted::answering(&[None]);
    let s = execute(spec(3, RESEARCH), fake, &mut console, frozen(), None).expect("run");

    let task = s.plan.tasks()[0].id();
    assert_eq!(s.ended, Ended::Blocked(vec![task]));
    assert_eq!(console.asked.len(), 1, "the question was offered once");
    assert!(
        console.asked[0].starts_with("E-1: fake Question"),
        "{:?}",
        console.asked
    );
    assert_eq!(s.board.open_questions().len(), 1);
    assert!(report::text(&s).contains("blocked on 1 card(s)"));
}

#[test]
fn an_answer_resumes_the_card_on_the_same_run() {
    let fake = FakeAgent::new([
        Reply::Write(vec![EntryKind::Question]),
        Reply::Write(vec![finding()]),
    ]);
    let mut console = Scripted::answering(&[Some("yes, go ahead")]);
    let s = execute(spec(3, RESEARCH), fake, &mut console, frozen(), None).expect("run");

    assert_eq!(s.ended, Ended::Finished);
    assert_eq!(s.ledger.calls(), 2);
    assert!(s.board.open_questions().is_empty());
    let answer = s
        .board
        .live()
        .find(|e| matches!(e.content().kind, EntryKind::Answer { .. }))
        .expect("answer");
    assert_eq!(answer.content().body.as_str(), "yes, go ahead");
    assert_eq!(answer.content().author, orch_core::board::Author::Human);
    assert_eq!(
        console.notes.len(),
        2,
        "one progress line per dispatcher run"
    );
}

#[test]
fn a_blank_answer_is_ignored_and_the_run_stays_blocked() {
    let fake = FakeAgent::new([Reply::Write(vec![EntryKind::Question])]);
    let mut console = Scripted::answering(&[Some("   ")]);
    let s = execute(spec(3, RESEARCH), fake, &mut console, frozen(), None).expect("run");

    assert!(matches!(s.ended, Ended::Blocked(_)));
    assert!(
        console
            .notes
            .iter()
            .any(|n| n.contains("blank answer ignored")),
        "{:?}",
        console.notes
    );
    assert_eq!(s.board.entries().len(), 1, "nothing appended");
}

#[test]
fn the_wall_clock_is_checked_before_every_run() {
    let fake = FakeAgent::new([
        Reply::Write(vec![EntryKind::Question]),
        Reply::Write(vec![finding()]),
    ]);
    let mut console = Scripted::answering(&[Some("yes")]);
    // The first check passes; the second finds the budget spent.
    let start = Instant::now();
    let calls = std::cell::Cell::new(0u32);
    let now = move || {
        let n = calls.get();
        calls.set(n + 1);
        if n <= 1 {
            start
        } else {
            start + Duration::from_secs(60)
        }
    };
    let s = execute(spec(3, RESEARCH), fake, &mut console, now, None).expect("run");

    assert_eq!(s.ended, Ended::WallClock(Duration::from_secs(60)));
    assert_eq!(s.ledger.calls(), 1, "the second run never started");
    assert!(report::text(&s).contains("wall clock of 60s exceeded"));
}

#[test]
fn a_dispatcher_error_ends_the_run_with_the_state_intact() {
    let fake = FakeAgent::new([Reply::Fail("boom".into()), Reply::Fail("boom".into())]);
    let mut console = Scripted::default();
    let s = execute(spec(3, RESEARCH), fake, &mut console, frozen(), None).expect("run");

    assert!(
        matches!(s.ended, Ended::Failed(DispatchError::Agent { .. })),
        "{:?}",
        s.ended
    );
    assert_eq!(s.plan.tasks()[0].state().status(), Status::Running);
    assert_eq!(s.ledger.calls(), 1);
    let json = report::json(&s);
    assert_eq!(
        json.get("ended")
            .and_then(|e| e.get("kind"))
            .and_then(|k| k.as_str()),
        Some("failed")
    );
    assert!(json
        .get("ended")
        .and_then(|e| e.get("error"))
        .and_then(|k| k.as_str())
        .expect("error")
        .contains("boom"));
}

#[test]
fn the_goal_ceiling_stops_the_second_card_as_a_typed_failure() {
    let two = r#"[{"role":"research","instruction":"i","acceptance":["a"],"max_calls":1},
                  {"role":"research","instruction":"j","acceptance":["a"],"max_calls":1}]"#;
    let fake = FakeAgent::new([Reply::Write(vec![finding()]), Reply::Write(vec![finding()])]);
    let mut console = Scripted::default();
    let s = execute(spec(1, two), fake, &mut console, frozen(), None).expect("run");

    let second = s.plan.tasks()[1].id();
    assert_eq!(
        s.ended,
        Ended::Failed(DispatchError::CeilingReached {
            ceiling: Ceiling::Goal,
            task: second,
            limit: std::num::NonZeroU32::new(1).expect("nz"),
        })
    );
    assert_eq!(s.plan.tasks()[0].state().status(), Status::Done);
    assert_eq!(s.plan.tasks()[1].state().status(), Status::Pending);
    assert_eq!(s.ledger.calls(), 1);
}

#[test]
fn a_blank_after_an_earlier_answer_stops_without_another_model_call() {
    let two = r#"[{"role":"research","instruction":"i","acceptance":["a"],"max_calls":2},
                  {"role":"research","instruction":"j","acceptance":["a"],"max_calls":2}]"#;
    // Both cards ask; a wrongful third call would consume this finding.
    let fake = FakeAgent::new([
        Reply::Write(vec![EntryKind::Question]),
        Reply::Write(vec![EntryKind::Question]),
        Reply::Write(vec![finding()]),
    ]);
    let mut console = Scripted::answering(&[Some("first answer"), None]);
    let s = execute(spec(10, two), fake, &mut console, frozen(), None).expect("run");

    assert!(
        matches!(&s.ended, Ended::Blocked(ids) if ids.len() == 2),
        "{:?}",
        s.ended
    );
    assert_eq!(console.asked.len(), 2, "both questions were offered");
    assert_eq!(s.ledger.calls(), 2, "no call after the human stopped");
    let answers = s
        .board
        .live()
        .filter(|e| matches!(e.content().kind, EntryKind::Answer { .. }))
        .count();
    assert_eq!(answers, 1, "the earlier answer is preserved on the board");
    assert_eq!(s.board.open_questions().len(), 1);
}

#[test]
fn progress_notes_name_the_category_and_never_the_agents_message() {
    const SENTINEL: &str = "SENTINEL-2b9d-must-not-leak";
    let fake = FakeAgent::new([Reply::Fail(format!("model said {SENTINEL}"))]);
    let mut console = Scripted::default();
    let s = execute(spec(3, RESEARCH), fake, &mut console, frozen(), None).expect("run");

    assert!(matches!(
        s.ended,
        Ended::Failed(DispatchError::Agent { .. })
    ));
    assert_eq!(
        console.notes,
        vec!["run: stopped: Codex failed T-1 (see report) after 1 call(s)"]
    );
    assert!(
        report::text(&s).contains(SENTINEL),
        "the report still carries the detail"
    );
}
