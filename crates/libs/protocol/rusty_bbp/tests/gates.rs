#![allow(clippy::expect_used, clippy::unwrap_used)]
//! #447 (D190, D191): a task's human gates. `task_opened.gates` is written only for a scratch
//! task, so owned logs keep their bytes, and claims mode refuses it.

mod common;
use common::*;
use rusty_bbp::*;

fn open(mode: Mode, gates: Option<Gates>) -> (Driver<MemStore>, Response) {
    let mut store = MemStore::new();
    let brief = store.blob_put(b"brief");
    let task = TaskId("T1".into());
    let mut d = Driver::new(store, task.clone());
    let cmd = Command::Open(OpenTask {
        task,
        roster: four_roles(),
        repo: "r".into(),
        profile_digest: DIGEST,
        budget: default_budget(),
        turn_ms: 60_000,
        request_ms: 3_600_000,
        max_live_turns: 1,
        brief,
        human: PrincipalId("human".into()),
        policy: SchedulingPolicy::default(),
        mode,
        claims: None,
        holdout_digest: None,
        baseline_digest: None,
        gates,
    });
    let r = d.dispatch(&cmd, Time(1)).expect("dispatch");
    (d, r)
}

fn opened_json(d: &Driver<MemStore>) -> String {
    let events = d.store.events(&d.state.task, 0).expect("events");
    rusty_serde::json::to_string(&events[0]).expect("json")
}

#[test]
fn an_owned_task_writes_no_gates_field() {
    let (d, _) = open(Mode::Stages, None);
    assert!(d.state.opened);
    assert_eq!(d.state.gates, None);
    assert!(!opened_json(&d).contains("gates"), "{}", opened_json(&d));
}

#[test]
fn a_scratch_task_records_its_gates_and_replays_them() {
    let (d, _) = open(Mode::Stages, Some(Gates::Scratch));
    assert_eq!(d.state.gates, Some(Gates::Scratch));
    assert!(
        opened_json(&d).contains(r#""gates":"scratch""#),
        "{}",
        opened_json(&d)
    );
    let events = d.store.events(&d.state.task, 0).expect("events");
    assert_eq!(
        TaskState::fold(d.state.task.clone(), &events).gates,
        Some(Gates::Scratch)
    );
}

#[test]
fn claims_mode_refuses_scratch_gates() {
    let (d, r) = open(Mode::Claims, Some(Gates::Scratch));
    let rej = rejection(r);
    assert_eq!(rej.code, Code::BadClaimsConfig);
    assert!(rej.detail.contains("gates"), "{}", rej.detail);
    assert!(!d.state.opened);
}
