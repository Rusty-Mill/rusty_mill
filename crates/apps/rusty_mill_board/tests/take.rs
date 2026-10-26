//! `Board::take`: pulling the next card for a role.

use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::domain::{Actor, BoardError, Column};
use rusty_mill_board::store::MemStore;

fn board() -> Board<MemStore> {
    Board::new(MemStore::default(), Roles::default())
}

fn who(b: &Board<MemStore>, name: &str, role: &str) -> Actor {
    b.roles().actor(name, role).expect("role")
}

/// A card put in Ready by a person.
fn ready(b: &Board<MemStore>, title: &str) -> uuid::Uuid {
    let h = who(b, "nano", "human");
    let c = b.add(&h, title, "", vec![]).expect("add");
    b.move_card(c.id, Column::Ready, &h).expect("ready");
    c.id
}

#[test]
fn take_enters_a_ready_card_and_claims_it() {
    let b = board();
    let id = ready(&b, "a");
    let dev = who(&b, "claude", "builder");
    let card = b.take(&dev, None).expect("take").expect("a card");
    assert_eq!(
        (card.id, card.column, card.claim.as_deref()),
        (id, Column::Building, Some("claude"))
    );
    assert!(
        b.take(&dev, None)
            .expect("take")
            .is_some_and(|c| c.id == id),
        "own claim is resumed"
    );
}

#[test]
fn take_is_role_aware_and_none_when_nothing_waits() {
    let b = board();
    ready(&b, "a");
    let qa = who(&b, "gemini", "qa");
    assert!(
        b.take(&qa, None).expect("take").is_none(),
        "a qa role has no work in Ready"
    );
    let rev = who(&b, "codex", "reviewer");
    assert!(b.take(&rev, None).expect("take").is_none());
}

#[test]
fn take_prefers_work_nearest_done_then_oldest() {
    let b = board();
    let first = ready(&b, "first");
    let second = ready(&b, "second");
    let dev = who(&b, "claude", "builder");
    // Move `second` on to Qa by hand: the qa role must see it, the builder takes `first`.
    let t = b.take(&dev, Some(second)).expect("take").expect("card");
    b.move_card(t.id, Column::Qa, &dev).expect("to qa");
    let got = b.take(&dev, None).expect("take").expect("card");
    assert_eq!(got.id, first, "builder has nothing in Qa");
    let qa = who(&b, "gemini", "qa");
    assert_eq!(b.take(&qa, None).expect("take").expect("card").id, second);
}

#[test]
fn two_workers_never_take_the_same_card() {
    let b = board();
    ready(&b, "only");
    let (a, c) = (who(&b, "claude", "builder"), who(&b, "codex", "builder"));
    let got = b.take(&a, None).expect("take").expect("card");
    assert!(
        b.take(&c, None).expect("take").is_none(),
        "the claimed card is skipped: {got:?}"
    );
}

#[test]
fn take_by_id_reports_why_it_cannot() {
    let b = board();
    let id = ready(&b, "a");
    let (a, c) = (who(&b, "claude", "builder"), who(&b, "codex", "builder"));
    b.take(&a, Some(id)).expect("take").expect("card");
    assert_eq!(
        b.take(&c, Some(id)),
        Err(BoardError::Claimed("claude".into()))
    );
    assert_eq!(
        b.take(&a, Some(uuid::Uuid::nil())),
        Err(BoardError::NotFound(uuid::Uuid::nil()))
    );
}

#[test]
fn a_claimed_ready_card_is_not_half_taken() {
    let b = board();
    let id = ready(&b, "a");
    let h = who(&b, "nano", "human");
    b.claim(id, &h).expect("a person parked a claim on it");
    let dev = who(&b, "claude", "builder");
    assert_eq!(
        b.take(&dev, Some(id)),
        Err(BoardError::Claimed("nano".into()))
    );
    assert_eq!(b.get(id).expect("card").column, Column::Ready, "not moved");
}

// ---- concurrency: the claim is the lock, so it must hold under a real race

use std::collections::HashSet;
use std::sync::{Arc, Barrier};
use std::thread;

use rusty_mill_board::mmdb::MmdbStore;
use rusty_mill_board::store::Store;

const WORKERS: usize = 12;

