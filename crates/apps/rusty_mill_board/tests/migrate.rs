//! The copy from rusty_tick into SQLite (ADR-0007): exact on a fully used board, read-only
//! towards rusty_tick, and loud when the copy cannot be right.

use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::domain::BoardError;
use rusty_mill_board::domain::{Column, Span};
use rusty_mill_board::migrate::{migrate, migrate_to_file};
use rusty_mill_board::milestones::{Milestone, MilestoneStore};
use rusty_mill_board::sqlite::SqliteStore;
use rusty_mill_board::store::Store;
use rusty_mill_board::tick::{TickApi, TickRequest, TickStore};
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

/// rusty_tick as `FakeTick`, with its answers to `/moves` or `/docs/board_number` altered.
struct Altered {
    tick: FakeTick,
    alter: fn(&str, &mut serde_json::Value),
}

impl TickApi for Altered {
    fn send(&self, r: &TickRequest<'_>) -> Result<(u16, serde_json::Value), BoardError> {
        let (status, mut body) = self.tick.send(r)?;
        (self.alter)(&r.path, &mut body);
        Ok((status, body))
    }
}

fn altered(alter: fn(&str, &mut serde_json::Value)) -> TickStore<Altered> {
    TickStore::new(Altered {
        tick: seeded().0,
        alter,
    })
}

#[test]
fn a_move_cursor_that_is_not_the_end_is_an_error_not_a_short_history() {
    let source = altered(|path, body| {
        if path.starts_with("/moves?") {
            body["next"] = json!("junk");
        }
    });
    let err = source.history().expect_err("history");
    assert!(err.to_string().contains("history is incomplete"), "{err}");
    let target = SqliteStore::open_in_memory().expect("open");
    assert!(migrate(&source, &target).is_err());
    assert!(
        target.list().expect("list").is_empty(),
        "nothing was written"
    );
    // A stalled cursor (not past `after`) is the same failure.
    let stalled = altered(|path, body| {
        if path.starts_with("/moves?") {
            body["next"] = json!(0);
        }
    });
    assert!(stalled.history().is_err());
}

#[test]
fn a_malformed_reservation_is_an_error_not_a_lost_high_water_mark() {
    let source = altered(|path, body| {
        if path.starts_with("/docs/board_number") {
            body["docs"][1]["body"]["number"] = json!("x");
        }
    });
    let err = source.history().expect_err("history");
    assert!(err.to_string().contains("reservation"), "{err}");
    assert!(migrate(&source, &SqliteStore::open_in_memory().expect("open")).is_err());
}

fn unreserved() -> (FakeTick, TickStore<FakeTick>) {
    let (tick, source) = seeded();
    let card = source.list().expect("list")[0].clone();
    tick.owner_edit(card.id, |task| {
        let tags = task["tags"].as_array_mut().expect("tags");
        tags.retain(|t| !t.as_str().is_some_and(|t| t.starts_with("number:")));
        tags.push(json!("number:99"));
    });
    (tick, source)
}

#[test]
fn a_source_known_to_be_wrong_writes_nothing() {
    let (_tick, source) = unreserved();
    let target = SqliteStore::open_in_memory().expect("open");
    let report = migrate(&source, &target).expect("report");
    assert!(!report.exact());
    assert!(
        target.list().expect("list").is_empty(),
        "no cards were imported"
    );
    assert_eq!(target.highest_reserved(), Ok(0));
}

fn dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mill_migrate_{name}_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("dir");
    dir
}

#[test]
fn a_failed_file_migration_leaves_no_file_and_can_be_run_again() {
    let dir = dir("retry");
    let path = dir.join("board.db");
    let (tick, source) = unreserved();
    let report = migrate_to_file(&source, &path).expect("report");
    assert!(!report.exact());
    assert_eq!(
        std::fs::read_dir(&dir).expect("dir").count(),
        0,
        "no file, no partial"
    );
    // The source is fixed and the same command succeeds.
    let card = source.list().expect("list")[0].clone();
    tick.owner_edit(card.id, |task| {
        let tags = task["tags"].as_array_mut().expect("tags");
        tags.retain(|t| !t.as_str().is_some_and(|t| t.starts_with("number:")));
        tags.push(json!("number:1"));
    });
    let report = migrate_to_file(&source, &path).expect("retry");
    assert!(report.exact(), "{:?}", report.mismatches);
    assert_eq!(
        SqliteStore::open(&path)
            .expect("open")
            .list()
            .expect("list")
            .len(),
        2
    );
    assert!(!dir.join("board.db.partial").exists());
    std::fs::remove_dir_all(dir).expect("cleanup");
}

#[test]
fn an_existing_destination_is_never_touched() {
    let dir = dir("exists");
    let path = dir.join("board.db");
    std::fs::write(&path, "precious").expect("write");
    let (_tick, source) = seeded();
    let err = migrate_to_file(&source, &path).expect_err("refused");
    assert!(err.contains("already exists"), "{err}");
    assert_eq!(std::fs::read_to_string(&path).expect("read"), "precious");
    std::fs::remove_dir_all(dir).expect("cleanup");
}

#[test]
fn an_unreadable_board_or_comment_document_is_reported_and_nothing_is_written() {
    for (kind, text) in [("board", "board document"), ("comment", "comment document")] {
        let source = TickStore::new(Altered {
            tick: seeded().0,
            alter: if kind == "board" {
                |path, body| {
                    if path.starts_with("/docs/board") && !path.starts_with("/docs/board_number") {
                        body["docs"].as_array_mut().expect("docs").push(
                            json!({"id": "00000000-0000-4000-8000-000000000001", "body": {"deps": ["not-a-uuid"]}}),
                        );
                    }
                }
            } else {
                |path, body| {
                    if path.starts_with("/docs/comment") {
                        body["docs"].as_array_mut().expect("docs").push(
                            json!({"id": "00000000-0000-4000-8000-000000000002", "body": {"taskId": "nope"}}),
                        );
                    }
                }
            },
        });
        let target = SqliteStore::open_in_memory().expect("open");
        let report = migrate(&source, &target).expect("report");
        assert!(
            report.mismatches.iter().any(|m| m.contains(text)),
            "{kind}: {:?}",
            report.mismatches
        );
        assert!(
            target.list().expect("list").is_empty(),
            "{kind}: nothing written"
        );
    }
}
