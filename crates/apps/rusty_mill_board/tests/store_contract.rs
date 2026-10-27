//! The same board scenario against every store adapter.

use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::domain::{BoardError, Column, EventKind};
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