fn racing_workers_never_share_a_card<S: Store + 'static>(store: S, cards: usize) {
    let b = Arc::new(Board::new(store, Roles::default()));
    let human = b.roles().actor("nano", "human").expect("role");
    let ids: HashSet<_> = (0..cards)
        .map(|i| {
            let c = b
                .add(&human, &format!("card {i}"), "", vec![])
                .expect("add");
            b.move_card(c.id, Column::Ready, &human).expect("ready");
            c.id
        })
        .collect();

    let start = Arc::new(Barrier::new(WORKERS));
    let handles: Vec<_> = (0..WORKERS)
        .map(|w| {
            let (b, start) = (Arc::clone(&b), Arc::clone(&start));
            thread::spawn(move || {
                let me = b
                    .roles()
                    .actor(&format!("worker-{w}"), "builder")
                    .expect("role");
                start.wait();
                // Each worker keeps taking until nothing is left for it.
                let mut mine = Vec::new();
                while let Some(card) = b.take(&me, None).expect("take") {
                    mine.push(card.id);
                    // Hand it on so the same worker does not just resume its own claim.
                    b.move_card(card.id, Column::Qa, &me).expect("to qa");
                }
                mine
            })
        })
        .collect();

    let taken: Vec<_> = handles
        .into_iter()
        .flat_map(|h| h.join().expect("worker"))
        .collect();
    let unique: HashSet<_> = taken.iter().copied().collect();
    assert_eq!(taken.len(), unique.len(), "a card was taken by two workers");
    assert_eq!(unique, ids, "every card was taken exactly once");
    for id in ids {
        let card = b.get(id).expect("card");
        assert_eq!(card.column, Column::Qa);
        assert_eq!(card.claim, None, "moving on cleared the claim");
    }
}

#[test]
fn racing_workers_never_share_a_card_in_memory() {
    racing_workers_never_share_a_card(MemStore::default(), 60);
}

