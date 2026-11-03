//! Board-wide card numbers: reserved in the store so no two cards ever share one, across boards
//! sharing a rusty_tick and across cards leaving and rejoining the board; lookups never guess.

use std::sync::Arc;
use std::thread;

use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::domain::{BoardError, Card, Event, EventKind};
use rusty_mill_board::projects::Projects;
use rusty_mill_board::store::{MemStore, Store};
use rusty_mill_board::tick::TickStore;
use serde_json::json;
use uuid::Uuid;

mod fake_tick;
use fake_tick::FakeTick;

fn clock() -> i64 {
    42
}

fn projects() -> Projects {
    Projects::parse(r#"{"orch": ["crates/apps/rusty_orch"]}"#).expect("projects")
}

fn tick_board(tick: &FakeTick) -> Board<TickStore<FakeTick>> {
    Board::with_clock(TickStore::new(tick.clone()), Roles::default(), clock)
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
    let numbered = card.numbered(3);
    assert_eq!((numbered.number, numbered.rev), (Some(3), card.rev + 1));
    assert_eq!(
        numbered.numbered(4),
        numbered,
        "a numbered card keeps its number"
    );
    assert!(Card::new("a", "", vec!["Number:9".into()], 0).is_err());
}

#[test]
fn a_task_made_in_rusty_tick_gets_a_number_with_the_next_card() {
    let tick = FakeTick::default();
    let board = tick_board(&tick);
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
    // #2 stays reserved for the stray's first life; numbers are never handed out twice.
    assert_eq!(board.get(stray.id).expect("get").number, Some(3));
    assert_eq!(next.number, Some(4));
    assert_eq!(board.get_number(4).expect("by number").id, next.id);
}

#[test]
fn two_boards_that_both_read_the_same_highest_number_still_reserve_different_ones() {
    let tick = FakeTick::default();
    let (one, two) = (TickStore::new(tick.clone()), TickStore::new(tick.clone()));
    // Both boards listed the cards and saw no number yet: the same floor.
    let a = one.reserve_number(Uuid::now_v7(), 0).expect("reserve");
    let b = two.reserve_number(Uuid::now_v7(), 0).expect("reserve");
    assert_eq!((a, b), (1, 2));
    assert_eq!(one.reserve_number(Uuid::now_v7(), 0), Ok(3));
    assert_eq!(tick.docs("board_number").len(), 3);
}

#[test]
fn two_boards_adding_at_once_never_share_a_number() {
    let tick = FakeTick::default();
    let boards = [Arc::new(tick_board(&tick)), Arc::new(tick_board(&tick))];
    let writers: Vec<_> = boards
        .iter()
        .flat_map(|b| [Arc::clone(b), Arc::clone(b)])
        .enumerate()
        .map(|(w, board)| {
            thread::spawn(move || {
                let human = board.roles().actor("nano", "human").expect("role");
                for i in 0..10 {
                    board
                        .add(&human, &format!("w{w}-{i}"), "", vec![])
                        .expect("add");
                }
            })
        })
        .collect();
    for w in writers {
        w.join().expect("writer");
    }
    let mut numbers: Vec<_> = boards[0]
        .list(None)
        .expect("list")
        .iter()
        .map(|c| c.number.expect("numbered"))
        .collect();
    numbers.sort_unstable();
    numbers.dedup();
    assert_eq!(numbers.len(), 40, "every card has its own number");
}

#[test]
fn a_card_that_leaves_the_board_keeps_its_number_to_itself() {
    let tick = FakeTick::default();
    let board = tick_board(&tick);
    let human = board.roles().actor("nano", "human").expect("role");
    let a = board.add(&human, "a", "", vec![]).expect("add");

    // Untagged in rusty_tick (the owner's switch), then a new card, then tagged again.
    tick.owner_edit(a.id, |t| t["tags"] = json!(["board:backlog", "number:1"]));
    let b = board.add(&human, "b", "", vec![]).expect("add");
    tick.owner_edit(a.id, |t| {
        t["tags"] = json!(["agent", "board:backlog", "number:1"]);
    });
    assert_eq!(b.number, Some(2));
    assert_eq!(board.get_number(1).expect("a").id, a.id);
    assert_eq!(board.get_number(2).expect("b").id, b.id);

    // Trashed, a new card on a fresh board (a restart), then restored.
    tick.owner_edit(b.id, |t| t["deletedMs"] = json!(43));
    let restarted = tick_board(&tick);
    let c = restarted.add(&human, "c", "", vec![]).expect("add");
    tick.owner_edit(b.id, |t| t["deletedMs"] = json!(null));
    assert_eq!(c.number, Some(3));
    assert_eq!(restarted.get_number(2).expect("b").id, b.id);
}

#[test]
fn a_number_two_cards_carry_is_refused_rather_than_guessed() {
    let tick = FakeTick::default();
    let board = tick_board(&tick);
    let human = board.roles().actor("nano", "human").expect("role");
    board.add(&human, "a", "", vec![]).expect("add");
    let b = board.add(&human, "b", "", vec![]).expect("add");
    // A number tag copied by hand in rusty_tick.
    tick.owner_edit(b.id, |t| {
        t["tags"] = json!(["agent", "board:backlog", "number:1"]);
    });
    assert_eq!(
        board.get_number(1),
        Err(BoardError::Invalid(
            "more than one card has that number; name it by id"
        ))
    );
}
