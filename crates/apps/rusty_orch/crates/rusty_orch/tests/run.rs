//! The loop around the dispatcher, driven by the scripted fake.

use std::io;
use std::time::{Duration, Instant};

use orch_core::board::{Confidence, EntryKind};
use orch_core::task::Status;
use orch_core::Text;
use orch_dispatch::fake::{FakeAgent, Reply};
use orch_dispatch::{Ceiling, DispatchError};
use orch_store::{fingerprint, Store};
use rusty_orch::input::parse;
use rusty_orch::millboard::CardLinks;
use rusty_orch::report;
use rusty_orch::run::{
    execute, execute_hosted, execute_resumable, CardHosting, Console, Ended, Hosting, Resume,
    RunError, Summary,
};

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
    let s = execute(spec(3, RESEARCH), fake, &mut console, frozen()).expect("run");

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
    let s = execute(spec(3, RESEARCH), fake, &mut console, frozen()).expect("run");

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
    let s = execute(spec(3, RESEARCH), fake, &mut console, frozen()).expect("run");

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
    let s = execute(spec(3, RESEARCH), fake, &mut console, frozen()).expect("run");

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
    let s = execute(spec(3, RESEARCH), fake, &mut console, now).expect("run");

    assert_eq!(s.ended, Ended::WallClock(Duration::from_secs(60)));
    assert_eq!(s.ledger.calls(), 1, "the second run never started");
    assert!(report::text(&s).contains("wall clock of 60s exceeded"));
}

