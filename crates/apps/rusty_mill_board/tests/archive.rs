//! Archiving Done cards: `Board::archive`, `restore`, `archive_done` and the shelved views.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::domain::{
    Actor, BoardError, Capability, Card, Column, Event, EventKind, Shelf,
};
use rusty_mill_board::projects::Projects;
use rusty_mill_board::store::{MemStore, Store};
use uuid::Uuid;

const DAY: i64 = 24 * 60 * 60 * 1000;

/// One clock per test binary would race between tests, so each test gets a board whose clock
/// reads its own slot.
static CLOCKS: [AtomicI64; 12] = [const { AtomicI64::new(0) }; 12];

macro_rules! clock {
    ($slot:expr) => {{
        fn now() -> i64 {
            CLOCKS[$slot].load(Ordering::SeqCst)
        }
        now as fn() -> i64
    }};
}

fn set(slot: usize, ms: i64) {
    CLOCKS[slot].store(ms, Ordering::SeqCst);
}

fn who<S: Store>(b: &Board<S>, name: &str, role: &str) -> Actor {
    b.roles().actor(name, role).expect("role")
}

fn titles(cards: Vec<Card>) -> Vec<String> {
    cards.into_iter().map(|c| c.title).collect()
}

#[test]
fn an_archived_card_leaves_the_default_view_and_is_still_done() {
    let b = Board::with_clock(MemStore::default(), Roles::default(), clock!(0));
    let dev = who(&b, "claude", "builder");
    let shipped = b
        .add_done_in(&dev, "default", "shipped", "", vec![])
        .expect("done");
    b.add(&dev, "open", "", vec![]).expect("add");

    let card = b.archive(shipped.id, &dev).expect("archive");
    assert_eq!((card.column, card.archived_ms), (Column::DONE, Some(0)));
    assert_eq!(
        titles(b.list_shelf(None, Shelf::Exclude, 14).expect("list")),
        ["open"]
    );
    assert_eq!(
        titles(b.list_shelf(None, Shelf::Only, 14).expect("list")),
        ["shipped"]
    );
    assert_eq!(
        b.list_shelf(None, Shelf::Include, 14).expect("list").len(),
        2
    );
    assert_eq!(
        b.get(shipped.id).expect("get").done_ms,
        Some(0),
        "still shipped"
    );

    let events = b.events(None, 100).expect("events");
    assert_eq!(events.last().map(|e| e.kind), Some(EventKind::Archived));

    let back = b.restore(shipped.id, &dev).expect("restore");
    assert_eq!(back.archived_ms, None);
    assert_eq!(
        b.list_shelf(None, Shelf::Exclude, 14).expect("list").len(),
        2
    );
}

#[test]
fn archiving_twice_or_restoring_an_unarchived_card_changes_nothing() {
    let b = Board::with_clock(MemStore::default(), Roles::default(), clock!(1));
    let dev = who(&b, "claude", "builder");
    let c = b
        .add_done_in(&dev, "default", "a", "", vec![])
        .expect("done");
    let once = b.archive(c.id, &dev).expect("archive");
    set(1, 5);
    let twice = b.archive(c.id, &dev).expect("again");
    assert_eq!((twice.rev, twice.archived_ms), (once.rev, Some(0)));
    let restored = b.restore(c.id, &dev).expect("restore");
    let again = b.restore(c.id, &dev).expect("restore again");
    assert_eq!(again.rev, restored.rev);
}

#[test]
fn only_done_cards_are_archived_and_only_by_close() {
    let b = Board::with_clock(MemStore::default(), Roles::default(), clock!(2));
    let human = who(&b, "nano", "human");
    let open = b.add(&human, "open", "", vec![]).expect("add");
    assert_eq!(
        b.archive(open.id, &human),
        Err(BoardError::Invalid("only done cards can be archived"))
    );

    let done = b
        .add_done_in(&human, "default", "done", "", vec![])
        .expect("done");
    let reviewer = who(&b, "rev", "reviewer");
    assert_eq!(
        b.archive(done.id, &reviewer),
        Err(BoardError::Forbidden(Capability::Close))
    );
    assert_eq!(
        b.restore(done.id, &reviewer),
        Err(BoardError::Forbidden(Capability::Close))
    );
}

