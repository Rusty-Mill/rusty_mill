//! The inbox tools (#595) over a fake `Escalations`: admin only, only the
//! tasks linked from cards, the card attached, and the rev check before
//! anything is sent.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use rusty_mill_board::api::{CallError, Identity, Shared, ops};
use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::domain::ExecutionLink;
use rusty_mill_board::inbox::{
    Action, DryRun, Escalation, Escalations, InboxError, Outcome, Response, plan,
};
use rusty_mill_board::store::{MemStore, Store};

/// Escalations held in memory; records the tasks asked for and the actions taken.
#[derive(Default)]
struct Fake {
    rows: Vec<Escalation>,
    asked: Mutex<Vec<Vec<String>>>,
    acted: Mutex<Vec<(String, Action, String)>>,
}

impl Escalations for Fake {
    fn open(&self, tasks: &[String]) -> Result<Vec<Escalation>, InboxError> {
        self.asked.lock().unwrap().push(tasks.to_vec());
        Ok(self
            .rows
            .iter()
            .filter(|e| tasks.contains(&e.task))
            .cloned()
            .collect())
    }
    fn preview(&self, e: &Escalation, a: &Action) -> Result<DryRun, InboxError> {
        let mut steps = plan(e, a)?.into_iter();
        Ok(DryRun {
            step: steps.next().expect("a step"),
            response: Response::Accepted(json!("Ok")),
            events: vec![],
            then: steps.collect(),
        })
    }
    fn act(&self, e: &Escalation, a: &Action, by: &str) -> Result<Outcome, InboxError> {
        plan(e, a)?;
        self.acted
            .lock()
            .unwrap()
            .push((e.id.clone(), a.clone(), by.to_owned()));
        Ok(Outcome::Done)
    }
}

