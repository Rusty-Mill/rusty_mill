//! `TickStore` specifics: how a card lands on its rusty_tick task, and how the
//! owner's edits in rusty_tick reach the board (ADR-0012).

use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::domain::{BoardError, Card, Column, Event, EventKind};
use rusty_mill_board::projects::Projects;
use rusty_mill_board::store::Store;
use rusty_mill_board::tick::TickStore;
use serde_json::json;

mod fake_tick;
use fake_tick::FakeTick;

fn clock() -> i64 {
    42
}

fn board() -> (Board<TickStore<FakeTick>>, FakeTick) {
    let tick = FakeTick::default();
    let projects = Projects::parse(r#"{"orch": ["crates/apps/rusty_orch"]}"#).expect("projects");
    let board = Board::with_clock(TickStore::new(tick.clone()), Roles::default(), clock)
        .with_projects(projects);
    (board, tick)
}

#[test]
fn a_task_tagged_agent_in_rusty_tick_is_on_the_board_and_untagging_removes_it() {
    let (board, tick) = board();
    let human = board.roles().actor("nano", "human").expect("role");
    let card = board.add(&human, "a", "", vec![]).expect("add");
    tick.owner_edit(card.id, |t| t["tags"] = json!(["board:ready", "Agent"]));
    tick.owner_edit(card.id, |t| t["tags"] = json!(["agent", "board:ready"]));
    assert_eq!(board.get(card.id).expect("get").column, Column::Ready);

    tick.owner_edit(card.id, |t| t["tags"] = json!(["board:ready"]));
    assert_eq!(board.get(card.id), Err(BoardError::NotFound(card.id)));
    assert!(board.list(None).expect("list").is_empty());
}

#[test]
fn an_owner_edit_in_rusty_tick_is_read_next_and_shows_as_an_event() {
    let (board, tick) = board();
    let human = board.roles().actor("nano", "human").expect("role");
    let card = board.add(&human, "a", "", vec![]).expect("add");
    let cursor = board.cursor().expect("cursor");

    tick.owner_edit(card.id, |t| t["title"] = json!("renamed on the phone"));
    let read = board.get(card.id).expect("get");
    assert_eq!((read.title.as_str(), read.rev), ("renamed on the phone", 1));

    let events = board.events(cursor, 10).expect("events");
    assert_eq!(events.len(), 1);
    assert_eq!(
        (events[0].kind, events[0].actor.as_str()),
        (EventKind::Edited, "tick")
    );
    // The board writes on top of the owner's edit, not over it.
    let moved = board
        .move_card(card.id, Column::Ready, &human)
        .expect("move");
    assert_eq!(
        (moved.title.as_str(), moved.rev),
        ("renamed on the phone", 2)
    );
}

#[test]
fn a_write_from_a_stale_read_conflicts() {
    let tick = FakeTick::default();
    let store = TickStore::new(tick.clone());
    let card = Card::new("a", "", vec![], 0).expect("card");
    store
        .insert(&card, Event::of(&card, EventKind::Created, "x"))
        .expect("insert");
    tick.owner_edit(card.id, |t| t["notes"] = json!("owner"));

    let human = Roles::default().actor("nano", "human").expect("role");
    let stale = card.comment(&human, "late", 1).expect("comment");
    assert_eq!(
        store.replace(&stale, Event::of(&stale, EventKind::Commented, "x")),
        Err(BoardError::Conflict)
    );
    assert!(tick.docs("comment").is_empty());
}

#[test]
fn done_closes_the_task_and_a_task_closed_in_rusty_tick_is_done() {
    let (board, tick) = board();
    let human = board.roles().actor("nano", "human").expect("role");
    let card = board.add(&human, "a", "", vec![]).expect("add");
    for column in [Column::Ready, Column::Building, Column::Qa, Column::Review] {
        board.move_card(card.id, column, &human).expect("move");
        assert_eq!(tick.task(card.id)["status"], "open");
    }
    board
        .move_card(card.id, Column::Done, &human)
        .expect("done");
    assert_eq!(tick.task(card.id)["status"], "done");

    let other = board.add(&human, "b", "", vec![]).expect("add");
    tick.owner_edit(other.id, |t| t["status"] = json!("wontdo"));
    assert_eq!(board.get(other.id).expect("get").column, Column::Done);
}

#[test]
fn the_default_project_is_the_inbox_and_a_project_is_a_list() {
    let (board, tick) = board();
    let human = board.roles().actor("nano", "human").expect("role");
    let card = board.add(&human, "a", "", vec![]).expect("add");
    let inbox = tick.task(card.id)["listId"].clone();
    assert_eq!(inbox, json!("00000000-0000-7000-8000-000000000001"));

    let moved = board.assign(card.id, "orch", &human).expect("assign");
    assert_eq!(moved.project, "orch");
    assert_ne!(tick.task(card.id)["listId"], inbox);
    assert_eq!(board.get(card.id).expect("get").project, "orch");
}

#[test]
fn deps_and_comments_are_rusty_tick_documents() {
    let (board, tick) = board();
    let human = board.roles().actor("nano", "human").expect("role");
    let first = board.add(&human, "first", "", vec![]).expect("add");
    let second = board.add(&human, "second", "", vec![]).expect("add");
    let planned = board
        .plan(second.id, &human, Some(5), Some(vec![first.id]))
        .expect("plan");
    assert_eq!(
        (planned.priority, planned.deps.clone()),
        (5, vec![first.id])
    );
    assert_eq!(tick.task(second.id)["priority"], 5);
    let doc = tick.doc("board", second.id).expect("board doc");
    assert_eq!(doc["deps"], json!([first.id]));

    board.comment(second.id, &human, "why").expect("comment");
    let comments = tick.docs("comment");
    assert_eq!(comments.len(), 1);
    assert_eq!(
        (
            &comments[0]["text"],
            &comments[0]["by"],
            &comments[0]["taskId"]
        ),
        (&json!("why"), &json!("nano"), &json!(second.id))
    );
    let read = board.get(second.id).expect("get");
    assert_eq!(
        (read.comments.len(), read.deps.clone()),
        (1, vec![first.id])
    );
}

#[test]
fn a_claim_rusty_tick_would_lowercase_is_refused() {
    let store = TickStore::new(FakeTick::default());
    let mut card = Card::new("a", "", vec![], 0).expect("card");
    card.claim = Some("Dev".into());
    assert!(matches!(
        store.insert(&card, Event::of(&card, EventKind::Created, "x")),
        Err(BoardError::Invalid(_))
    ));
}
