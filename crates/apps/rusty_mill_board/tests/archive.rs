//! Archiving Done cards: `Board::archive`, `restore`, `archive_done` and the shelved views.

use std::sync::atomic::{AtomicI64, Ordering};

use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::domain::{Actor, BoardError, Capability, Column, EventKind, Shelf};
use rusty_mill_board::store::{MemStore, Store};

const DAY: i64 = 24 * 60 * 60 * 1000;

/// One clock per test binary would race between tests, so each test gets a board whose clock
/// reads its own slot.
static CLOCKS: [AtomicI64; 6] = [const { AtomicI64::new(0) }; 6];

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

fn titles(cards: Vec<rusty_mill_board::domain::Card>) -> Vec<String> {
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