fn escalation(task: &str, rev: u64) -> Escalation {
    serde_json::from_value(json!({
        "id": format!("{task}/msg:1/3"), "task": task, "rev": rev, "seq": 3,
        "reason": "Irreversible",
        "claim": {"id": 1, "text": "ship it", "status": "Escalated", "score": 12,
                  "confidence": 96, "epoch": 0, "spent": 2, "budget": 16},
        "stakes": "High", "reversible": false, "evidence": [], "candidates": [],
        "mark": null, "candidate": null
    }))
    .unwrap()
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

/// A board with one linked card (task T1) and one unlinked card; the fake
/// also holds an escalation of T9, which no card links.
fn setup() -> (Arc<Shared>, Arc<Fake>) {
    let fake = Arc::new(Fake {
        rows: vec![escalation("T1", 7), escalation("T9", 1)],
        ..Fake::default()
    });
    let board: Shared = Board::new(
        Box::new(MemStore::default()) as Box<dyn Store>,
        Roles::default(),
    )
    .with_escalations(Arc::clone(&fake) as Arc<dyn Escalations>);
    let nano = board.roles().actor("nano", "human").unwrap();
    let card = board.add(&nano, "ship the release", "", vec![]).unwrap();
    board.bind_execution(card.id, link("T1"), &nano).unwrap();
    board.add(&nano, "unlinked", "", vec![]).unwrap();
    (Arc::new(board), fake)
}

fn call(b: &Shared, who: &Identity, name: &str, args: Value) -> Result<Value, CallError> {
    ops()
        .iter()
        .find(|o| o.name == name)
        .expect("op")
        .call(b, Some(who), args)
}

fn owner() -> Identity {
    Identity {
        actor: "nano".into(),
        role: "human".into(),
        projects: None,
    }
}

fn refused(r: Result<Value, CallError>) -> String {
    match r {
        Err(CallError::Board(e)) => e.to_string(),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn the_list_shows_only_linked_tasks_with_their_card() {
    let (b, fake) = setup();
    let out = call(&b, &owner(), "inbox_list", json!({})).unwrap();
    let rows = out.as_array().unwrap();
    assert_eq!(rows.len(), 1, "{out}");
    assert_eq!(rows[0]["id"], "T1/msg:1/3");
    assert_eq!(rows[0]["label"], "Irreversible action");
    assert_eq!(rows[0]["card"]["title"], "ship the release");
    assert_eq!(*fake.asked.lock().unwrap(), [vec!["T1".to_owned()]]);
}

#[test]
fn a_model_token_is_forbidden() {
    let (b, fake) = setup();
    let model = Identity {
        actor: "codex".into(),
        role: "builder".into(),
        projects: None,
    };
    assert!(refused(call(&b, &model, "inbox_list", json!({}))).contains("Admin"));
    let act = json!({"id": "T1/msg:1/3", "rev": 7, "action": "approve"});
    assert!(refused(call(&b, &model, "inbox_act", act)).contains("Admin"));
    assert!(fake.acted.lock().unwrap().is_empty());
}

/// Without a transport-bound credential (open board, shared token, stdio) the
/// caller names its own actor and role; naming the owner must not reach BBP.
#[test]
fn a_caller_named_human_without_a_credential_is_refused() {
    let (b, fake) = setup();
    let unbound = |name: &str, args: Value| {
        ops()
            .iter()
            .find(|o| o.name == name)
            .expect("op")
            .call(&b, None, args)
    };
    let spoof = json!({"actor": "nano", "role": "human"});
    assert!(refused(unbound("inbox_list", spoof)).contains("authenticated admin credential"));
    let act = json!({"actor": "nano", "role": "human", "id": "T1/msg:1/3", "rev": 7, "action": "approve"});
    assert!(refused(unbound("inbox_act", act)).contains("authenticated admin credential"));
    assert!(fake.asked.lock().unwrap().is_empty());
    assert!(fake.acted.lock().unwrap().is_empty());
}

#[test]
fn approve_at_the_read_rev_reaches_bbp_as_the_owner() {
    let (b, fake) = setup();
    let out = call(
        &b,
        &owner(),
        "inbox_act",
        json!({"id": "T1/msg:1/3", "rev": 7, "action": "approve"}),
    )
    .unwrap();
    assert_eq!(out, json!({"outcome": "Done"}));
    assert_eq!(
        *fake.acted.lock().unwrap(),
        [("T1/msg:1/3".to_owned(), Action::Approve, "nano".to_owned())]
    );
}

#[test]
fn a_stale_rev_or_closed_escalation_sends_nothing() {
    let (b, fake) = setup();
    let stale = call(
        &b,
        &owner(),
        "inbox_act",
        json!({"id": "T1/msg:1/3", "rev": 6, "action": "approve"}),
    );
    assert!(refused(stale).contains("stale_rev"));
    let gone = call(
        &b,
        &owner(),
        "inbox_act",
        json!({"id": "T1/msg:9/3", "rev": 7, "action": "approve"}),
    );
    assert!(refused(gone).contains("no open escalation"));
    let unlinked = call(
        &b,
        &owner(),
        "inbox_act",
        json!({"id": "T9/msg:1/3", "rev": 1, "action": "approve"}),
    );
    assert!(
        refused(unlinked).contains("no open escalation"),
        "a task no card links is not the board's"
    );
    assert!(fake.acted.lock().unwrap().is_empty());
}

#[test]
fn a_dry_run_writes_nothing_and_a_bad_action_is_refused() {
    let (b, fake) = setup();
    let dry = call(
        &b,
        &owner(),
        "inbox_act",
        json!({"id": "T1/msg:1/3", "rev": 7, "action": "reject", "dry_run": true}),
    )
    .unwrap();
    assert_eq!(dry["dry_run"]["step"]["verb"], "claim");
    let choose = call(
        &b,
        &owner(),
        "inbox_act",
        json!({"id": "T1/msg:1/3", "rev": 7, "action": "choose", "chosen": 4}),
    );
    assert!(refused(choose).contains("choose is for a stuck question"));
    let refine = call(
        &b,
        &owner(),
        "inbox_act",
        json!({"id": "T1/msg:1/3", "rev": 7, "action": "refine", "note": ""}),
    );
    assert!(refused(refine).contains("1 to 1200"));
    assert!(fake.acted.lock().unwrap().is_empty());
}

#[test]
fn without_a_bbp_host_the_inbox_says_how_to_enable_it() {
    let board: Shared = Board::new(
        Box::new(MemStore::default()) as Box<dyn Store>,
        Roles::default(),
    );
    assert!(refused(call(&board, &owner(), "inbox_list", json!({}))).contains("--bbp-bin"));
}
