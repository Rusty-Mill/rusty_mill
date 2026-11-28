//! Registered agents in Settings: the stale marking matches what revoke-stale revokes (#1643),
//! and revoking an agent releases its card claims (#1644), with faults injected into the card
//! store and the registry file to show a failed release is retried without a restart.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use rusty_mill_board::agents::{Grant, MAX_PER_CONNECTOR, Registry};
use rusty_mill_board::api::{CallError, Identity, Shared};
use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::domain::{BoardError, Capability, Card, CommentKind, Event};
use rusty_mill_board::projects::Projects;
use rusty_mill_board::settings::{Files, Settings};
use rusty_mill_board::store::{MemStore, Store};
use rusty_mill_board::syncer::Syncer;
use serde_json::{Value, json};
use uuid::Uuid;

const H: i64 = 3600 * 1000;

thread_local! {
    /// The board's clock: each test runs on its own thread, so each has its own time.
    static NOW: Cell<i64> = const { Cell::new(0) };
}

fn clock() -> i64 {
    NOW.with(Cell::get)
}

fn at(ms: i64) {
    NOW.with(|n| n.set(ms));
}

/// Faults to inject into [`Faulty`].
#[derive(Default)]
struct Faults {
    /// Fail this many of the next card listings.
    lists: AtomicUsize,
    /// Fail every write to this card.
    card: Mutex<Option<Uuid>>,
}

/// A [`MemStore`] that fails on demand.
struct Faulty {
    inner: MemStore,
    faults: Arc<Faults>,
}

fn injected() -> BoardError {
    BoardError::Storage("injected fault".into())
}

impl Store for Faulty {
    fn get(&self, id: Uuid) -> Result<Option<Card>, BoardError> {
        self.inner.get(id)
    }
    fn list(&self) -> Result<Vec<Card>, BoardError> {
        let armed = self
            .faults
            .lists
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok();
        if armed {
            Err(injected())
        } else {
            self.inner.list()
        }
    }
    fn insert(&self, card: &Card, event: Event) -> Result<u64, BoardError> {
        self.inner.insert(card, event)
    }
    fn replace(&self, card: &Card, event: Event) -> Result<u64, BoardError> {
        if *self.faults.card.lock().expect("faults") == Some(card.id) {
            return Err(injected());
        }
        self.inner.replace(card, event)
    }
    fn events_since(&self, after: Option<Uuid>, limit: usize) -> Result<Vec<Event>, BoardError> {
        self.inner.events_since(after, limit)
    }
    fn last_event_id(&self) -> Result<Option<Uuid>, BoardError> {
        self.inner.last_event_id()
    }
    fn holds_event(&self, id: Uuid) -> Result<bool, BoardError> {
        self.inner.holds_event(id)
    }
    fn reserve_number(&self, card: Uuid, floor: u64) -> Result<u64, BoardError> {
        self.inner.reserve_number(card, floor)
    }
    fn highest_reserved(&self) -> Result<u64, BoardError> {
        self.inner.highest_reserved()
    }
    fn keeps_merge_record(&self) -> bool {
        self.inner.keeps_merge_record()
    }
}

