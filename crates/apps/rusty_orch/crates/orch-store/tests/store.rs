//! A snapshot survives a reopen, every lifecycle and entry shape round
//! trips, and the store refuses what it should.

use std::num::NonZeroU32;
use std::path::PathBuf;

use orch_core::board::{Author, Board, Confidence, EntryKind, NewEntry, Verdict};
use orch_core::task::{Agent, Plan, Role, TaskSpec};
use orch_core::{EntryId, GoalId, Ref, TaskId, Text};
use orch_dispatch::Ledger;
use orch_store::{fingerprint, GoalState, Store, StoreError};
use rusty_multimodal_db_engine::test_support::fresh_temp_dir;
use rusty_multimodal_db_engine::test_support::{arm_fault, Fault};

fn dir(label: &str) -> PathBuf {
    fresh_temp_dir(&format!("orch_store_{label}")).expect("temp dir")
}

fn text(s: &str) -> Text {
    Text::new(s).expect("non-blank")
}

fn spec(role: Role, depends_on: &[u64]) -> TaskSpec {
    TaskSpec {
        role,
        instruction: text("do it"),
        acceptance: vec![text("done")],
        refs: vec![
            Ref::Path(text("src/lib.rs")),
            Ref::Commit(text("abc123")),
            Ref::Url(text("https://example.invalid")),
        ],
        depends_on: depends_on.iter().map(|d| TaskId::from_raw(*d)).collect(),
        max_calls: NonZeroU32::new(3).expect("non-zero"),
    }
}

fn entry(task: Option<u64>, author: Author, kind: EntryKind, refs: Vec<Ref>) -> NewEntry {
    NewEntry {
        task: task.map(TaskId::from_raw),
        author,
        kind,
        body: text("body"),
        refs,
        supersedes: None,
    }
}

/// Every `TaskState` and every `EntryKind`, plus a supersession, on one
/// goal, built the way the dispatcher would build them.
fn full_state() -> GoalState {
    let goal = GoalId::from_raw(1);
    let mut plan = Plan::new(goal);
    let t1 = plan.add(spec(Role::Research, &[])).expect("add");
    let t2 = plan.add(spec(Role::Design, &[1])).expect("add");
    let t3 = plan.add(spec(Role::Triage, &[])).expect("add");
    let t4 = plan.add(spec(Role::Implement, &[])).expect("add");
    let t5 = plan
        .add(spec(Role::Review { target: t1 }, &[]))
        .expect("add");
    let _t6 = plan.add(spec(Role::Research, &[])).expect("add");

    let mut board = Board::new(goal);
    let codex = Author::Agent(Agent::Codex);
    let e1 = board
        .append(entry(
            Some(1),
            codex,
            EntryKind::Finding {
                confidence: Confidence::Medium,
            },
            vec![],
        ))
        .expect("append");
    let e2 = board
        .append(entry(
            Some(1),
            codex,
            EntryKind::Question,
            vec![Ref::Entry(e1)],
        ))
        .expect("append");
    let e3 = board
        .append(entry(
            None,
            Author::Human,
            EntryKind::Answer { to: e2 },
            vec![],
        ))
        .expect("append");
    let e4 = board
        .append(entry(
            Some(5),
            Author::Agent(Agent::Local),
            EntryKind::Review {
                of: t1,
                verdict: Verdict::Approve,
            },
            vec![Ref::Entry(e1)],
        ))
        .expect("append");
    let e5 = board
        .append(entry(
            Some(1),
            codex,
            EntryKind::Decision,
            vec![Ref::Entry(e4)],
        ))
        .expect("append");
    let e6 = board
        .append(entry(Some(1), codex, EntryKind::Assumption, vec![]))
        .expect("append");
    let _e7 = board
        .append(entry(
            Some(1),
            codex,
            EntryKind::Artifact,
            vec![Ref::Path(text("out.md"))],
        ))
        .expect("append");
    let superseding = NewEntry {
        supersedes: Some(e6),
        ..entry(Some(1), codex, EntryKind::Assumption, vec![])
    };
    board.append(superseding).expect("append");
    let _ = (e3, e5);

    plan.start(t1, Agent::Codex).expect("start");
    plan.complete(t1, vec![e1]).expect("complete");
    plan.start(t2, Agent::Codex).expect("start");
    plan.block(t2, e2).expect("block");
    plan.start(t3, Agent::Local).expect("start");
    plan.start(t4, Agent::Codex).expect("start");
    plan.fail(t4, text("no adapter")).expect("fail");
    plan.start(t5, Agent::Local).expect("start");
    plan.complete(t5, vec![e4]).expect("complete");

    let ledger = Ledger::from_counts(5, [(t1, 1), (t2, 2), (t4, 1), (t5, 1)]);
    GoalState {
        fingerprint: fingerprint("{\"goal\":\"g\"}"),
        plan,
        board,
        ledger,
    }
}