#[test]
fn a_done_card_ages_out_of_the_default_view_after_the_window() {
    let b = Board::with_clock(MemStore::default(), Roles::default(), clock!(3));
    let dev = who(&b, "claude", "builder");
    b.add_done_in(&dev, "default", "old", "", vec![])
        .expect("done");

    set(3, 14 * DAY);
    assert_eq!(
        titles(b.list_shelf(None, Shelf::Exclude, 14).expect("list")),
        ["old"],
        "exactly the window is still on the board"
    );
    set(3, 14 * DAY + 1);
    assert!(
        b.list_shelf(None, Shelf::Exclude, 14)
            .expect("list")
            .is_empty()
    );
    assert_eq!(
        titles(b.list_shelf(None, Shelf::Only, 14).expect("list")),
        ["old"]
    );
    assert_eq!(
        titles(b.list_shelf(None, Shelf::Exclude, 0).expect("list")),
        ["old"],
        "no window: only archiving shelves"
    );
}

#[test]
fn archive_done_takes_old_enough_done_cards_in_one_project() {
    let b = Board::with_clock(MemStore::default(), Roles::default(), clock!(4)).with_projects(
        rusty_mill_board::projects::Projects::parse(r#"{"orch": ["crates/apps/rusty_orch"]}"#)
            .expect("projects"),
    );
    let human = who(&b, "nano", "human");
    b.add_done_in(&human, "default", "old inbox", "", vec![])
        .expect("done");
    b.add_done_in(&human, "orch", "old orch", "", vec![])
        .expect("done");
    set(4, 10 * DAY);
    b.add_done_in(&human, "orch", "new orch", "", vec![])
        .expect("done");
    b.add(&human, "open", "", vec![]).expect("add");

    let archived = b.archive_done(&human, Some("orch"), 5).expect("bulk");
    assert_eq!(titles(archived), ["old orch"]);
    let all = b.archive_done(&human, None, 0).expect("bulk");
    assert_eq!(titles(all), ["old inbox", "new orch"]);
    assert!(b.archive_done(&human, None, 0).expect("bulk").is_empty());

    let dev = who(&b, "claude", "builder");
    assert_eq!(
        b.archive_done(&dev, None, 0),
        Err(BoardError::Forbidden(Capability::Triage))
    );
}

#[test]
fn archiving_leaves_roadmap_and_metrics_alone() {
    let b = Board::with_clock(MemStore::default(), Roles::default(), clock!(5));
    let human = who(&b, "nano", "human");
    let c = b
        .add_done_in(&human, "default", "shipped", "", vec![])
        .expect("done");
    let roadmap = serde_json::to_value(b.roadmap("default").expect("roadmap")).expect("json");
    let metrics = serde_json::to_value(b.metrics(|_| true).expect("metrics")).expect("json");
    b.archive(c.id, &human).expect("archive");
    assert_eq!(
        serde_json::to_value(b.roadmap("default").expect("roadmap")).expect("json"),
        roadmap
    );
    assert_eq!(
        serde_json::to_value(b.metrics(|_| true).expect("metrics")).expect("json"),
        metrics
    );
}

#[test]
fn restoring_an_old_archived_card_puts_it_back_without_rewriting_its_ship_time() {
    let b = Board::with_clock(MemStore::default(), Roles::default(), clock!(6));
    let dev = who(&b, "claude", "builder");
    let c = b
        .add_done_in(&dev, "default", "old", "", vec![])
        .expect("done");
    set(6, 20 * DAY);
    b.archive(c.id, &dev).expect("archive");
    let back = b.restore(c.id, &dev).expect("restore");
    assert_eq!(
        (back.archived_ms, back.restored_ms, back.done_ms),
        (None, Some(20 * DAY), Some(0))
    );
    assert_eq!(
        titles(b.list_shelf(None, Shelf::Exclude, 14).expect("list")),
        ["old"]
    );
    assert!(
        b.list_shelf(None, Shelf::Only, 14)
            .expect("list")
            .is_empty()
    );

    // Its window starts again at the restore.
    set(6, 34 * DAY + 1);
    assert_eq!(
        titles(b.list_shelf(None, Shelf::Only, 14).expect("list")),
        ["old"]
    );
}

#[test]
fn restoring_a_card_that_only_aged_out_puts_it_back() {
    let b = Board::with_clock(MemStore::default(), Roles::default(), clock!(7));
    let dev = who(&b, "claude", "builder");
    let c = b
        .add_done_in(&dev, "default", "aged", "", vec![])
        .expect("done");
    set(7, 15 * DAY);
    assert!(
        b.list_shelf(None, Shelf::Exclude, 14)
            .expect("list")
            .is_empty()
    );
    let back = b.restore(c.id, &dev).expect("restore");
    assert_eq!(back.restored_ms, Some(15 * DAY));
    assert_eq!(
        titles(b.list_shelf(None, Shelf::Exclude, 14).expect("list")),
        ["aged"]
    );
    let events = b.events(None, 100).expect("events");
    assert_eq!(events.last().map(|e| e.kind), Some(EventKind::Restored));

    // Restoring a card on the board changes nothing.
    assert_eq!(b.restore(c.id, &dev).expect("again").rev, back.rev);
    // A bulk sweep by age counts from the restore too.
    let human = who(&b, "nano", "human");
    assert!(b.archive_done(&human, None, 14).expect("bulk").is_empty());
}

/// A store that changes a card under the board between its read and its commit: the first
/// `replace` of `target` first commits `meddle(card)`, so the board's commit conflicts and
/// it re-reads the meddled card.
struct Meddling {
    inner: MemStore,
    target: Arc<Mutex<Option<Uuid>>>,
    meddle: fn(&Card) -> Card,
}

impl Store for Meddling {
    fn get(&self, id: Uuid) -> Result<Option<Card>, BoardError> {
        self.inner.get(id)
    }
    fn list(&self) -> Result<Vec<Card>, BoardError> {
        self.inner.list()
    }
    fn insert(&self, card: &Card, event: Event) -> Result<u64, BoardError> {
        self.inner.insert(card, event)
    }
    fn replace(&self, card: &Card, event: Event) -> Result<u64, BoardError> {
        let first = self.target.lock().expect("lock").take_if(|t| *t == card.id);
        if first.is_some() {
            let current = self.inner.get(card.id)?.expect("card");
            let meddled = Card {
                rev: current.rev + 1,
                ..(self.meddle)(&current)
            };
            self.inner
                .replace(&meddled, Event::of(&meddled, EventKind::Edited, "other"))?;
        }
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
}

/// Two Done cards in `orch`, done at 0, on a board whose store meddles with the first once
/// [`arm`]ed.
type Meddled = (Board<Meddling>, Arc<Mutex<Option<Uuid>>>, Uuid, Uuid);

fn meddled(slot: usize, meddle: fn(&Card) -> Card) -> Meddled {
    let target = Arc::new(Mutex::new(None));
    let store = Meddling {
        inner: MemStore::default(),
        target: Arc::clone(&target),
        meddle,
    };
    let projects = Projects::parse(r#"{"orch": ["a"], "tick": ["b"]}"#).expect("projects");
    let b = Board::with_clock(store, Roles::default(), clock_for(slot)).with_projects(projects);
    let human = who(&b, "nano", "human");
    let first = b
        .add_done_in(&human, "orch", "meddled", "", vec![])
        .expect("done");
    let second = b
        .add_done_in(&human, "orch", "plain", "", vec![])
        .expect("done");
    (b, target, first.id, second.id)
}

fn clock_for(slot: usize) -> fn() -> i64 {
    match slot {
        8 => clock!(8),
        9 => clock!(9),
        10 => clock!(10),
        _ => clock!(11),
    }
}

fn arm(target: &Mutex<Option<Uuid>>, id: Uuid) {
    *target.lock().expect("lock") = Some(id);
}

#[test]
fn bulk_archive_skips_a_card_moved_to_another_project_meanwhile() {
    let (b, target, moved, plain) = meddled(8, |c| Card {
        project: "tick".into(),
        ..c.clone()
    });
    arm(&target, moved);
    let human = who(&b, "nano", "human");
    let archived = b.archive_done(&human, Some("orch"), 0).expect("bulk");
    assert_eq!(archived.iter().map(|c| c.id).collect::<Vec<_>>(), [plain]);
    let card = b.get(moved).expect("get");
    assert_eq!((card.project.as_str(), card.archived_ms), ("tick", None));
}

#[test]
fn bulk_archive_skips_a_card_archived_or_restored_meanwhile() {
    let (b, target, archived_meanwhile, _) = meddled(9, |c| Card {
        archived_ms: Some(1),
        ..c.clone()
    });
    set(9, 10 * DAY);
    arm(&target, archived_meanwhile);
    let human = who(&b, "nano", "human");
    let archived = b.archive_done(&human, None, 0).expect("bulk");
    assert_eq!(archived.len(), 1, "only the other card is counted");
    assert_eq!(
        b.get(archived_meanwhile).expect("get").archived_ms,
        Some(1),
        "the other writer's archive stands"
    );

    let (b, target, restored_meanwhile, _) = meddled(10, |c| Card {
        restored_ms: Some(10 * DAY),
        ..c.clone()
    });
    set(10, 10 * DAY);
    arm(&target, restored_meanwhile);
    let archived = b.archive_done(&human, None, 5).expect("bulk");
    assert_eq!(
        archived.len(),
        1,
        "a card restored just now is no longer old enough"
    );
    assert_eq!(b.get(restored_meanwhile).expect("get").archived_ms, None);
}
