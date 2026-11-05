//! A snapshot survives a reopen, every lifecycle and entry shape round
//! trips, and the store refuses what it should.

use std::num::NonZeroU32;
use std::path::PathBuf;

use orch_core::board::{Author, Board, Confidence, EntryKind, NewEntry, Verdict};
use orch_core::task::{AgentId, Plan, Role, TaskSpec};
use orch_core::{EntryId, GoalId, Ref, TaskId, Text};
use orch_dispatch::Ledger;
use orch_store::{fingerprint, GoalState, Store, StoreError};
use rusty_multimodal_db_engine::test_support::fresh_temp_dir;
use rusty_multimodal_db_engine::test_support::{arm_fault, Fault};

const CODEX: AgentId = AgentId::from_static("codex");
const LOCAL: AgentId = AgentId::from_static("local");

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
    let codex = Author::Agent(CODEX);
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
            Author::Agent(LOCAL),
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

    plan.start(t1, CODEX).expect("start");
    plan.complete(t1, vec![e1]).expect("complete");
    plan.start(t2, CODEX).expect("start");
    plan.block(t2, e2).expect("block");
    plan.start(t3, LOCAL).expect("start");
    plan.start(t4, CODEX).expect("start");
    plan.fail(t4, text("no adapter")).expect("fail");
    plan.start(t5, LOCAL).expect("start");
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
    state.plan.start(t6, CODEX).expect("start");
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
    before.plan.start(t6, CODEX).expect("start");
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
        "the replacement log does not advance the existing scannable slot"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn failed_initial_insert_can_recover_with_a_synthesized_first_revision() {
    let dir = dir("insert_after_log");
    let state = full_state();
    let mut store = Store::open(&dir).expect("open");

    arm_fault(Fault::InsertAfterLog);
    assert!(matches!(store.save(&state), Err(StoreError::Engine(_))));
    drop(store);

    let store = Store::open(&dir).expect("portable reopen");
    let recovered = store.load(state.goal()).expect("load").expect("record");
    assert_eq!(recovered, state, "the durable insert log is recovered");
    assert_eq!(
        store.revision(state.goal()),
        1,
        "recovery synthesizes the initial scannable slot despite the save error"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn assignments_generations_and_links_survive_a_reopen() {
    use orch_core::factory::{
        Assignment, ExecutionLink, ExecutionState, Factory, FactoryError, Grant, Identity, Lease,
    };
    let dir = dir("assignments");
    let goal = GoalId::from_raw(1);
    let identity = || Identity {
        id: "orch".into(),
        target: "rusty_orch run".into(),
    };
    let grant = |projects: &[&str], delegation_budget, call_budget| Grant {
        projects: projects.iter().map(|p| (*p).into()).collect(),
        delegation_budget,
        call_budget,
        lease: Some(Lease {
            read: vec![format!("/src/{call_budget}")],
            write: vec!["/src".into()],
            expires: Some(100 + u64::from(call_budget)),
        }),
    };
    let mut factory = Factory::default();
    {
        let mut store = Store::open(&dir).expect("open");
        assert_eq!(store.load_factory(goal).expect("load"), Factory::default());
        factory
            .take_over("root", identity(), grant(&["orch"], 1, 4))
            .expect("first holder");
        factory
            .delegate(
                "root",
                1,
                Assignment {
                    id: "worker".into(),
                    generation: 1,
                    identity: identity(),
                    parent: Some("root".into()),
                    grant: grant(&["orch"], 0, 2),
                    active: true,
                },
            )
            .expect("delegate");
        factory
            .initiate_or_adopt(ExecutionLink {
                card: "card".into(),
                execution: "execution".into(),
                goal: "goal".into(),
                bbp_task: "bbp".into(),
                attempt: "attempt".into(),
                assignment: "root".into(),
                generation: 1,
                state: ExecutionState::Running,
            })
            .expect("link");
        store.save_factory(goal, &factory).expect("save");
        factory
            .take_over("root", identity(), grant(&["orch"], 1, 4))
            .expect("second holder");
        store.save_factory(goal, &factory).expect("save again");
    }
    let store = Store::open(&dir).expect("reopen");
    let loaded = store.load_factory(goal).expect("load");
    assert_eq!(loaded, factory);
    assert_eq!(loaded.authorize("root", 2), Ok(()));
    assert_eq!(
        loaded.authorize("worker", 1),
        Err(FactoryError::StaleGeneration)
    );
    assert_eq!(
        store.load_factory(GoalId::from_raw(2)).expect("other goal"),
        Factory::default()
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

#[test]
fn hosted_handle_gate_fence_and_charge_survive_a_store_reopen() {
    use orch_core::goal::{Goal, GoalDraft, StopRule};
    use orch_dispatch::{
        fake::{FakeAgent, FakeHost},
        Dispatcher, GateContext, Outcome, Progress, Routing, RoutingConfig,
    };
    let dir = dir("hosted");
    let goal = Goal::try_from(GoalDraft {
        outcome: Some("ship".into()),
        done_when: vec!["tests pass".into()],
        out_of_scope: Some(vec![]),
        max_calls: Some(1),
        wall_clock_secs: Some(60),
        stop: Some(StopRule::Checkpoint),
        ..GoalDraft::default()
    })
    .unwrap();
    let mut plan = Plan::new(GoalId::from_raw(1));
    let id = plan.add(spec(Role::Implement, &[])).unwrap();
    let mut board = Board::new(plan.goal());
    let mut ledger = Ledger::new();
    let host = FakeHost::default();
    host.0
        .borrow_mut()
        .progress
        .push_back(Ok(Progress::Gate(GateContext {
            prompt: text("approve-plan art:2"),
            rev: 42,
            ids: vec![text("art:2")],
        })));
    let route = Routing::try_from(RoutingConfig {
        research: CODEX,
        design: CODEX,
        implement: CODEX,
        triage: LOCAL,
        reviewers: vec![LOCAL, CODEX],
    })
    .unwrap();
    let make = || {
        Dispatcher::new(route.clone(), FakeAgent::new([])).with_host(
            host.clone(),
            orch_dispatch::Fence {
                assignment: text("goal-orchestrator"),
                generation: 1,
                holder: text("test"),
                lease: None,
            },
        )
    };
    let mut dispatcher = make();
    assert_eq!(
        dispatcher.run(&goal, &mut plan, &mut board, &mut ledger),
        Ok(Outcome::Blocked(vec![id]))
    );
    let original = GoalState {
        fingerprint: 42,
        plan,
        board,
        ledger,
    };
    {
        let mut store = Store::open(&dir).unwrap();
        store.save(&original).unwrap();
    }
    drop(dispatcher);
    let mut loaded = Store::open(&dir)
        .unwrap()
        .load(GoalId::from_raw(1))
        .unwrap()
        .unwrap();
    assert_eq!(loaded, original);
    let question = loaded.board.open_questions()[0].content();
    assert!(question.refs.contains(&Ref::Url(text("host:rev:42"))));
    assert_eq!(loaded.ledger.calls(), 1);
    let mut dispatcher = make();
    assert_eq!(
        dispatcher.run(
            &goal,
            &mut loaded.plan,
            &mut loaded.board,
            &mut loaded.ledger
        ),
        Ok(Outcome::Blocked(vec![id]))
    );
    assert_eq!(host.0.borrow().opens, 1);
    assert_eq!(loaded.ledger.calls(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}

mod meters {
    use orch_lens::meter::{Ceilings, Meter, Snapshot};
    use orch_store::{Store, StoreError};

    use super::dir;

    const CEIL: Ceilings = Ceilings {
        experiment: 100,
        task: 50,
        task_calls: 10,
    };

    fn snapshot() -> Snapshot {
        let mut m = Meter::new(CEIL, 3);
        let t = m.reserve("task-1", 4).expect("room");
        m.settle(t, 2);
        let _held = m.reserve("task-1", 5).expect("room");
        m.snapshot()
    }

    #[test]
    fn a_meter_survives_reopening_and_restores_held_reservations_as_spent() {
        let d = dir("meter_reopen");
        let snap = snapshot();
        {
            let mut s = Store::open(&d).expect("open");
            assert_eq!(s.load_meter("dedup").expect("load"), None);
            s.save_meter("dedup", &snap).expect("save");
        }
        let s = Store::open(&d).expect("reopen");
        let loaded = s.load_meter("dedup").expect("load").expect("saved");
        assert_eq!(loaded, snap);
        let m = Meter::restore(CEIL, &loaded);
        assert_eq!((m.task_spent("task-1"), m.calls("task-1")), (2 + 5, 2));
        assert_eq!(m.reserved(), 0);
        assert_eq!(s.load_meter("other").expect("load"), None, "per experiment");
    }

    #[test]
    fn a_later_save_replaces_the_meter() {
        let d = dir("meter_replace");
        let mut s = Store::open(&d).expect("open");
        s.save_meter("dedup", &snapshot()).expect("first");
        let mut m = Meter::restore(CEIL, &snapshot());
        let t = m.reserve("task-2", 1).expect("room");
        m.settle(t, 1);
        s.save_meter("dedup", &m.snapshot()).expect("second");
        let loaded = s.load_meter("dedup").expect("load").expect("saved");
        assert_eq!(loaded, m.snapshot());
    }

    #[test]
    fn a_meter_listing_a_task_twice_is_refused() {
        let d = dir("meter_dupe");
        let mut s = Store::open(&d).expect("open");
        let bad = Snapshot {
            experiment_spent: 0,
            tasks: vec![("t".to_owned(), 1, 1), ("t".to_owned(), 2, 2)],
            reserved: Vec::new(),
        };
        s.save_meter("dedup", &bad).expect("save");
        assert!(matches!(s.load_meter("dedup"), Err(StoreError::Corrupt(_))));
    }
}

mod lens_states {
    use orch_lens::saved::{Saved, SavedPass, SavedProgress, SavedQuestion};
    use orch_store::Store;

    use super::dir;

    fn saved() -> Saved {
        Saved {
            progress: vec![SavedProgress {
                lens: "adversary".to_owned(),
                claim: 7,
                blind: Some(SavedPass {
                    provider: "claude".to_owned(),
                    posted: 1,
                    found_counter: true,
                }),
                look_harder: None,
                foreign_seen: 2,
                epoch: 1,
            }],
            questions: vec![SavedQuestion {
                claim: 3,
                used: vec!["codex".to_owned()],
                settled: vec![0],
                tried: vec![(None, vec!["local".to_owned()])],
                rebel_landed: false,
                rebel_failures: 1,
                attacks: vec![(11, 1, 2)],
            }],
        }
    }

    #[test]
    fn a_boards_runner_memory_survives_reopening() {
        let d = dir("lens_state_reopen");
        {
            let mut s = Store::open(&d).expect("open");
            assert_eq!(s.load_lens_state("task-1").expect("load"), None);
            s.save_lens_state("task-1", &saved()).expect("save");
        }
        let s = Store::open(&d).expect("reopen");
        assert_eq!(s.load_lens_state("task-1").expect("load"), Some(saved()));
        assert_eq!(
            s.load_lens_state("task-2").expect("load"),
            None,
            "per board"
        );
    }

    #[test]
    fn a_later_save_replaces_the_memory() {
        let d = dir("lens_state_replace");
        let mut s = Store::open(&d).expect("open");
        s.save_lens_state("task-1", &saved()).expect("first");
        s.save_lens_state("task-1", &Saved::default())
            .expect("second");
        assert_eq!(
            s.load_lens_state("task-1").expect("load"),
            Some(Saved::default())
        );
    }

    #[test]
    fn the_memory_and_a_meter_of_the_same_name_are_kept_apart() {
        let d = dir("lens_state_meter");
        let mut s = Store::open(&d).expect("open");
        s.save_lens_state("dedup", &saved()).expect("save");
        assert!(matches!(s.load_meter("dedup"), Ok(None)));
        assert_eq!(s.load_lens_state("dedup").expect("load"), Some(saved()));
    }
}

mod probes {
    use orch_lens::alien::{judge, summarize, Bar, HuntRow, Outcome, ProbeRow};
    use orch_store::Store;

    use super::dir;

    fn row(task: &str, claim: u64, site: &str, outcome: Outcome) -> ProbeRow {
        ProbeRow {
            task: task.to_owned(),
            claim,
            candidate: 9,
            site: site.to_owned(),
            mechanism: "rs-path".to_owned(),
            novel: true,
            outcome,
            runs: 2,
            probe_sha256: "ab".repeat(32),
        }
    }

    #[test]
    fn probe_rows_survive_reopening_sorted_and_filtered_by_task() {
        let d = dir("probe_reopen");
        let rows = [
            row("t2", 1, "a::f", Outcome::Pass),
            row("t1", 7, "b::g", Outcome::Regression),
            row("t1", 3, "a::f", Outcome::PreExisting),
        ];
        {
            let mut s = Store::open(&d).expect("open");
            assert!(s.probes(None).expect("empty").is_empty());
            for r in &rows {
                s.save_probe(r).expect("save");
            }
        }
        let s = Store::open(&d).expect("reopen");
        assert_eq!(
            s.probes(None).expect("all"),
            [rows[2].clone(), rows[1].clone(), rows[0].clone()]
        );
        assert_eq!(s.probes(Some("t2")).expect("t2"), [rows[0].clone()]);
        assert!(s.probes(Some("t3")).expect("none").is_empty());
    }

    #[test]
    fn saving_the_same_probe_again_replaces_it() {
        let d = dir("probe_replace");
        let mut s = Store::open(&d).expect("open");
        s.save_probe(&row("t", 1, "a::f", Outcome::Error))
            .expect("first");
        let fixed = row("t", 1, "a::f", Outcome::Regression);
        s.save_probe(&fixed).expect("second");
        assert_eq!(s.probes(None).expect("load"), [fixed]);
    }

    fn hunt(task: &str, claim: u64, probes: u32) -> HuntRow {
        HuntRow {
            task: task.to_owned(),
            claim,
            candidate: 9,
            suspects: 4,
            probes,
            tokens: 500,
            cost: 0,
            started: 100 + claim,
        }
    }

    #[test]
    fn twenty_zero_probe_hunts_survive_reopening_and_are_judged() {
        let d = dir("hunt_zero");
        {
            let mut s = Store::open(&d).expect("open");
            for c in 0..20 {
                s.save_hunt(&hunt("t", c, 0)).expect("save");
            }
            s.save_hunt(&hunt("t", 0, 0)).expect("save again");
        }
        let s = Store::open(&d).expect("reopen");
        let hunts = s.hunts(None).expect("hunts");
        assert_eq!(hunts.len(), 20, "idempotent: one row per hunt");
        let summary = summarize(&hunts, &s.probes(None).expect("probes"));
        assert_eq!((summary.hunts, summary.probes), (20, 0));
        assert_eq!(judge(&hunts, &[], None), Bar::RateNotMet);
        assert_eq!(hunts[0].started, 100, "start time survives reopening");
    }

    #[test]
    fn a_mixed_cohort_counts_each_hunt_once() {
        let d = dir("hunt_mixed");
        let mut s = Store::open(&d).expect("open");
        s.save_hunt(&hunt("t", 1, 1)).expect("hunt with a probe");
        s.save_hunt(&hunt("t", 2, 0)).expect("hunt without");
        s.save_hunt(&hunt("u", 1, 0)).expect("other task");
        s.save_probe(&row("t", 1, "a::f", Outcome::Regression))
            .expect("probe");
        let all = summarize(&s.hunts(None).expect("h"), &s.probes(None).expect("p"));
        assert_eq!((all.hunts, all.probes, all.regressions), (3, 1, 1));
        let t = summarize(
            &s.hunts(Some("t")).expect("h"),
            &s.probes(Some("t")).expect("p"),
        );
        assert_eq!((t.hunts, t.probes, t.tokens), (2, 1, 1_000));
    }
}

mod watch {
    use orch_store::{PendingPost, Store, WatchFinding, WatchSlot};

    use super::dir;

    fn row(key: &str, claim: u64, pending: Option<&str>) -> WatchSlot {
        WatchSlot {
            posted: Some(WatchFinding {
                key: key.to_owned(),
                claim,
            }),
            pending: pending.map(|k| PendingPost {
                key: k.to_owned(),
                post: format!("{{\"op\":\"{k}\"}}"),
            }),
        }
    }

    #[test]
    fn a_watch_slot_survives_reopening_and_is_per_app_and_slot() {
        let d = dir("watch_reopen");
        {
            let mut s = Store::open(&d).expect("open");
            let empty = s.watch_slot("demo", "updates").expect("load");
            assert_eq!(empty, WatchSlot::default());
            let pending = row("updates:aa", 3, Some("updates:bb"));
            s.save_watch_slot("demo", "updates", &pending)
                .expect("save");
        }
        let mut s = Store::open(&d).expect("reopen");
        let got = s.watch_slot("demo", "updates").expect("load");
        assert_eq!(got, row("updates:aa", 3, Some("updates:bb")));
        assert_eq!(
            s.watch_slot("other", "updates").expect("load"),
            WatchSlot::default()
        );
        let tool = s.watch_slot("demo", "toolchain:rust").expect("load");
        assert_eq!(tool, WatchSlot::default());
        s.save_watch_slot("demo", "updates", &row("updates:bb", 7, None))
            .expect("replace");
        let got = s.watch_slot("demo", "updates").expect("load");
        assert_eq!(got, row("updates:bb", 7, None), "a later save replaces it");
    }
}
