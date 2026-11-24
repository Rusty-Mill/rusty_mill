//! The inbox tools (#595) over a fake `Escalations`: admin only, only the
//! tasks linked from cards, the card attached, and the rev check before
//! anything is sent.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use rusty_mill_board::api::{CallError, Identity, Shared, ops};
use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::domain::ExecutionLink;
use rusty_mill_board::domain::{BoardError, Card, Event};
use rusty_mill_board::inbox::{
    Action, DryRun, Escalation, Escalations, InboxError, Outcome, Response, plan,
};
use rusty_mill_board::projects::Projects;
use rusty_mill_board::store::{MemStore, Store};
use uuid::Uuid;

/// Escalations held in memory; records the tasks asked for and the actions taken.
/// With `consume`, a decision closes its escalation, as the real host's does; with
/// `lose_answer` it does so and then fails, as a host whose reply never arrived.
#[derive(Default)]
struct Fake {
    rows: Mutex<Vec<Escalation>>,
    consume: bool,
    lose_answer: AtomicBool,
    /// Refuse the command at this step; the ones before it commit.
    refuse_at: Option<usize>,
    /// The code a refusal carries (default `dep_closed`).
    refuse_code: Option<&'static str>,
    /// A refusal that comes from another send of the same decision winning the fence.
    winner_is_same_decision: bool,
    /// Fail before the host is reached: nothing is applied or recorded.
    fail_before: AtomicBool,
    /// The next send is refused at step 0 without writing anything (a tracked card stays).
    refuse_next: AtomicBool,
    /// The host predates `bbp decision`.
    old_host: AtomicBool,
    /// The host ignores the decision id it is sent (rolled back between probe and send).
    ignore_id: AtomicBool,
    /// `open` answers only a lone task, not a list (so the board's inbox cannot see rows).
    list_only_alone: AtomicBool,
    /// The decision ids the host applied, with the rev.
    decided: Mutex<Vec<String>>,
    /// The `verb args` of every command each act sent.
    planned: Mutex<Vec<Vec<String>>>,
    asked: Mutex<Vec<Vec<String>>>,
    acted: Mutex<Vec<(String, Action, String)>>,
}

impl Fake {
    fn with(rows: Vec<Escalation>) -> Self {
        Self {
            rows: Mutex::new(rows),
            ..Self::default()
        }
    }
}

