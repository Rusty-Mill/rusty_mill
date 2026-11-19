//! The copy from rusty_tick into SQLite (ADR-0007): exact on a fully used board, read-only
//! towards rusty_tick, and loud when the copy cannot be right.

use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::domain::{Column, Span};
use rusty_mill_board::migrate::migrate;
use rusty_mill_board::milestones::{Milestone, MilestoneStore};
use rusty_mill_board::sqlite::SqliteStore;
use rusty_mill_board::store::Store;
use rusty_mill_board::tick::TickStore;
use serde_json::json;

mod fake_tick;
use fake_tick::FakeTick;

/// A tick board with cards in several columns, a claim, comments, a reservation gap, a
/// milestone and move history.
fn seeded() -> (FakeTick, TickStore<FakeTick>) {
    let tick = FakeTick::default();
    let board = Board::new(TickStore::new(tick.clone()), Roles::default());
    let human = board.roles().actor("nano", "human").expect("role");
    let dev = board.roles().actor("dev", "builder").expect("role");
    let first = board
        .add(&human, "one", "body", vec!["x".into()])
        .expect("add");
    board
        .move_card(first.id, &Column::READY, &human)
        .expect("ready");
    board
        .move_card(first.id, &Column::BUILDING, &dev)
        .expect("building");
    board.claim(first.id, &dev).expect("claim");
    board.comment(first.id, &dev, "wip").expect("comment");
    let second = board.add(&human, "two", "", vec![]).expect("add");
    board.comment(second.id, &human, "hi").expect("comment");
    tick.record_move(first.id, 100);
    tick.record_move(first.id, 200);
    tick.record_move(second.id, 150);
    // A task that left the board keeps its moves in rusty_tick.
    tick.record_move(uuid::Uuid::new_v4(), 175);
    (tick.clone(), TickStore::new(tick))
}

#[test]
fn a_used_board_copies_exactly() {
    let (_tick, source) = seeded();
    let target = SqliteStore::open_in_memory().expect("open");
    let report = migrate(&source, &target).expect("migrate");
    assert!(report.exact(), "{:?}", report.mismatches);
    assert_eq!(
        (
            report.cards,
            report.numbered,
            report.comments,
            report.reservations
        ),
        (2, 2, 2, 2)
    );
    assert_eq!((report.moves, report.moves_dropped), (3, 1));
    assert_eq!(report.per_column.get("building"), Some(&1));
    assert_eq!(report.per_column.get("backlog"), Some(&1));
    assert_eq!(target.list(), source.list(), "every field of every card");
    assert_eq!(target.highest_reserved(), source.highest_reserved());
    assert_eq!(
        target
            .moved_in(Span {
                from: Some(120),
                to: Some(160)
            })
            .expect("moved")
            .len(),
        1
    );
    // Everyone who moved in rusty_tick and is still a card; the departed task is left out.
    let moved = target.moved_in(Span::default()).expect("moved");
    let cards: std::collections::BTreeSet<_> =
        source.list().expect("list").iter().map(|c| c.id).collect();
    assert_eq!(moved, cards);
}

#[test]
fn a_number_keeps_being_handed_out_above_the_copy() {
    let (_tick, source) = seeded();
    let target = SqliteStore::open_in_memory().expect("open");
    migrate(&source, &target).expect("migrate");
    let board = Board::new(target, Roles::default());
    let human = board.roles().actor("nano", "human").expect("role");
    assert_eq!(
        board.add(&human, "new", "", vec![]).expect("add").number,
        Some(3)
    );
}

#[test]
fn a_milestone_keeps_its_revision() {
    let (_tick, source) = seeded();
    let milestone = Milestone {
        id: uuid::Uuid::new_v4(),
        project: "default".into(),
        title: "m".into(),
        target_ms: 9,
        done_ms: None,
        rev: 0,
    };
    let rev = source.add_milestone(&milestone).expect("add");
    let target = SqliteStore::open_in_memory().expect("open");
    let report = migrate(&source, &target).expect("migrate");
    assert!(report.exact(), "{:?}", report.mismatches);
    assert_eq!(report.milestones, 1);
    assert_eq!(
        target
            .milestone(milestone.id)
            .expect("get")
            .expect("some")
            .rev,
        rev
    );
}

#[test]
fn rusty_tick_is_only_read() {
    let (tick, source) = seeded();
    let before: Vec<_> = source
        .list()
        .expect("list")
        .iter()
        .map(|c| tick.task(c.id))
        .collect();
    migrate(&source, &SqliteStore::open_in_memory().expect("open")).expect("migrate");
    let after: Vec<_> = source
        .list()
        .expect("list")
        .iter()
        .map(|c| tick.task(c.id))
        .collect();
    assert_eq!(before, after);
}

#[test]
fn a_target_that_already_has_cards_is_refused_and_left_alone() {
    let (_tick, source) = seeded();
    let target = SqliteStore::open_in_memory().expect("open");
    migrate(&source, &target).expect("first");
    let err = migrate(&source, &target).expect_err("second");
    assert!(err.contains("writing SQLite"), "{err}");
    assert_eq!(target.list().expect("list").len(), 2);
}

#[test]
fn a_card_number_nothing_reserved_is_reported() {
    let (tick, source) = seeded();
    let card = source.list().expect("list")[0].clone();
    // The owner stamps a number in rusty_tick that was never reserved.
    tick.owner_edit(card.id, |task| {
        let tags = task["tags"].as_array_mut().expect("tags");
        tags.retain(|t| !t.as_str().is_some_and(|t| t.starts_with("number:")));
        tags.push(json!("number:99"));
    });
    let report = migrate(&source, &SqliteStore::open_in_memory().expect("open")).expect("migrate");
    assert!(!report.exact());
    assert!(
        report
            .mismatches
            .iter()
            .any(|m| m.contains("no reservation")),
        "{:?}",
        report.mismatches
    );
}