#[test]
fn a_snapshot_round_trips_through_a_reopen() {
    let dir = dir("roundtrip");
    let state = full_state();
    {
        let mut store = Store::open(&dir).expect("open");
        assert_eq!(store.load(state.goal()).expect("load"), None);
        store.save(&state).expect("save");
        assert_eq!(store.revision(state.goal()), 1);
    }
    let store = Store::open(&dir).expect("reopen");
    let loaded = store.load(state.goal()).expect("load").expect("saved");
    assert_eq!(loaded, state);
    assert_eq!(store.revision(state.goal()), 1);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn saving_again_replaces_the_snapshot_and_bumps_the_revision() {
    let dir = dir("replace");
    let mut state = full_state();
    let mut store = Store::open(&dir).expect("open");
    store.save(&state).expect("first save");
    let t6 = TaskId::from_raw(6);
    state.plan.start(t6, Agent::Codex).expect("start");
    state.ledger = Ledger::from_counts(6, [(t6, 1)]);
    store.save(&state).expect("second save");
    assert_eq!(store.revision(state.goal()), 2);
    drop(store);
    let store = Store::open(&dir).expect("reopen");
    let loaded = store.load(state.goal()).expect("load").expect("saved");
    assert_eq!(loaded, state);
    assert_eq!(loaded.ledger.calls(), 6);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn failed_replace_can_recover_new_aggregate_with_previous_revision() {
    let dir = dir("replace_after_log");
    let mut before = full_state();
    let mut store = Store::open(&dir).expect("open");
    store.save(&before).expect("first save");

    let t6 = TaskId::from_raw(6);
    before.plan.start(t6, Agent::Codex).expect("start");
    before
        .board
        .append(entry(
            Some(6),
            Author::Human,
            EntryKind::Artifact,
            vec![Ref::Path(text("recovered.txt"))],
        ))
        .expect("append");
    before.fingerprint = 99;
    before.ledger = Ledger::from_counts(6, [(t6, 1)]);
    arm_fault(Fault::ReplaceAfterLog);
    assert!(matches!(store.save(&before), Err(StoreError::Engine(_))));
    drop(store);

    let store = Store::open(&dir).expect("portable reopen");
    let recovered = store.load(before.goal()).expect("load").expect("record");
    assert_eq!(
        recovered, before,
        "the whole aggregate payload is recovered"
    );
    assert_eq!(
        store.revision(before.goal()),
        1,
        "the separate revision slot records only completed checkpoints"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_directory_is_held_by_one_process_at_a_time() {
    let dir = dir("lock");
    let first = Store::open(&dir).expect("open");
    match Store::open(&dir) {
        Err(StoreError::Locked(held)) => assert_eq!(held, dir),
        Err(other) => panic!("expected Locked, got {other}"),
        Ok(_) => panic!("a second open succeeded"),
    }
    drop(first);
    Store::open(&dir).expect("reopens once released");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_plan_and_board_for_different_goals_are_refused() {
    let dir = dir("mismatch");
    let mut store = Store::open(&dir).expect("open");
    let state = GoalState {
        fingerprint: 0,
        plan: Plan::new(GoalId::from_raw(1)),
        board: Board::new(GoalId::from_raw(2)),
        ledger: Ledger::new(),
    };
    match store.save(&state) {
        Err(StoreError::GoalMismatch { plan, board }) => {
            assert_eq!((plan.get(), board.get()), (1, 2));
        }
        other => panic!("expected GoalMismatch, got {other:?}"),
    }
    assert_eq!(store.revision(GoalId::from_raw(1)), 0);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn goals_are_kept_apart_by_id() {
    let dir = dir("two_goals");
    let mut store = Store::open(&dir).expect("open");
    let one = full_state();
    let two = GoalState {
        fingerprint: 7,
        plan: Plan::new(GoalId::from_raw(2)),
        board: Board::new(GoalId::from_raw(2)),
        ledger: Ledger::new(),
    };
    store.save(&one).expect("save one");
    store.save(&two).expect("save two");
    assert_eq!(store.load(GoalId::from_raw(2)).expect("load"), Some(two));
    assert_eq!(store.load(GoalId::from_raw(1)).expect("load"), Some(one));
    assert_eq!(store.load(GoalId::from_raw(3)).expect("load"), None);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_fingerprint_follows_the_text() {
    assert_eq!(fingerprint("a"), fingerprint("a"));
    assert_ne!(fingerprint("a"), fingerprint("a "));
    let _ = EntryId::from_raw(1);
}
