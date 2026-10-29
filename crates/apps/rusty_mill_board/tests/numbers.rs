//! Board-wide card numbers: handed out in creation order, never changed, and given to cards that
//! came without one.

use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::domain::{BoardError, Card, Event, EventKind};
use rusty_mill_board::projects::Projects;
use rusty_mill_board::store::{MemStore, Store};
use rusty_mill_board::tick::TickStore;
use serde_json::json;

mod fake_tick;
use fake_tick::FakeTick;

fn clock() -> i64 {
    42
}

fn projects() -> Projects {
    Projects::parse(r#"{"orch": ["crates/apps/rusty_orch"]}"#).expect("projects")
}

#[test]
fn numbers_count_up_across_projects_and_imports() {
    let board =
        Board::with_clock(MemStore::default(), Roles::default(), clock).with_projects(projects());
    let human = board.roles().actor("nano", "human").expect("role");
    let a = board.add(&human, "a", "", vec![]).expect("add");
    let b = board.add_in(&human, "orch", "b", "", vec![]).expect("add");
    let c = board
        .add_done_in(&human, "orch", "c", "", vec![])
        .expect("add");
    let d = board.import(&human, "d", "", vec![], 7).expect("import");
    let numbers: Vec<_> = [&a, &b, &c, &d].iter().map(|c| c.number).collect();
    assert_eq!(numbers, [Some(1), Some(2), Some(3), Some(4)]);
    assert_eq!(board.get_number(2).expect("by number").id, b.id);
    assert_eq!(
        board.get_number(5),
        Err(BoardError::Invalid("no card has that number"))
    );
    let moved = board.assign(b.id, "default", &human).expect("assign");
    assert_eq!(moved.number, Some(2), "re-homing keeps the number");
}

#[test]
fn unnumbered_cards_are_numbered_oldest_first_after_the_highest() {
    let store = MemStore::default();
    let older = Card::new("older", "", vec![], 1).expect("card");
    let newer = Card::new("newer", "", vec![], 2).expect("card");
    let numbered = Card {
        number: Some(5),
        ..Card::new("numbered", "", vec![], 0).expect("card")
    };
    for card in [&newer, &older, &numbered] {
        store
            .insert(card, Event::of(card, EventKind::Created, "x"))
            .expect("insert");
    }
    let board = Board::with_clock(store, Roles::default(), clock);

    assert_eq!(board.number_cards(), Ok(2));
    assert_eq!(board.get(older.id).expect("get").number, Some(6));
    assert_eq!(board.get(newer.id).expect("get").number, Some(7));
    assert_eq!(
        board.number_cards(),
        Ok(0),
        "numbering twice changes nothing"
    );
    let kinds: Vec<_> = board
        .events(None, 10)
        .expect("events")
        .iter()
        .map(|e| (e.kind, e.actor.clone()))
        .collect();
    assert_eq!(
        kinds[3..],
        [
            (EventKind::Numbered, "board".into()),
            (EventKind::Numbered, "board".into())
        ]
    );
}

#[test]
fn a_number_never_changes_and_is_not_a_tag_people_set() {
    let card = Card::new("a", "", vec![], 0).expect("card");
    let numbered = card.numbered(3).expect("number");
    assert_eq!((numbered.number, numbered.rev), (Some(3), card.rev + 1));
    assert_eq!(
        numbered.numbered(4),
        Err(BoardError::Invalid("card is already numbered"))
    );
    assert!(Card::new("a", "", vec!["Number:9".into()], 0).is_err());
}

#[test]
fn a_task_made_in_rusty_tick_gets_the_next_number_with_the_next_card() {
    let tick = FakeTick::default();
    let board = Board::with_clock(TickStore::new(tick.clone()), Roles::default(), clock);
    let human = board.roles().actor("nano", "human").expect("role");
    let first = board.add(&human, "first", "", vec![]).expect("add");
    assert_eq!(
        tick.task(first.id)["tags"],
        json!(["agent", "board:backlog", "number:1"])
    );

    // As if the owner had tagged a task `agent` on the phone: on the board, but unnumbered.
    let stray = board.add(&human, "stray", "", vec![]).expect("add");
    tick.owner_edit(stray.id, |t| t["tags"] = json!(["agent"]));
    assert_eq!(board.get(stray.id).expect("get").number, None);

    let next = board.add(&human, "next", "", vec![]).expect("add");
    assert_eq!(board.get(stray.id).expect("get").number, Some(2));
    assert_eq!(next.number, Some(3));
    assert_eq!(board.get_number(3).expect("by number").id, next.id);
}