impl Escalations for Fake {
    fn open(&self, tasks: &[String]) -> Result<Vec<Escalation>, InboxError> {
        self.asked.lock().unwrap().push(tasks.to_vec());
        if self.list_only_alone.load(Ordering::SeqCst) && tasks.len() != 1 {
            return Ok(Vec::new());
        }
        Ok(self
            .rows
            .lock()
            .unwrap()
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
    fn applied(&self, _task: &str, decision: &str) -> Result<Option<u64>, InboxError> {
        if self.old_host.load(Ordering::SeqCst) {
            return Err(InboxError::Host("unknown subcommand decision".into()));
        }
        let at = self
            .decided
            .lock()
            .unwrap()
            .iter()
            .position(|d| d == decision);
        Ok(at.map(|n| 8 + n as u64))
    }
    fn act(
        &self,
        e: &Escalation,
        a: &Action,
        by: &str,
        decision: Option<&str>,
    ) -> Result<Outcome, InboxError> {
        if self.fail_before.load(Ordering::SeqCst) {
            return Err(InboxError::Host("connection refused".into()));
        }
        if self.refuse_next.swap(false, Ordering::SeqCst) {
            return Ok(Outcome::Refused {
                step: 0,
                code: "dep_closed".into(),
                detail: "a dependency closed".into(),
            });
        }
        let steps = plan(e, a)?;
        self.planned.lock().unwrap().push(
            steps
                .iter()
                .map(|s| format!("{} {}", s.verb, s.args.join(" ")))
                .collect(),
        );
        self.acted
            .lock()
            .unwrap()
            .push((e.id.clone(), a.clone(), by.to_owned()));
        if self.consume && self.refuse_at.is_some() {
            // Another send of the decision won the fence first.
            self.rows.lock().unwrap().retain(|r| r.id != e.id);
        }
        let record = |this: &Self| {
            if let Some(id) = decision.filter(|_| !this.ignore_id.load(Ordering::SeqCst)) {
                this.decided.lock().unwrap().push(id.to_owned());
            }
        };
        if let Some(step) = self.refuse_at {
            if self.winner_is_same_decision {
                record(self);
            }
            return Ok(Outcome::Refused {
                step,
                code: self.refuse_code.unwrap_or("dep_closed").into(),
                detail: "a dependency closed".into(),
            });
        }
        record(self);
        if self.consume {
            self.rows.lock().unwrap().retain(|r| r.id != e.id);
        }
        if self.lose_answer.load(Ordering::SeqCst) {
            return Err(InboxError::Host("connection reset".into()));
        }
        Ok(Outcome::Done)
    }
}

fn escalation(task: &str, rev: u64) -> Escalation {
    serde_json::from_value(json!({
        "id": format!("{task}/msg:1/3"), "task": task, "brief": 2, "rev": rev, "seq": 3, "acted": [], "totals": [],
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
    let fake = Arc::new(Fake::with(vec![escalation("T1", 7), escalation("T9", 1)]));
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
        connector: false,
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
        connector: false,
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

/// A store whose inserts fail while `down` is set; the cards live on in `inner`, so
/// a new board over the same store stands for a restart.
struct Flaky {
    inner: Arc<MemStore>,
    down: Arc<AtomicBool>,
}

impl Store for Flaky {
    fn get(&self, id: Uuid) -> Result<Option<Card>, BoardError> {
        self.inner.get(id)
    }
    fn list(&self) -> Result<Vec<Card>, BoardError> {
        self.inner.list()
    }
    fn insert(&self, card: &Card, event: Event) -> Result<u64, BoardError> {
        if self.down.load(Ordering::SeqCst) {
            return Err(BoardError::Storage("disk full".into()));
        }
        self.inner.insert(card, event)
    }
    fn replace(&self, card: &Card, event: Event) -> Result<u64, BoardError> {
        self.inner.replace(card, event)
    }
    fn events_since(&self, after: Option<Uuid>, limit: usize) -> Result<Vec<Event>, BoardError> {
        self.inner.events_since(after, limit)
    }
    fn last_event_id(&self) -> Result<Option<Uuid>, BoardError> {
        self.inner.last_event_id()
    }
    fn reserve_number(&self, card: Uuid, floor: u64) -> Result<u64, BoardError> {
        self.inner.reserve_number(card, floor)
    }
    fn durable(&self) -> bool {
        true
    }
}

/// An escalation of T1 whose claim was already acted on (mark `pr-7`).
fn shipped() -> Escalation {
    let mut e = escalation("T1", 7);
    e.reason = rusty_mill_board::inbox::Reason::ReopenedAfterAction;
    e.acted = Some(vec![rusty_mill_board::inbox::Acted {
        claim: 1,
        marks: vec!["pr-7".into()],
    }]);
    e
}

/// A board over `store` with `fake` as its host. With `link`, it also adds the
/// card bound to T1 (the first boot; a restart finds it in the store).
fn board_over(
    store: &Arc<MemStore>,
    down: &Arc<AtomicBool>,
    fake: &Arc<Fake>,
    link_t1: bool,
) -> Shared {
    let flaky = Flaky {
        inner: Arc::clone(store),
        down: Arc::clone(down),
    };
    let board: Shared = Board::new(Box::new(flaky) as Box<dyn Store>, Roles::default())
        .with_projects(Projects::parse(r#"{"other": ["crates/other"]}"#).unwrap())
        .with_escalations(Arc::clone(fake) as Arc<dyn Escalations>);
    if link_t1 {
        let nano = board.roles().actor("nano", "human").unwrap();
        let card = board.add(&nano, "ship the release", "", vec![]).unwrap();
        board.bind_execution(card.id, link("T1"), &nano).unwrap();
    }
    board
}

fn remediate_cards(b: &Shared) -> Vec<Card> {
    b.list(None)
        .unwrap()
        .into_iter()
        .filter(|c| c.tags.contains(&"remediate:T1/msg:1/3".to_owned()))
        .collect()
}

const REJECT: &str = r#"{"id": "T1/msg:1/3", "rev": 7, "action": "reject"}"#;

fn reject() -> Value {
    serde_json::from_str(REJECT).unwrap()
}

#[test]
fn rejecting_acted_on_work_files_one_remediate_card() {
    let fake = Arc::new(Fake {
        consume: true,
        ..Fake::with(vec![shipped()])
    });
    let (store, down) = (
        Arc::new(MemStore::default()),
        Arc::new(AtomicBool::new(false)),
    );
    let board = board_over(&store, &down, &fake, true);

    let out = call(&board, &owner(), "inbox_act", reject()).unwrap();
    assert_eq!(out["remediated"]["outcome"], "Done", "{out}");
    let title = out["remediated"]["card"]["title"].as_str().unwrap();
    assert_eq!(title, "Remediate: ship it");
    let made = remediate_cards(&board);
    assert_eq!(made.len(), 1);
    assert!(
        made[0].body.contains("pr-7") && made[0].body.contains("claim msg:1 drop"),
        "{}",
        made[0].body
    );

    // The host closed the escalation. A retry (the answer was lost on the way back)
    // returns the same card and sends nothing to BBP again.
    let again = call(&board, &owner(), "inbox_act", reject()).unwrap();
    assert_eq!(
        again["recorded"]["card"]["id"], out["remediated"]["card"]["id"],
        "{again}"
    );
    assert_eq!(fake.acted.lock().unwrap().len(), 1);
    assert_eq!(remediate_cards(&board).len(), 1);
    // A dry run of a closed escalation is still refused.
    let dry = json!({"id": "T1/msg:1/3", "rev": 7, "action": "reject", "dry_run": true});
    assert!(refused(call(&board, &owner(), "inbox_act", dry)).contains("no open escalation"));
}

#[test]
fn approving_acted_on_work_files_nothing() {
    let fake = Arc::new(Fake::with(vec![shipped()]));
    let (store, down) = (
        Arc::new(MemStore::default()),
        Arc::new(AtomicBool::new(false)),
    );
    let board = board_over(&store, &down, &fake, true);
    let approve = json!({"id": "T1/msg:1/3", "rev": 7, "action": "approve"});
    let out = call(&board, &owner(), "inbox_act", approve).unwrap();
    assert_eq!(out, json!({"outcome": "Done"}));
    assert!(remediate_cards(&board).is_empty());
}

/// The card is written before BBP is asked: if it cannot be written, BBP is never asked.
#[test]
fn a_card_that_cannot_be_written_sends_nothing() {
    let fake = Arc::new(Fake {
        consume: true,
        ..Fake::with(vec![shipped()])
    });
    let (store, down) = (
        Arc::new(MemStore::default()),
        Arc::new(AtomicBool::new(false)),
    );
    let board = board_over(&store, &down, &fake, true);
    down.store(true, Ordering::SeqCst);
    assert!(matches!(
        call(&board, &owner(), "inbox_act", reject()),
        Err(CallError::Board(BoardError::Storage(_)))
    ));
    assert!(fake.acted.lock().unwrap().is_empty(), "BBP was asked");
    assert_eq!(
        fake.rows.lock().unwrap().len(),
        1,
        "the escalation is still open"
    );

    // The store is back after a restart: the retry files the card and drops the work.
    down.store(false, Ordering::SeqCst);
    let board = board_over(&store, &down, &fake, false);
    let out = call(&board, &owner(), "inbox_act", reject()).unwrap();
    assert_eq!(out["remediated"]["outcome"], "Done", "{out}");
    assert_eq!(fake.acted.lock().unwrap().len(), 1);
    assert_eq!(remediate_cards(&board).len(), 1);
}

/// BBP dropped the work but its answer was lost: the error names the card, and a
/// retry after a restart recovers exactly that one card without a second drop.
#[test]
fn a_lost_answer_after_the_drop_is_recovered_on_retry() {
    let fake = Arc::new(Fake {
        consume: true,
        lose_answer: AtomicBool::new(true),
        ..Fake::with(vec![shipped()])
    });
    let (store, down) = (
        Arc::new(MemStore::default()),
        Arc::new(AtomicBool::new(false)),
    );
    let board = board_over(&store, &down, &fake, true);
    let err = refused_storage(call(&board, &owner(), "inbox_act", reject()));
    assert!(
        err.contains("Remediate card") && err.contains("retry"),
        "{err}"
    );
    assert!(fake.rows.lock().unwrap().is_empty(), "the host dropped it");

    let board = board_over(&store, &down, &fake, false);
    let out = call(&board, &owner(), "inbox_act", reject()).unwrap();
    assert_eq!(
        out["recorded"]["card"]["title"], "Remediate: ship it",
        "{out}"
    );
    assert_eq!(fake.acted.lock().unwrap().len(), 1, "no second drop");
    assert_eq!(remediate_cards(&board).len(), 1);
}

fn refused_storage(r: Result<Value, CallError>) -> String {
    match r {
        Err(CallError::Board(e @ BoardError::Storage(_))) => e.to_string(),
        other => panic!("expected a storage error, got {other:?}"),
    }
}

/// BBP refused this send with the escalation untouched: the card still says only that
/// this send was refused, since a pending send of the same decision may yet commit.
#[test]
fn a_refused_send_leaves_its_card_for_the_log_to_settle() {
    let fake = Arc::new(Fake {
        refuse_at: Some(0),
        ..Fake::with(vec![shipped()])
    });
    let (store, down) = (
        Arc::new(MemStore::default()),
        Arc::new(AtomicBool::new(false)),
    );
    let board = board_over(&store, &down, &fake, true);
    let out = call(&board, &owner(), "inbox_act", reject()).unwrap();
    assert_eq!(out["remediated"]["outcome"]["Refused"]["step"], 0, "{out}");
    let card = &remediate_cards(&board)[0];
    let said = card.comments.last().map(|c| c.text.as_str()).unwrap_or("");
    assert!(said.contains("check BBP's log"), "{said}");
    assert!(!said.contains("nothing was dropped"), "{said}");
}

/// The record of a remediation is the card at its derived id, never a tag: a card
/// someone tagged `remediate:<id>` by hand, here or in another project, is no record.
#[test]
fn a_forged_remediate_tag_is_no_record() {
    let fake = Arc::new(Fake {
        consume: true,
        ..Fake::with(vec![shipped()])
    });
    let (store, down) = (
        Arc::new(MemStore::default()),
        Arc::new(AtomicBool::new(false)),
    );
    let board = board_over(&store, &down, &fake, true);
    let nano = board.roles().actor("nano", "human").unwrap();
    let tag = vec!["remediate:T1/msg:1/3".to_owned()];
    let forged = board.add(&nano, "forged here", "", tag.clone()).unwrap();
    let elsewhere = board
        .add_in(&nano, "other", "secret elsewhere", "", tag)
        .unwrap();
    let scoped = Identity {
        projects: Some(vec!["default".into()]),
        ..owner()
    };

    let out = call(&board, &scoped, "inbox_act", reject()).unwrap();
    let card = &out["remediated"]["card"];
    assert_eq!(card["title"], "Remediate: ship it", "{out}");
    assert_eq!(card["project"], "default");
    assert_ne!(card["id"], json!(forged.id));
    assert_ne!(card["id"], json!(elsewhere.id));
    assert_eq!(fake.acted.lock().unwrap().len(), 1);

    let again = call(&board, &scoped, "inbox_act", reject()).unwrap();
    assert_eq!(again["recorded"]["card"], *card, "{again}");
    assert!(!again.to_string().contains("secret elsewhere"));
}

/// A host older than #1420 does not say what was acted on: a drop fails closed.
#[test]
fn an_old_host_without_acted_on_metadata_cannot_drop() {
    let mut old = shipped();
    old.acted = None;
    let fake = Arc::new(Fake::with(vec![old]));
    let (store, down) = (
        Arc::new(MemStore::default()),
        Arc::new(AtomicBool::new(false)),
    );
    let board = board_over(&store, &down, &fake, true);
    assert!(refused(call(&board, &owner(), "inbox_act", reject())).contains("predates #1420"));
    assert!(fake.acted.lock().unwrap().is_empty());
    assert!(remediate_cards(&board).is_empty());
}

/// Approve's extends committed and its resume did not: the host now reports every
/// total funded, and a retry only resumes, without a second allowance.
#[test]
fn a_task_total_whose_resume_failed_resumes_on_retry_without_more_allowance() {
    let mut paused: Escalation = serde_json::from_value(json!({
        "id": "T1/task/9", "task": "T1", "brief": 2, "rev": 7, "seq": 9, "reason": "TaskTotal",
        "claim": null, "stakes": null, "reversible": null, "mark": null, "candidate": null,
        "acted": [], "totals": [{"field": "Messages", "spent": 41, "limit": 40, "allowance": 40}]
    }))
    .unwrap();
    let fake = Arc::new(Fake {
        refuse_at: Some(1),
        ..Fake::with(vec![paused.clone()])
    });
    let (store, down) = (
        Arc::new(MemStore::default()),
        Arc::new(AtomicBool::new(false)),
    );
    let board = board_over(&store, &down, &fake, true);
    let approve = |rev: u64| json!({"id": "T1/task/9", "rev": rev, "action": "approve"});
    let out = call(&board, &owner(), "inbox_act", approve(7)).unwrap();
    assert_eq!(out["outcome"]["Refused"]["step"], 1, "{out}");
    assert_eq!(
        *fake.planned.lock().unwrap(),
        [vec![
            "extend messages 80".to_owned(),
            "resume open".to_owned()
        ]]
    );

    // The extend committed (rev 8) and the host reports the total funded again.
    paused.rev = 8;
    paused.totals = Some(vec![]);
    let fake = Arc::new(Fake::with(vec![paused]));
    let board = board_over(&store, &down, &fake, false);
    let out = call(&board, &owner(), "inbox_act", approve(8)).unwrap();
    assert_eq!(out, json!({"outcome": "Done"}));
    assert_eq!(
        *fake.planned.lock().unwrap(),
        [vec!["resume open".to_owned()]]
    );
}

/// Each record belongs to one exact decision: Choose 5 and Choose 6 on the same stuck
/// question, even at one rev (two tabs racing), file two cards, each listing what
/// its own choice drops, and a retry of either finds its own.
#[test]
fn conflicting_choices_each_record_their_own_dropped_work() {
    let stuck: Escalation = serde_json::from_value(json!({
        "id": "T1/msg:1/3", "task": "T1", "brief": 2, "rev": 7, "seq": 3, "reason": "Stuck",
        "claim": {"id": 1, "text": "which cache?", "status": "Escalated", "score": 0,
                  "confidence": 50, "epoch": 0, "spent": 2, "budget": 8},
        "candidates": [
            {"id": 5, "text": "use an LRU", "status": "Open", "score": 4, "confidence": 70, "epoch": 0, "spent": 2, "budget": 8},
            {"id": 6, "text": "use redis", "status": "Open", "score": 4, "confidence": 70, "epoch": 0, "spent": 2, "budget": 8}
        ],
        "stakes": "Low", "reversible": true, "evidence": [], "mark": null, "candidate": null,
        "acted": [{"claim": 5, "marks": ["pr-5"]}, {"claim": 6, "marks": ["pr-6"]}],
        "totals": []
    }))
    .unwrap();
    let fake = Arc::new(Fake::with(vec![stuck]));
    let (store, down) = (
        Arc::new(MemStore::default()),
        Arc::new(AtomicBool::new(false)),
    );
    let board = board_over(&store, &down, &fake, true);
    let choose = |k: u64| json!({"id": "T1/msg:1/3", "rev": 7, "action": "choose", "chosen": k});

    let a = call(&board, &owner(), "inbox_act", choose(5)).unwrap();
    let b = call(&board, &owner(), "inbox_act", choose(6)).unwrap();
    let (a, b) = (&a["remediated"]["card"], &b["remediated"]["card"]);
    assert_ne!(a["id"], b["id"], "one record per decision");
    assert_eq!(a["title"], "Remediate: use redis");
    assert_eq!(b["title"], "Remediate: use an LRU");
    let body = |c: &Value| {
        let id: Uuid = serde_json::from_value(c["id"].clone()).unwrap();
        board.get(id).unwrap().body
    };
    assert!(
        body(a).contains("pr-6") && !body(a).contains("pr-5"),
        "{}",
        body(a)
    );
    assert!(
        body(b).contains("pr-5") && !body(b).contains("pr-6"),
        "{}",
        body(b)
    );

    let again = call(&board, &owner(), "inbox_act", choose(5)).unwrap();
    assert_eq!(
        again["remediated"]["card"]["id"], a["id"],
        "a retry finds its own"
    );
    assert_eq!(remediate_cards(&board).len(), 2);
}

/// Two sends of one decision share its card. The one that loses the fence must not
/// tell the operator to close the card the winner's drop needs.
#[test]
fn a_losing_send_does_not_close_the_winners_card() {
    let fake = Arc::new(Fake {
        consume: true,
        refuse_at: Some(0),
        ..Fake::with(vec![shipped()])
    });
    let (store, down) = (
        Arc::new(MemStore::default()),
        Arc::new(AtomicBool::new(false)),
    );
    let board = board_over(&store, &down, &fake, true);
    call(&board, &owner(), "inbox_act", reject()).unwrap();
    let card = &remediate_cards(&board)[0];
    let said = card.comments.last().map(|c| c.text.as_str()).unwrap_or("");
    assert!(said.contains("check BBP's log"), "{said}");
    assert!(!said.contains("nothing was dropped"), "{said}");
}

/// The Remediate card is a write-ahead record: on a board that keeps cards only in
/// memory a restart would lose it, so a drop of acted-on work is refused there.
#[test]
fn a_memory_only_board_refuses_a_drop_of_acted_on_work() {
    let fake = Arc::new(Fake::with(vec![shipped()]));
    let board: Shared = Board::new(
        Box::new(MemStore::default()) as Box<dyn Store>,
        Roles::default(),
    )
    .with_escalations(Arc::clone(&fake) as Arc<dyn Escalations>);
    let nano = board.roles().actor("nano", "human").unwrap();
    let card = board.add(&nano, "ship the release", "", vec![]).unwrap();
    board.bind_execution(card.id, link("T1"), &nano).unwrap();
    assert!(refused(call(&board, &owner(), "inbox_act", reject())).contains("durable store"));
    assert!(fake.acted.lock().unwrap().is_empty());
    let approve = json!({"id": "T1/msg:1/3", "rev": 7, "action": "approve"});
    assert_eq!(
        call(&board, &owner(), "inbox_act", approve).unwrap(),
        json!({"outcome": "Done"}),
        "a decision that drops nothing still works"
    );
}

fn fresh() -> (Arc<MemStore>, Arc<AtomicBool>) {
    (
        Arc::new(MemStore::default()),
        Arc::new(AtomicBool::new(false)),
    )
}

fn last_comment(c: &Card) -> &str {
    c.comments.last().map_or("", |c| c.text.as_str())
}

/// The answer to a send was lost after BBP applied it: the retry reads BBP's log, keeps the
/// card, and says at which rev.
#[test]
fn a_lost_answer_that_was_applied_keeps_its_card() {
    let fake = Arc::new(Fake {
        consume: true,
        lose_answer: AtomicBool::new(true),
        ..Fake::with(vec![shipped()])
    });
    let (store, down) = fresh();
    let board = board_over(&store, &down, &fake, true);
    refused_storage(call(&board, &owner(), "inbox_act", reject()));
    let board = board_over(&store, &down, &fake, false);
    let out = call(&board, &owner(), "inbox_act", reject()).unwrap();
    assert_eq!(
        out["recorded"]["settled"],
        json!({"applied": {"rev": 8}}),
        "{out}"
    );
    let card = &remediate_cards(&board)[0];
    assert!(card.closed.is_none(), "an applied drop keeps its card");
    assert!(last_comment(card).contains("applied (rev 8)"), "{card:?}");
    // Asking again does not repeat the note.
    call(&board, &owner(), "inbox_act", reject()).unwrap();
    assert_eq!(
        remediate_cards(&board)[0].comments.len(),
        card.comments.len()
    );
}

/// The send never reached BBP and another decision then closed the escalation: the retry
/// finds nothing in the log and no send can apply now, so the card is closed.
#[test]
fn a_lost_answer_that_was_not_applied_closes_its_card() {
    let fake = Arc::new(Fake::with(vec![shipped()]));
    let (store, down) = fresh();
    let board = board_over(&store, &down, &fake, true);
    // The card is written first, then the host cannot be reached.
    fake.refuse_next.store(true, Ordering::SeqCst);
    call(&board, &owner(), "inbox_act", reject()).unwrap();
    assert!(remediate_cards(&board)[0].closed.is_none());
    // Someone else resolved the escalation meanwhile.
    fake.rows.lock().unwrap().clear();

    let out = call(&board, &owner(), "inbox_act", reject()).unwrap();
    assert_eq!(out["recorded"]["settled"], "not_applied", "{out}");
    let card = &remediate_cards(&board)[0];
    assert!(card.closed.is_some(), "{card:?}");
    assert!(
        last_comment(card).contains("nothing was dropped"),
        "{card:?}"
    );
}

/// The same, with the escalation still open but moved past the rev the send was read at.
#[test]
fn a_retry_at_a_stale_rev_settles_the_card() {
    let fake = Arc::new(Fake::with(vec![shipped()]));
    let (store, down) = fresh();
    let board = board_over(&store, &down, &fake, true);
    fake.refuse_next.store(true, Ordering::SeqCst);
    call(&board, &owner(), "inbox_act", reject()).unwrap();
    fake.rows.lock().unwrap()[0].rev = 9;

    let out = call(&board, &owner(), "inbox_act", reject()).unwrap();
    assert_eq!(out["recorded"]["settled"], "not_applied", "{out}");
    assert!(remediate_cards(&board)[0].closed.is_some());
}

/// A send that loses the fence to another decision is settled at once: its card is closed.
/// Losing to a send of the same decision keeps the card, since that send applied it.
#[test]
fn a_losing_send_settles_its_card_from_the_log() {
    let lose = |same: bool| {
        let fake = Arc::new(Fake {
            consume: true,
            refuse_at: Some(0),
            refuse_code: Some("StaleRev"),
            winner_is_same_decision: same,
            ..Fake::with(vec![shipped()])
        });
        let (store, down) = fresh();
        let board = board_over(&store, &down, &fake, true);
        call(&board, &owner(), "inbox_act", reject()).unwrap();
        remediate_cards(&board).remove(0)
    };
    let other = lose(false);
    assert!(other.closed.is_some(), "{other:?}");
    let same = lose(true);
    assert!(same.closed.is_none(), "{same:?}");
    assert!(last_comment(&same).contains("applied"), "{same:?}");
}

/// A host that predates `bbp decision` cannot settle anything: the card stays, as before.
#[test]
fn an_old_host_leaves_the_card_for_a_person() {
    let fake = Arc::new(Fake {
        consume: true,
        lose_answer: AtomicBool::new(true),
        old_host: AtomicBool::new(true),
        ..Fake::with(vec![shipped()])
    });
    let (store, down) = fresh();
    let board = board_over(&store, &down, &fake, true);
    refused_storage(call(&board, &owner(), "inbox_act", reject()));
    let board = board_over(&store, &down, &fake, false);
    let out = call(&board, &owner(), "inbox_act", reject()).unwrap();
    assert_eq!(out["recorded"]["settled"], "unknown", "{out}");
    assert!(remediate_cards(&board)[0].closed.is_none());
}

/// The id a drop is sent under is the Remediate card's id, so the card and BBP's log meet.
#[test]
fn a_drop_is_sent_under_its_cards_id() {
    let fake = Arc::new(Fake::with(vec![shipped()]));
    let (store, down) = fresh();
    let board = board_over(&store, &down, &fake, true);
    let out = call(&board, &owner(), "inbox_act", reject()).unwrap();
    let card = out["remediated"]["card"]["id"].as_str().unwrap().to_owned();
    assert_eq!(*fake.decided.lock().unwrap(), [card]);
}

/// A Remediate card filed before BBP recorded decision ids, at the id the old board gave it.
/// Its send carried no id, so BBP's log cannot say whether it was applied.
fn legacy_card(store: &MemStore) -> Card {
    const NS: Uuid = Uuid::from_u128(0x6d69_6c6c_2d72_656d_6564_6961_7465_0595);
    let action = serde_json::to_string(&Action::Reject { reason: None }).unwrap();
    let key = format!("default\nT1/msg:1/3\n7\n{action}");
    let card = Card {
        id: Uuid::new_v5(&NS, key.as_bytes()),
        ..Card::new(
            "Remediate: ship it",
            "filed before the send",
            vec!["remediate:T1/msg:1/3".to_owned()],
            1,
        )
        .unwrap()
    };
    let event = Event::of(&card, rusty_mill_board::domain::EventKind::Created, "nano");
    store.insert(&card, event).unwrap();
    card
}

/// After an upgrade, a retry of a decision the old board sent finds no id in BBP's log. That
/// proves nothing: the old send may have applied. The card stays for a person.
#[test]
fn a_card_from_before_decision_ids_is_never_closed_on_absence() {
    let fake = Arc::new(Fake::with(vec![]));
    let (store, down) = fresh();
    let board = board_over(&store, &down, &fake, true);
    let legacy = legacy_card(&store);
    // The old send applied and the escalation is gone; BBP holds no id for it.
    let out = call(&board, &owner(), "inbox_act", reject()).unwrap();
    assert_eq!(out["recorded"]["settled"], "unknown", "{out}");
    assert_eq!(out["recorded"]["card"]["id"], legacy.id.to_string());
    assert!(board.get(legacy.id).unwrap().closed.is_none());
    assert_eq!(remediate_cards(&board).len(), 1, "no second card");
    assert!(
        fake.acted.lock().unwrap().is_empty(),
        "nothing is sent again"
    );
}

/// The escalation is still open at the same rev, so the retry is sent again: as the old
/// board sent it, under no id, onto the card it already has. Refused or not, that card stays.
#[test]
fn a_retry_of_an_old_send_reuses_its_card_and_sends_no_id() {
    let fake = Arc::new(Fake {
        refuse_at: Some(0),
        refuse_code: Some("StaleRev"),
        ..Fake::with(vec![shipped()])
    });
    let (store, down) = fresh();
    let board = board_over(&store, &down, &fake, true);
    let legacy = legacy_card(&store);
    let out = call(&board, &owner(), "inbox_act", reject()).unwrap();
    assert_eq!(
        out["remediated"]["card"]["id"],
        legacy.id.to_string(),
        "{out}"
    );
    assert!(fake.decided.lock().unwrap().is_empty(), "no id was sent");
    let card = board.get(legacy.id).unwrap();
    assert!(card.closed.is_none(), "{card:?}");
    assert!(last_comment(&card).contains("check BBP's log"), "{card:?}");
    assert_eq!(remediate_cards(&board).len(), 1, "no second card");
}

/// A host that predates `bbp decision` ignores `--decision`, so a drop sent to it is
/// untracked. If the host is upgraded afterwards, the log's silence about that send must
/// not close its card: the card was filed at the legacy id, which is never settled.
#[test]
fn a_drop_sent_to_a_host_that_cannot_track_it_is_never_closed_after_an_upgrade() {
    let fake = Arc::new(Fake {
        consume: true,
        lose_answer: AtomicBool::new(true),
        old_host: AtomicBool::new(true),
        ..Fake::with(vec![shipped()])
    });
    let (store, down) = fresh();
    let board = board_over(&store, &down, &fake, true);
    refused_storage(call(&board, &owner(), "inbox_act", reject()));
    assert!(fake.decided.lock().unwrap().is_empty(), "no id was sent");

    fake.old_host.store(false, Ordering::SeqCst); // the host is upgraded
    let out = call(&board, &owner(), "inbox_act", reject()).unwrap();
    assert_eq!(out["recorded"]["settled"], "unknown", "{out}");
    assert!(remediate_cards(&board)[0].closed.is_none());
    assert_eq!(remediate_cards(&board).len(), 1);
}

/// Two retries of one decision probe the host with different outcomes, so one files the
/// tracked card and the other the legacy one. The untracked send may have won the fence:
/// the tracked card's retry must not conclude that nothing was dropped, whichever order
/// the two cards were filed in, and neither card is closed.
#[test]
fn concurrent_retries_with_different_probes_never_close_either_card() {
    let fake = Arc::new(Fake::with(vec![shipped()]));
    let (store, down) = fresh();
    let board = board_over(&store, &down, &fake, true);
    // Retry A: the probe passes, its card is filed, and the host is then unreachable.
    fake.refuse_next.store(true, Ordering::SeqCst);
    call(&board, &owner(), "inbox_act", reject()).unwrap();
    assert_eq!(remediate_cards(&board).len(), 1);
    // Retry B: its probe failed, so it filed the legacy card and sent untracked, and won.
    let legacy = legacy_card(&store);
    fake.rows.lock().unwrap().clear();

    let out = call(&board, &owner(), "inbox_act", reject()).unwrap();
    assert_eq!(out["recorded"]["settled"], "unknown", "{out}");
    assert_eq!(
        out["recorded"]["card"]["id"],
        legacy.id.to_string(),
        "{out}"
    );
    assert!(remediate_cards(&board).iter().all(|c| c.closed.is_none()));
}

/// The other order: the legacy card exists first, so a retry whose probe passes adds no
/// second card and sends no id.
#[test]
fn a_passing_probe_after_a_legacy_card_files_nothing_and_sends_no_id() {
    let fake = Arc::new(Fake::with(vec![shipped()]));
    let (store, down) = fresh();
    let board = board_over(&store, &down, &fake, true);
    legacy_card(&store);
    call(&board, &owner(), "inbox_act", reject()).unwrap();
    assert_eq!(remediate_cards(&board).len(), 1);
    assert!(fake.decided.lock().unwrap().is_empty());
}

/// A tracked card exists but the host has since been rolled back: the retry's probe fails,
/// so the legacy card is filed before the send goes untracked. After the host is upgraded
/// again, the log's silence about that send must not close either card.
#[test]
fn a_host_rolled_back_after_a_tracked_card_never_closes_it() {
    let fake = Arc::new(Fake {
        consume: true,
        lose_answer: AtomicBool::new(true),
        ..Fake::with(vec![shipped()])
    });
    let (store, down) = fresh();
    let board = board_over(&store, &down, &fake, true);
    fake.refuse_next.store(true, Ordering::SeqCst); // the first try never reaches BBP
    call(&board, &owner(), "inbox_act", reject()).unwrap();
    assert_eq!(remediate_cards(&board).len(), 1);

    fake.fail_before.store(false, Ordering::SeqCst);
    fake.old_host.store(true, Ordering::SeqCst); // rolled back: cannot record ids
    refused_storage(call(&board, &owner(), "inbox_act", reject()));
    assert_eq!(
        remediate_cards(&board).len(),
        2,
        "the legacy card was filed first"
    );
    assert!(
        fake.decided.lock().unwrap().is_empty(),
        "the send carried no id"
    );

    fake.old_host.store(false, Ordering::SeqCst); // upgraded again
    let out = call(&board, &owner(), "inbox_act", reject()).unwrap();
    assert_eq!(out["recorded"]["settled"], "unknown", "{out}");
    assert!(remediate_cards(&board).iter().all(|c| c.closed.is_none()));
}

/// The uncertainty is written before the send: a send whose answer never came back (a lost
/// answer, a crash, an unreachable host) leaves its mark on the card, so no retry, however it
/// overlaps, settles the card as "not applied" from the log's silence.
#[test]
fn a_send_with_no_answer_is_never_settled_as_not_applied() {
    for lose in [false, true] {
        let fake = Arc::new(Fake {
            consume: true,
            ignore_id: AtomicBool::new(true),
            lose_answer: AtomicBool::new(lose),
            ..Fake::with(vec![shipped()])
        });
        fake.fail_before.store(!lose, Ordering::SeqCst);
        let (store, down) = fresh();
        let board = board_over(&store, &down, &fake, true);
        refused_storage(call(&board, &owner(), "inbox_act", reject()));
        fake.fail_before.store(false, Ordering::SeqCst);
        fake.lose_answer.store(false, Ordering::SeqCst);
        // Someone else resolved the escalation, and the host never recorded the id.
        fake.rows.lock().unwrap().clear();

        let out = call(&board, &owner(), "inbox_act", reject()).unwrap();
        assert_eq!(out["recorded"]["settled"], "unknown", "{out}");
        assert!(remediate_cards(&board).iter().all(|c| c.closed.is_none()));
    }
}

/// The host is rolled back between the capability probe and the send, so it applies the send
/// without recording its id. The board notices the id is missing after the commit and files the
/// legacy card, so a later retry is never settled as "not applied".
#[test]
fn a_send_whose_id_never_reached_the_log_is_marked_untracked() {
    let fake = Arc::new(Fake {
        consume: true,
        ignore_id: AtomicBool::new(true),
        ..Fake::with(vec![shipped()])
    });
    let (store, down) = fresh();
    let board = board_over(&store, &down, &fake, true);
    call(&board, &owner(), "inbox_act", reject()).unwrap();
    assert_eq!(remediate_cards(&board).len(), 2, "tracked and legacy cards");

    let out = call(&board, &owner(), "inbox_act", reject()).unwrap();
    assert_eq!(out["recorded"]["settled"], "unknown", "{out}");
    assert!(remediate_cards(&board).iter().all(|c| c.closed.is_none()));
}

/// The board's inbox can hide an escalation BBP still lists (here a host that answers a lone
/// task and not a list). Gone from the board's view is not gone from BBP, so nothing settles.
#[test]
fn an_escalation_bbp_still_lists_is_not_gone() {
    let fake = Arc::new(Fake {
        list_only_alone: AtomicBool::new(true),
        ..Fake::with(vec![shipped()])
    });
    let (store, down) = fresh();
    let board = board_over(&store, &down, &fake, true);
    let nano = board.roles().actor("nano", "human").unwrap();
    let other = board.add(&nano, "another task", "", vec![]).unwrap();
    board.bind_execution(other.id, link("T2"), &nano).unwrap();
    fake.fail_before.store(true, Ordering::SeqCst);
    // Sent while it was visible to the board? No: the board never lists it now, but a card
    // for the decision exists from an earlier send.
    fake.list_only_alone.store(false, Ordering::SeqCst);
    refused_storage(call(&board, &owner(), "inbox_act", reject()));
    fake.fail_before.store(false, Ordering::SeqCst);
    fake.list_only_alone.store(true, Ordering::SeqCst);

    let out = call(&board, &owner(), "inbox_act", reject()).unwrap();
    assert_eq!(out["recorded"]["settled"], "unknown", "{out}");
    assert!(remediate_cards(&board)[0].closed.is_none());
}
