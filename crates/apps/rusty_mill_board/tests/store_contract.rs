//! The same board scenario against every store adapter.

use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::domain::{BoardError, Column, EventKind};
use rusty_mill_board::mmdb::MmdbStore;
use rusty_mill_board::store::{MemStore, Store};
use rusty_mill_board::tick::TickStore;

mod fake_tick;

fn clock() -> i64 {
    42
}

fn scenario<S: Store>(board: &Board<S>) {
    let human = board.roles().actor("nano", "human").expect("role");
    let dev = board.roles().actor("dev", "builder").expect("role");
    let other = board.roles().actor("dev2", "builder").expect("role");

    let card = board
        .add(&human, "ship it", "body", vec!["x".into()])
        .expect("add");
    assert_eq!(board.list(Some(Column::Backlog)).expect("list").len(), 1);
    board
        .move_card(card.id, Column::Ready, &human)
        .expect("ready");
    board
        .move_card(card.id, Column::Building, &dev)
        .expect("building");
    board.claim(card.id, &dev).expect("claim");
    assert_eq!(
        board.claim(card.id, &other),
        Err(BoardError::Claimed("dev".into()))
    );
    board.comment(card.id, &dev, "wip").expect("comment");

    let now = board.get(card.id).expect("get");
    assert_eq!(
        (now.column, now.claim.as_deref(), now.comments.len()),
        (Column::Building, Some("dev"), 1)
    );

    let events = board.events(None, 100).expect("events");
    let kinds: Vec<_> = events.iter().map(|e| e.kind).collect();
    assert_eq!(
        kinds,
        [
            EventKind::Created,
            EventKind::Moved,
            EventKind::Moved,
            EventKind::Claimed,
            EventKind::Commented
        ]
    );
    let tail = board.events(Some(events[2].id), 100).expect("tail");
    assert_eq!(tail.len(), 2);
    assert_eq!(
        board.get(uuid::Uuid::nil()),
        Err(BoardError::NotFound(uuid::Uuid::nil()))
    );
}

#[test]
fn mem_store() {
    scenario(&Board::with_clock(
        MemStore::default(),
        Roles::default(),
        clock,
    ));
}

#[test]
fn tick_store() {
    let tick = fake_tick::FakeTick::default();
    let board = Board::with_clock(TickStore::new(tick.clone()), Roles::default(), clock);
    scenario(&board);
    let card = &board.list(None).expect("list")[0];
    assert_eq!(card.rev, 4);
    let task = tick.task(card.id);
    assert_eq!(
        task["tags"],
        serde_json::json!(["agent", "board:building", "claim:dev", "x"])
    );
}

#[test]
fn mmdb_store_persists_across_reopen() {
    let dir = std::env::temp_dir().join(format!("mill_board_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let path = dir.join("board.mmap");
    let board = Board::with_clock(
        MmdbStore::open(&path).expect("open"),
        Roles::default(),
        clock,
    );
    scenario(&board);
    let id = board.list(None).expect("list")[0].id;
    drop(board);

    let reopened = Board::with_clock(
        MmdbStore::open(&path).expect("reopen"),
        Roles::default(),
        clock,
    );
    let card = reopened.get(id).expect("persisted card");
    assert_eq!((card.title.as_str(), card.rev), ("ship it", 4));
    assert_eq!(reopened.events(None, 100).expect("events").len(), 5);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn stale_replace_conflicts() {
    use rusty_mill_board::domain::{Card, Event};
    let store = MemStore::default();
    let card = Card::new("a", "", vec![], 0).expect("card");
    store
        .insert(&card, Event::of(&card, EventKind::Created, "x"))
        .expect("insert");
    let human = Roles::default().actor("n", "human").expect("role");
    let v1 = card.edit(Some("b"), None, None, 1).expect("edit");
    store
        .replace(&v1, Event::of(&v1, EventKind::Edited, "x"))
        .expect("v1");
    let stale = card.comment(&human, "late", 2).expect("comment");
    assert_eq!(
        store.replace(&stale, Event::of(&stale, EventKind::Commented, "x")),
        Err(BoardError::Conflict)
    );
}

/// The engine can fail *after* its record is durable. The store must not claim "nothing
/// changed": it refuses further writes, and a reopen reports what really happened.
#[test]
fn an_append_that_fails_after_the_log_write_is_unsettled_until_reopened() {
    use rusty_multimodal_db_engine::test_support::{Fault, arm_fault};

    let dir = std::env::temp_dir().join(format!("mill_board_fault_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let path = dir.join("board.mmdb");
    let _ = std::fs::remove_file(&path);
    let board = Board::new(MmdbStore::open(&path).expect("open"), Roles::default());
    let human = board.roles().actor("nano", "human").expect("role");
    let card = board.add(&human, "a", "", vec![]).expect("add");

    arm_fault(Fault::InsertAfterLog);
    let failed = board.move_card(card.id, Column::Ready, &human);
    assert!(matches!(failed, Err(BoardError::Storage(_))), "{failed:?}");
    assert_eq!(
        board.get(card.id).expect("get").column,
        Column::Backlog,
        "memory keeps the last known state"
    );
    let unhealthy = board.health();
    assert!(
        matches!(unhealthy, Err(BoardError::Storage(ref m)) if m.contains("restart")),
        "health says a restart is needed: {unhealthy:?}"
    );
    // The log holds an event the state lacks: neither a copy nor a compaction may touch it.
    assert!(board.backup(&dir).is_err(), "no backup while unsettled");
    assert!(board.compact().is_err(), "no compaction while unsettled");
    let refused = board.comment(card.id, &human, "later");
    assert!(
        matches!(refused, Err(BoardError::Storage(ref m)) if m.contains("reopen")),
        "no write on a guess: {refused:?}"
    );
    drop(board);

    let board = Board::new(MmdbStore::open(&path).expect("reopen"), Roles::default());
    let now = board.get(card.id).expect("get");
    assert_eq!(now.column, Column::Ready, "the durable record is recovered");
    assert!(now.comments.is_empty(), "the refused write never happened");
    board
        .comment(card.id, &human, "now fine")
        .expect("writable again");
    assert_eq!(board.health(), Ok(()));
    let _ = std::fs::remove_dir_all(&dir);
}