#[test]
fn a_dispatcher_error_ends_the_run_with_the_state_intact() {
    let fake = FakeAgent::new([Reply::Fail("boom".into()), Reply::Fail("boom".into())]);
    let mut console = Scripted::default();
    let s = execute(spec(3, RESEARCH), fake, &mut console, frozen()).expect("run");

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
    let s = execute(spec(1, two), fake, &mut console, frozen()).expect("run");

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
    let s = execute(spec(10, two), fake, &mut console, frozen()).expect("run");

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
    let s = execute(spec(3, RESEARCH), fake, &mut console, frozen()).expect("run");

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

#[test]
fn an_io_error_after_one_of_two_answers_checkpoints_the_partial_round() {
    struct FailSecond {
        asks: usize,
    }
    impl Console for FailSecond {
        fn note(&mut self, _line: &str) {}
        fn ask(&mut self, _question: &str) -> io::Result<Option<String>> {
            self.asks += 1;
            if self.asks == 1 {
                Ok(Some("kept answer".to_owned()))
            } else {
                Err(io::Error::other("input disappeared"))
            }
        }
    }

    let two = r#"[{"role":"research","instruction":"i","acceptance":["a"],"max_calls":2},
                  {"role":"research","instruction":"j","acceptance":["a"],"max_calls":2}]"#;
    let spec = spec(10, two);
    let dir =
        std::env::temp_dir().join(format!("rusty_orch_partial_answers_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut store = Store::open(&dir).expect("open");
    let fake = FakeAgent::new([
        Reply::Write(vec![EntryKind::Question]),
        Reply::Write(vec![EntryKind::Question]),
    ]);
    let err = execute_resumable(
        spec,
        fake,
        &mut FailSecond { asks: 0 },
        frozen(),
        Some(Resume {
            store: &mut store,
            fingerprint: fingerprint("two-question-goal"),
        }),
    )
    .expect_err("second read fails");
    assert!(matches!(err, RunError::Io(_)));
    drop(store);

    let store = Store::open(&dir).expect("reopen");
    let saved = store
        .load(orch_core::GoalId::from_raw(1))
        .expect("load")
        .expect("saved state");
    assert_eq!(saved.ledger.calls(), 2, "no dispatcher call was replayed");
    assert_eq!(saved.board.open_questions().len(), 1);
    let answer = saved
        .board
        .entries()
        .iter()
        .find(|entry| matches!(entry.content().kind, EntryKind::Answer { .. }))
        .expect("first answer persisted");
    assert_eq!(answer.content().body.as_str(), "kept answer");
    let _ = std::fs::remove_dir_all(dir);
}

/// Under best effort the adapters withdraw `question` (orch-cli tests), so a
/// card that would have asked writes an assumption and finishes. Nothing is
/// asked of the console, and the report shows the assumption.
#[test]
fn a_best_effort_run_records_assumptions_and_never_asks() {
    let spec = parse(
        r#"{"goal":"g","done_when":["d"],"out_of_scope":[],"wall_clock_secs":60,"max_calls":3,"stop":"best_effort",
            "tasks":[{"role":"research","instruction":"i","acceptance":["a"],"max_calls":2}]}"#,
    )
    .expect("spec");
    assert_eq!(
        spec.goal.budget().stop(),
        orch_core::goal::StopRule::BestEffort
    );
    let fake = FakeAgent::new([Reply::Write(vec![EntryKind::Assumption, finding()])]);
    let mut console = Scripted::default();
    let s = execute(spec, fake, &mut console, frozen()).expect("run");

    assert_eq!(s.ended, Ended::Finished);
    assert!(
        console.asked.is_empty(),
        "nobody was asked: {:?}",
        console.asked
    );
    let text = report::text(&s);
    assert!(
        text.contains("assumptions (taken as true, unverified):\n  E-1 codex: fake Assumption"),
        "{text}"
    );
    assert!(!report::text(&s).contains("waits on"), "{text}");
}

#[test]
fn the_assumptions_block_is_absent_when_none_were_made() {
    let fake = FakeAgent::new([Reply::Write(vec![finding()])]);
    let mut console = Scripted::default();
    let s = execute(spec(3, RESEARCH), fake, &mut console, frozen()).expect("run");
    assert!(!report::text(&s).contains("assumptions ("));
}

/// A card on a board with Millboard's compare-and-set, shared across runs.
#[derive(Clone, Default)]
struct Card(std::rc::Rc<std::cell::RefCell<(Option<u64>, Vec<String>)>>);

impl CardLinks for Card {
    fn stored(&mut self, _: &str) -> Result<Option<u64>, String> {
        Ok(self.0.borrow().0)
    }
    fn link(&mut self, link: &orch_core::factory::ExecutionLink) -> Result<(), String> {
        let mut card = self.0.borrow_mut();
        card.1.push(format!("link {}", link.generation));
        card.0 = Some(link.generation);
        Ok(())
    }
    fn recover(
        &mut self,
        expected: u64,
        link: &orch_core::factory::ExecutionLink,
    ) -> Result<(), String> {
        let mut card = self.0.borrow_mut();
        card.1
            .push(format!("recover {expected}->{}", link.generation));
        if card.0 != Some(expected) {
            return Err("execution generation changed".into());
        }
        card.0 = Some(link.generation);
        Ok(())
    }
}

const IMPLEMENT: &str =
    r#"[{"role":"implement","instruction":"i","acceptance":["a"],"max_calls":2}]"#;

fn hosted(
    store: &mut Store,
    card: &Card,
    host: &orch_dispatch::fake::FakeHost,
) -> Result<Summary, RunError> {
    hosted_for(store, Some(card), host)
}

fn hosted_for(
    store: &mut Store,
    card: Option<&Card>,
    host: &orch_dispatch::fake::FakeHost,
) -> Result<Summary, RunError> {
    hosted_with(store, card, host, None)
}

fn cancelled(
    store: &mut Store,
    card: &Card,
    host: &orch_dispatch::fake::FakeHost,
) -> Result<Summary, RunError> {
    hosted_with(
        store,
        Some(card),
        host,
        Text::new("owner withdrew the card"),
    )
}

fn hosted_with(
    store: &mut Store,
    card: Option<&Card>,
    host: &orch_dispatch::fake::FakeHost,
    cancel: Option<Text>,
) -> Result<Summary, RunError> {
    execute_hosted(
        spec(5, IMPLEMENT),
        FakeAgent::new([]),
        &mut Scripted::default(),
        frozen(),
        Some(Resume {
            store,
            fingerprint: fingerprint("hosted-goal"),
        }),
        Some(Hosting {
            host: Box::new(host.clone()),
            card: card.map(|card| CardHosting {
                card: "card".into(),
                namespace: "namespace".into(),
                board: Box::new(card.clone()),
            }),
            cancel,
        }),
    )
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rusty_orch_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn each_hosted_run_records_its_generation_with_the_card_before_hosting() {
    let dir = scratch("card_generations");
    let card = Card::default();
    let host = orch_dispatch::fake::FakeHost::default();
    for _ in 0..2 {
        let mut store = Store::open(&dir).expect("open");
        let s = hosted(&mut store, &card, &host).expect("run");
        assert!(matches!(s.ended, Ended::Waiting(_)), "{}", s.ended);
    }
    assert_eq!(card.0.borrow().1, ["link 1", "recover 1->2"]);
    let mut generations: Vec<u64> = host
        .0
        .borrow()
        .fences
        .iter()
        .map(|f| f.generation)
        .collect();
    generations.dedup();
    assert_eq!(
        generations,
        [1, 2],
        "each run hosts under the generation it linked"
    );
    let factory = Store::open(&dir)
        .expect("reopen")
        .load_factory(orch_core::GoalId::from_raw(1))
        .expect("factory");
    let link = factory.link("card").expect("linked");
    assert_eq!(
        (
            link.execution.as_str(),
            link.bbp_task.as_str(),
            link.generation
        ),
        ("namespace", "namespace", 2)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_card_held_at_a_later_generation_stops_the_run_before_any_host_command() {
    let dir = scratch("card_superseded");
    let card = Card::default();
    card.0.borrow_mut().0 = Some(7);
    let host = orch_dispatch::fake::FakeHost::default();
    let mut store = Store::open(&dir).expect("open");
    let error = hosted(&mut store, &card, &host).expect_err("superseded");
    assert!(
        matches!(&error, RunError::Lease(why) if why.starts_with("superseded")),
        "{error}"
    );
    assert!(card.0.borrow().1.is_empty());
    assert_eq!(host.0.borrow().opens, 0);
    assert!(host.0.borrow().fences.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_card_linked_state_resumed_without_its_card_is_refused_before_any_generation() {
    let dir = scratch("card_omitted");
    let card = Card::default();
    let host = orch_dispatch::fake::FakeHost::default();
    let mut store = Store::open(&dir).expect("open");
    hosted(&mut store, &card, &host).expect("linked run");
    drop(store);
    let fences = host.0.borrow().fences.len();
    let mut store = Store::open(&dir).expect("reopen");
    let error = hosted_for(&mut store, None, &host).expect_err("card omitted");
    assert!(
        matches!(&error, RunError::Lease(why) if why.contains("linked to card card")),
        "{error}"
    );
    assert_eq!(host.0.borrow().fences.len(), fences, "nothing was hosted");
    assert_eq!(card.0.borrow().1, ["link 1"]);
    let factory = store
        .load_factory(orch_core::GoalId::from_raw(1))
        .expect("factory");
    let saved = factory.assignment("goal-orchestrator").expect("assignment");
    assert_eq!(saved.generation, 1, "no unpublished generation was saved");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_cancel_fences_earlier_holders_cancels_each_hosted_task_and_retires_the_goal() {
    let dir = scratch("cancel");
    let card = Card::default();
    let host = orch_dispatch::fake::FakeHost::default();
    let mut store = Store::open(&dir).expect("open");
    hosted(&mut store, &card, &host).expect("hosted run");
    drop(store);

    let mut store = Store::open(&dir).expect("reopen");
    let s = cancelled(&mut store, &card, &host).expect("cancel");
    assert_eq!(s.ended, Ended::Cancelled);
    let script = host.0.borrow();
    assert_eq!(script.cancels.len(), 1, "the one hosted task was cancelled");
    assert_eq!(
        script.fences.last().map(|f| f.generation),
        Some(2),
        "the cancel ran under a generation newer than the run it stops"
    );
    drop(script);
    assert_eq!(card.0.borrow().1, ["link 1", "recover 1->2"]);
    let factory = store
        .load_factory(orch_core::GoalId::from_raw(1))
        .expect("factory");
    let assignment = factory.assignment("goal-orchestrator").expect("assignment");
    assert!(!assignment.active, "the goal's assignment is retired");
    assert_eq!(
        factory.link("card").map(|l| l.state),
        Some(orch_core::factory::ExecutionState::Cancelled)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_cancelled_goal_is_never_resumed_and_a_second_cancel_does_nothing() {
    let dir = scratch("cancel_twice");
    let card = Card::default();
    let host = orch_dispatch::fake::FakeHost::default();
    let mut store = Store::open(&dir).expect("open");
    hosted(&mut store, &card, &host).expect("hosted run");
    cancelled(&mut store, &card, &host).expect("cancel");
    let commands = host.0.borrow().fences.len();

    let s = cancelled(&mut store, &card, &host).expect("cancel again");
    assert_eq!(s.ended, Ended::Cancelled);
    let error = hosted(&mut store, &card, &host).expect_err("resume refused");
    assert!(
        matches!(&error, RunError::Lease(why) if why.contains("Retired")),
        "{error}"
    );
    assert_eq!(host.0.borrow().fences.len(), commands, "nothing was hosted");
    assert_eq!(host.0.borrow().cancels.len(), 1);
    assert_eq!(card.0.borrow().1, ["link 1", "recover 1->2"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_cancel_the_host_could_not_finish_leaves_the_goal_to_cancel_again() {
    let dir = scratch("cancel_retry");
    let card = Card::default();
    let host = orch_dispatch::fake::FakeHost::default();
    let mut store = Store::open(&dir).expect("open");
    hosted(&mut store, &card, &host).expect("hosted run");
    host.0
        .borrow_mut()
        .fail_cancel
        .push_back(orch_dispatch::ClassifiedError::Transient(
            orch_dispatch::AgentError("moderator still running after cancel".into()),
        ));
    let error = cancelled(&mut store, &card, &host).expect_err("not reaped yet");
    assert!(
        matches!(&error, RunError::Lease(why) if why.contains("still running")),
        "{error}"
    );
    let factory = store
        .load_factory(orch_core::GoalId::from_raw(1))
        .expect("factory");
    assert!(
        factory
            .assignment("goal-orchestrator")
            .is_some_and(|a| a.active),
        "nothing is retired before the host confirms its processes are gone"
    );

    let s = cancelled(&mut store, &card, &host).expect("cancel retried");
    assert_eq!(s.ended, Ended::Cancelled);
    assert_eq!(
        card.0.borrow().1,
        ["link 1", "recover 1->2", "recover 2->3"]
    );
    let _ = std::fs::remove_dir_all(&dir);
}