struct Rig {
    board: Arc<Shared>,
    settings: Settings,
    faults: Arc<Faults>,
    dir: std::path::PathBuf,
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A board whose admin, `keeper`, holds `Admin` only: no `Triage`, so no release of its own.
fn rig(name: &str) -> Rig {
    at(0);
    let dir = std::env::temp_dir().join(format!("mill_regagents_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    let mut roles = Roles::default();
    roles
        .0
        .insert("keeper".into(), [Capability::Admin].into_iter().collect());
    let faults = Arc::new(Faults::default());
    let store = Faulty {
        inner: MemStore::default(),
        faults: Arc::clone(&faults),
    };
    let registry = Registry::open(dir.join("agents.json"), 0).expect("registry");
    let board: Arc<Shared> = Arc::new(
        Board::with_clock(Box::new(store) as Box<dyn Store>, roles, clock)
            .with_projects(Projects::default())
            .with_agents(registry),
    );
    let settings = Settings::new(
        board.clone(),
        Files::in_dir(&dir),
        None,
        Syncer::new(board.clone()),
        None,
    );
    Rig {
        board,
        settings,
        faults,
        dir,
    }
}

impl Rig {
    fn admin(&self, op: &str, args: Value) -> Value {
        self.try_admin(op, args)
            .unwrap_or_else(|e| panic!("{op}: {e:?}"))
    }

    fn try_admin(&self, op: &str, args: Value) -> Result<Value, CallError> {
        let keeper = Identity {
            actor: "keeper".into(),
            role: "keeper".into(),
            projects: None,
            connector: false,
            provider: None,
            view: Default::default(),
        };
        self.settings
            .call(op, Some(&keeper), args)
            .expect("a settings operation")
    }

    /// Revoke-stale for real with nothing stale: the retry path for orphaned claims.
    fn retry(&self) -> Value {
        self.admin(
            "settings_registered_agent_revoke_stale",
            json!({"revoke": true, "ids": []}),
        )
    }

    fn register(&self) -> (String, String) {
        self.board
            .register_agent("conn", Grant::default())
            .expect("register")
    }

    fn use_agent(&self, (id, key): &(String, String)) {
        assert!(self.board.verify_agent("conn", id, key).is_some());
    }

    /// A card claimed by `holder`.
    fn claimed(&self, title: &str, holder: &str) -> Uuid {
        let nano = self.board.roles().actor("nano", "human").expect("nano");
        let card = self.board.add(&nano, title, "", vec![]).expect("add");
        let agent = self.board.roles().actor(holder, "builder").expect("agent");
        self.board.claim(card.id, &agent).expect("claim");
        card.id
    }

    fn holder(&self, card: Uuid) -> Option<String> {
        self.board.get(card).expect("card").claim
    }
}

fn ids(v: &Value) -> Vec<String> {
    serde_json::from_value(v.clone()).expect("ids")
}

fn claimed_cards(v: &Value) -> BTreeMap<String, String> {
    v.as_array()
        .expect("claims")
        .iter()
        .map(|c| {
            (
                c["card"].as_str().expect("card").to_owned(),
                c["holder"].as_str().expect("holder").to_owned(),
            )
        })
        .collect()
}

#[test]
fn the_view_marks_as_stale_exactly_what_revoke_stale_revokes() {
    let r = rig("view");
    let idle = r.register();
    let edge = r.register();
    let busy = r.register();
    at(2 * H);
    r.use_agent(&edge);
    at(25 * H);
    r.use_agent(&busy);
    at(26 * H + 1);
    let view = r.admin("settings_get", json!({}));
    assert_eq!(view["registered_agent_slots"], json!(MAX_PER_CONNECTOR));
    assert_eq!(view["registered_agent_stale_hours"], json!(24));
    let mut stale = vec![idle.0.clone(), edge.0.clone()];
    stale.sort();
    assert_eq!(ids(&view["registered_agents_stale"]), stale);
    let preview = r.admin("settings_registered_agent_revoke_stale", json!({}));
    assert_eq!(
        ids(&preview["ids"]),
        stale,
        "the button previews what the view marks"
    );
    let done = r.admin(
        "settings_registered_agent_revoke_stale",
        json!({"revoke": true, "ids": preview["ids"]}),
    );
    assert_eq!(ids(&done["ids"]), stale, "and revokes exactly the preview");
    let left: Vec<String> = r.board.agent_list().into_iter().map(|a| a.id).collect();
    assert_eq!(left, vec![busy.0.clone()]);
    assert!(ids(&r.admin("settings_get", json!({}))["registered_agents_stale"]).is_empty());
    // A revoked agent's key is dead, and it can register again under a new id.
    assert!(r.board.verify_agent("conn", &idle.0, &idle.1).is_none());
    let again = r.register();
    assert_ne!(again.0, idle.0);
}

#[test]
fn revoking_an_agent_releases_its_claims_with_a_handoff_and_leaves_others() {
    let r = rig("single");
    let gone = r.register();
    let other = r.register();
    let a = r.claimed("a", &gone.0);
    let b = r.claimed("b", &gone.0);
    let c = r.claimed("c", &other.0);
    let out = r.admin("settings_registered_agent_revoke", json!({"id": gone.0}));
    assert_eq!(out["revoked"], json!(gone.0));
    let released = claimed_cards(&out["released"]);
    assert_eq!(
        released,
        BTreeMap::from([
            (a.to_string(), gone.0.clone()),
            (b.to_string(), gone.0.clone())
        ])
    );
    assert_eq!(out["not_released"], json!([]));
    for card in [a, b] {
        assert_eq!(r.holder(card), None);
        let last = r
            .board
            .get(card)
            .expect("card")
            .comments
            .pop()
            .expect("comment");
        assert_eq!(
            (last.by.as_str(), last.kind),
            ("keeper", CommentKind::Handoff)
        );
        assert!(
            last.text.contains(&gone.0) && last.text.contains("revoked"),
            "{}",
            last.text
        );
    }
    assert_eq!(
        r.holder(c),
        Some(other.0.clone()),
        "another agent's claim is kept"
    );
    // An agent holding nothing: revoked, nothing released.
    let idle = r.register();
    let out = r.admin("settings_registered_agent_revoke", json!({"id": idle.0}));
    assert_eq!(out["released"], json!([]));
    // Registering again never restores a claim.
    r.register();
    assert_eq!((r.holder(a), r.holder(b)), (None, None));
}

#[test]
fn the_stale_dry_run_lists_the_claims_the_real_run_releases() {
    let r = rig("stale");
    let old = r.register();
    let fresh = r.register();
    let a = r.claimed("a", &old.0);
    let b = r.claimed("b", &fresh.0);
    at(25 * H);
    r.use_agent(&fresh);
    at(25 * H + 2);
    let dry = r.admin("settings_registered_agent_revoke_stale", json!({}));
    assert_eq!(ids(&dry["ids"]), vec![old.0.clone()]);
    let expected = BTreeMap::from([(a.to_string(), old.0.clone())]);
    assert_eq!(claimed_cards(&dry["claims"]), expected);
    assert_eq!(
        r.holder(a),
        Some(old.0.clone()),
        "a dry run releases nothing"
    );
    let real = r.admin(
        "settings_registered_agent_revoke_stale",
        json!({"revoke": true, "ids": dry["ids"]}),
    );
    assert_eq!(
        claimed_cards(&real["claims"]),
        expected,
        "the real run matches the dry run"
    );
    assert_eq!(real["not_released"], json!([]));
    assert_eq!((r.holder(a), r.holder(b)), (None, Some(fresh.0.clone())));
}

#[test]
fn a_failed_scan_after_a_revoke_is_retried_without_a_restart_or_re_registering() {
    let r = rig("scan");
    let gone = r.register();
    let live = r.register();
    let a = r.claimed("a", &gone.0);
    let b = r.claimed("b", &live.0);
    let c = r.claimed("c", "claude");
    r.faults.lists.store(1, Ordering::SeqCst);
    let e = r
        .try_admin("settings_registered_agent_revoke", json!({"id": gone.0}))
        .expect_err("the scan fails");
    assert!(format!("{e:?}").contains("injected"), "{e:?}");
    assert!(
        r.board.verify_agent("conn", &gone.0, &gone.1).is_none(),
        "the revoke itself stands"
    );
    assert_eq!(
        r.holder(a),
        Some(gone.0.clone()),
        "its claim is orphaned for now"
    );
    // The dry run already shows the orphan, and a real run with nothing stale releases it.
    let dry = r.admin("settings_registered_agent_revoke_stale", json!({}));
    assert_eq!(
        claimed_cards(&dry["claims"]),
        BTreeMap::from([(a.to_string(), gone.0.clone())])
    );
    let out = r.retry();
    assert_eq!(ids(&out["ids"]), Vec::<String>::new());
    assert_eq!(
        claimed_cards(&out["claims"]),
        BTreeMap::from([(a.to_string(), gone.0.clone())])
    );
    assert_eq!(r.holder(a), None);
    assert_eq!(
        (r.holder(b), r.holder(c)),
        (Some(live.0.clone()), Some("claude".to_owned())),
        "a live agent's and a bearer holder's claims are never released"
    );
    assert!(
        r.retry()["claims"].as_array().expect("claims").is_empty(),
        "nothing twice"
    );
}

#[test]
fn a_card_that_fails_to_release_is_reported_and_released_by_the_next_run() {
    let r = rig("card");
    let gone = r.register();
    let a = r.claimed("a", &gone.0);
    let b = r.claimed("b", &gone.0);
    *r.faults.card.lock().expect("faults") = Some(b);
    let out = r.admin("settings_registered_agent_revoke", json!({"id": gone.0}));
    assert_eq!(
        claimed_cards(&out["released"]),
        BTreeMap::from([(a.to_string(), gone.0.clone())])
    );
    assert_eq!(
        claimed_cards(&out["not_released"]),
        BTreeMap::from([(b.to_string(), gone.0.clone())])
    );
    assert!(
        out["not_released"][0]["reason"]
            .as_str()
            .expect("reason")
            .contains("injected")
    );
    *r.faults.card.lock().expect("faults") = None;
    let again = r.retry();
    assert_eq!(
        claimed_cards(&again["claims"]),
        BTreeMap::from([(b.to_string(), gone.0.clone())])
    );
    assert_eq!((r.holder(a), r.holder(b)), (None, None));
}

#[test]
fn a_failed_registry_write_still_releases_the_claims_and_a_retry_makes_it_durable() {
    let r = rig("write");
    let old = r.register();
    let fresh = r.register();
    let a = r.claimed("a", &old.0);
    let b = r.claimed("b", &fresh.0);
    at(25 * H);
    r.use_agent(&fresh);
    at(25 * H + 2);
    // The registry file cannot be written: its directory is gone.
    std::fs::remove_dir_all(&r.dir).expect("remove dir");
    let e = r
        .try_admin(
            "settings_registered_agent_revoke_stale",
            json!({"revoke": true}),
        )
        .expect_err("the write fails");
    assert!(format!("{e:?}").contains("run it again"), "{e:?}");
    assert_eq!(
        r.holder(a),
        None,
        "revoked in memory, so its claim is released"
    );
    assert_eq!(r.holder(b), Some(fresh.0.clone()));
    std::fs::create_dir_all(&r.dir).expect("restore dir");
    let out = r.retry();
    assert_eq!(
        out["unsaved"],
        json!(false),
        "the retry wrote the revocation"
    );
    let reopened = Registry::open(r.dir.join("agents.json"), 25 * H + 3).expect("reopen");
    assert!(!reopened.contains(&old.0) && reopened.contains(&fresh.0));
}