#[test]
fn racing_workers_never_share_a_card_on_disk() {
    let dir = std::env::temp_dir().join(format!("mill_take_race_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    racing_workers_never_share_a_card(MmdbStore::open(&dir.join("b.mmap")).expect("open"), 30);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn many_workers_claiming_one_card_produce_exactly_one_winner() {
    let b = Arc::new(Board::new(MemStore::default(), Roles::default()));
    let id = ready(&b, "contested");
    let start = Arc::new(Barrier::new(WORKERS));
    let handles: Vec<_> = (0..WORKERS)
        .map(|w| {
            let (b, start) = (Arc::clone(&b), Arc::clone(&start));
            thread::spawn(move || {
                let me = b
                    .roles()
                    .actor(&format!("worker-{w}"), "builder")
                    .expect("role");
                start.wait();
                b.take(&me, Some(id))
                    .map(|c| c.map(|c| c.claim))
                    .ok()
                    .flatten()
                    .flatten()
            })
        })
        .collect();
    let winners: Vec<_> = handles
        .into_iter()
        .filter_map(|h| h.join().expect("worker"))
        .collect();
    assert_eq!(winners.len(), 1, "exactly one claim wins: {winners:?}");
    let card = b.get(id).expect("card");
    assert_eq!(
        (card.column, card.claim),
        (Column::Building, Some(winners[0].clone()))
    );
}

/// A store that, right after the first `get` once armed, lets a rival claim the card:
/// the race between `take`'s read of a Ready card and its move.
struct RivalAfterGet {
    inner: MemStore,
    rival: Actor,
    armed: Arc<std::sync::atomic::AtomicBool>,
}

impl Store for RivalAfterGet {
    fn get(&self, id: uuid::Uuid) -> Result<Option<rusty_mill_board::domain::Card>, BoardError> {
        let seen = self.inner.get(id)?;
        if self.armed.swap(false, std::sync::atomic::Ordering::SeqCst) {
            let claimed = seen
                .as_ref()
                .expect("card")
                .claim(&self.rival, 1)
                .expect("rival claim");
            let event = rusty_mill_board::domain::Event::of(
                &claimed,
                rusty_mill_board::domain::EventKind::Claimed,
                &self.rival.name,
            );
            self.inner.replace(&claimed, event)?;
        }
        Ok(seen)
    }
    fn list(&self) -> Result<Vec<rusty_mill_board::domain::Card>, BoardError> {
        self.inner.list()
    }
    fn insert(
        &self,
        c: &rusty_mill_board::domain::Card,
        e: rusty_mill_board::domain::Event,
    ) -> Result<u64, BoardError> {
        self.inner.insert(c, e)
    }
    fn replace(
        &self,
        c: &rusty_mill_board::domain::Card,
        e: rusty_mill_board::domain::Event,
    ) -> Result<u64, BoardError> {
        self.inner.replace(c, e)
    }
    fn events_since(
        &self,
        a: Option<uuid::Uuid>,
        l: usize,
    ) -> Result<Vec<rusty_mill_board::domain::Event>, BoardError> {
        self.inner.events_since(a, l)
    }
    fn last_event_id(&self) -> Result<Option<uuid::Uuid>, BoardError> {
        self.inner.last_event_id()
    }
}

#[test]
fn a_rival_claim_landing_after_the_read_stops_the_move() {
    let roles = Roles::default();
    let rival = roles.actor("codex", "builder").expect("role");
    let armed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let store = RivalAfterGet {
        inner: MemStore::default(),
        rival,
        armed: Arc::clone(&armed),
    };
    let b = Board::new(store, roles);
    let h = who_in(&b, "nano", "human");
    let c = b.add(&h, "a", "", vec![]).expect("add");
    b.move_card(c.id, Column::Ready, &h).expect("ready");
    let dev = who_in(&b, "claude", "builder");
    // `take` by id: the first get inside it (the "current" read) triggers the rival.
    armed.store(true, std::sync::atomic::Ordering::SeqCst);
    let err = b.take(&dev, Some(c.id)).expect_err("claimed by rival");
    assert!(
        matches!(err, BoardError::Claimed(ref h) if h == "codex"),
        "{err:?}"
    );
    let now = b.get(c.id).expect("card");
    assert_eq!(
        (now.column, now.claim.as_deref()),
        (Column::Ready, Some("codex")),
        "not moved"
    );
}

fn who_in<S: Store>(b: &Board<S>, name: &str, role: &str) -> Actor {
    b.roles().actor(name, role).expect("role")
}

#[test]
fn take_prefers_higher_priority_within_a_queue() {
    let b = board();
    let old = ready(&b, "old, no priority");
    let urgent = ready(&b, "new, high priority");
    let h = who(&b, "nano", "human");
    b.plan(urgent, &h, Some(5), None).expect("plan");
    let dev = who(&b, "claude", "builder");
    assert_eq!(
        b.take(&dev, None).expect("take").map(|c| c.id),
        Some(urgent)
    );
    let other = who(&b, "codex", "builder");
    assert_eq!(b.take(&other, None).expect("take").map(|c| c.id), Some(old));
}

#[test]
fn take_skips_a_card_until_its_dependencies_are_done() {
    let b = board();
    let first = ready(&b, "first");
    let then = ready(&b, "then");
    let h = who(&b, "nano", "human");
    b.plan(then, &h, Some(5), Some(vec![first])).expect("plan");
    let dev = who(&b, "claude", "builder");
    assert!(
        matches!(b.take(&dev, Some(then)), Err(BoardError::Refused(_))),
        "named explicitly, it says why"
    );
    let taken = b.take(&dev, None).expect("take").expect("a card");
    assert_eq!(
        taken.id, first,
        "the waiting card is skipped despite its priority"
    );
    let other = who(&b, "codex", "builder");
    assert!(b.take(&other, None).expect("take").is_none());

    for to in [Column::Qa, Column::Review, Column::Done] {
        b.move_card(first, to, &h).expect("move first on");
    }
    assert_eq!(
        b.take(&other, None).expect("take").map(|c| c.id),
        Some(then)
    );
}

#[test]
fn planning_needs_triage_and_real_cards() {
    let b = board();
    let a = ready(&b, "a");
    let dev = who(&b, "claude", "builder");
    assert!(matches!(
        b.plan(a, &dev, Some(5), None),
        Err(BoardError::Forbidden(_))
    ));
    let h = who(&b, "nano", "human");
    assert!(matches!(
        b.plan(a, &h, None, Some(vec![uuid::Uuid::new_v4()])),
        Err(BoardError::NotFound(_))
    ));
    assert!(matches!(
        b.plan(a, &h, None, Some(vec![a])),
        Err(BoardError::Invalid(_))
    ));
    assert!(matches!(
        b.plan(a, &h, Some(2), None),
        Err(BoardError::Invalid(_))
    ));
}

#[test]
fn the_board_keeps_its_own_tags() {
    let b = board();
    let h = who(&b, "nano", "human");
    for tag in ["agent", "Board:ready", "claim:claude"] {
        assert!(
            matches!(
                b.add(&h, "t", "", vec![tag.into()]),
                Err(BoardError::Invalid(_))
            ),
            "{tag}"
        );
    }
    let c = b
        .add(&h, "t", "", vec!["agents".into()])
        .expect("a near miss is fine");
    assert!(matches!(
        b.edit(c.id, &h, None, None, Some(vec!["agent".into()])),
        Err(BoardError::Invalid(_))
    ));
}

type Hook = Box<dyn FnOnce(&MemStore, &rusty_mill_board::domain::Card) + Send>;

/// A store that runs `hook` once, right after the first `get` once armed: a rival write
/// landing between `take`'s read of a card and its guarded commit.
struct HookAfterGet {
    inner: MemStore,
    hook: Arc<std::sync::Mutex<Option<Hook>>>,
}

impl Store for HookAfterGet {
    fn get(&self, id: uuid::Uuid) -> Result<Option<rusty_mill_board::domain::Card>, BoardError> {
        let seen = self.inner.get(id)?;
        let hook = self.hook.lock().expect("lock").take();
        if let (Some(hook), Some(card)) = (hook, &seen) {
            hook(&self.inner, card);
        }
        Ok(seen)
    }
    fn list(&self) -> Result<Vec<rusty_mill_board::domain::Card>, BoardError> {
        self.inner.list()
    }
    fn insert(
        &self,
        c: &rusty_mill_board::domain::Card,
        e: rusty_mill_board::domain::Event,
    ) -> Result<u64, BoardError> {
        self.inner.insert(c, e)
    }
    fn replace(
        &self,
        c: &rusty_mill_board::domain::Card,
        e: rusty_mill_board::domain::Event,
    ) -> Result<u64, BoardError> {
        self.inner.replace(c, e)
    }
    fn events_since(
        &self,
        a: Option<uuid::Uuid>,
        l: usize,
    ) -> Result<Vec<rusty_mill_board::domain::Event>, BoardError> {
        self.inner.events_since(a, l)
    }
    fn last_event_id(&self) -> Result<Option<uuid::Uuid>, BoardError> {
        self.inner.last_event_id()
    }
}

/// A Ready card, and an unfinished card that a rival `card_plan` makes it depend on
/// right after `take` first reads it.
fn planned_mid_take() -> (Board<HookAfterGet>, uuid::Uuid, uuid::Uuid) {
    let slot: Arc<std::sync::Mutex<Option<Hook>>> = Arc::default();
    let b = Board::new(
        HookAfterGet {
            inner: MemStore::default(),
            hook: Arc::clone(&slot),
        },
        Roles::default(),
    );
    let h = who_in(&b, "nano", "human");
    let blocker = b.add(&h, "blocker", "", vec![]).expect("add").id;
    let card = b.add(&h, "card", "", vec![]).expect("add").id;
    b.move_card(card, Column::Ready, &h).expect("ready");
    let hook: Hook = Box::new(move |store, seen| {
        let planned = seen.plan(None, Some(vec![blocker]), 2).expect("plan");
        let event = rusty_mill_board::domain::Event::of(
            &planned,
            rusty_mill_board::domain::EventKind::Edited,
            "nano",
        );
        store.replace(&planned, event).expect("rival plan");
    });
    *slot.lock().expect("lock") = Some(hook);
    (b, card, blocker)
}

#[test]
fn a_dependency_planned_during_a_named_take_refuses_it() {
    let (b, card, _) = planned_mid_take();
    let dev = who_in(&b, "claude", "builder");
    assert!(matches!(
        b.take(&dev, Some(card)),
        Err(BoardError::Refused(_))
    ));
    let now = b.get(card).expect("card");
    assert_eq!((now.column, now.claim), (Column::Ready, None), "not taken");
}

#[test]
fn a_dependency_planned_during_an_automatic_take_skips_the_card() {
    let (b, card, _) = planned_mid_take();
    let dev = who_in(&b, "claude", "builder");
    assert_eq!(b.take(&dev, None).expect("take"), None);
    let now = b.get(card).expect("card");
    assert_eq!((now.column, now.claim), (Column::Ready, None), "not taken");
}

#[test]
fn an_invalid_plan_changes_nothing() {
    let b = board();
    let id = ready(&b, "card");
    let h = who(&b, "nano", "human");
    let before = b.get(id).expect("card");
    let events = b.events(None, 100).expect("events").len();
    for deps in [vec![id], (0..33).map(|_| uuid::Uuid::now_v7()).collect()] {
        assert!(b.plan(id, &h, Some(5), Some(deps)).is_err());
    }
    let after = b.get(id).expect("card");
    assert_eq!(
        (after.rev, after.priority, after.deps),
        (before.rev, 0, vec![])
    );
    assert_eq!(b.events(None, 100).expect("events").len(), events);
}
