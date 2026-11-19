//! The copy from rusty_tick into SQLite (ADR-0007): exact on a fully used board, read-only
//! towards rusty_tick, and loud when the copy cannot be right.

use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::domain::{BoardError, Card, Event, EventKind};
use rusty_mill_board::domain::{Column, Span};
use rusty_mill_board::migrate::{migrate, migrate_to_file};
use rusty_mill_board::milestones::{Milestone, MilestoneStore};
use rusty_mill_board::sqlite::{Import, SqliteStore};
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
    assert_eq!(
        std::fs::read_dir(&dir).expect("dir").count(),
        1,
        "no staging left behind"
    );
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

/// rusty_tick that creates `path` the first time the move history is read: someone else
/// creating the destination while the copy runs.
struct Intruder {
    tick: FakeTick,
    path: std::path::PathBuf,
}

impl TickApi for Intruder {
    fn send(&self, r: &TickRequest<'_>) -> Result<(u16, serde_json::Value), BoardError> {
        if r.path.starts_with("/moves?") && !self.path.exists() {
            std::fs::write(&self.path, "someone else's").expect("write");
        }
        self.tick.send(r)
    }
}

#[test]
fn a_destination_created_during_the_copy_is_never_replaced() {
    let dir = dir("race");
    let path = dir.join("board.db");
    let source = TickStore::new(Intruder {
        tick: seeded().0,
        path: path.clone(),
    });
    let err = migrate_to_file(&source, &path).expect_err("refused");
    assert!(err.contains("already exists"), "{err}");
    assert_eq!(
        std::fs::read_to_string(&path).expect("read"),
        "someone else's"
    );
    assert_eq!(
        std::fs::read_dir(&dir).expect("dir").count(),
        1,
        "no staging left behind"
    );
    std::fs::remove_dir_all(dir).expect("cleanup");
}

#[test]
fn a_run_removes_only_its_own_staging_files() {
    let dir = dir("own");
    let path = dir.join("board.db");
    let other = dir.join("board.db.otherrun.partial");
    std::fs::write(&other, "another run's copy").expect("write");
    let (_tick, source) = seeded();
    assert!(migrate_to_file(&source, &path).expect("migrate").exact());
    assert_eq!(
        std::fs::read_to_string(&other).expect("read"),
        "another run's copy"
    );
    assert_eq!(
        std::fs::read_dir(&dir).expect("dir").count(),
        2,
        "the copy and the other run's file"
    );
    std::fs::remove_dir_all(dir).expect("cleanup");
}

#[test]
fn the_published_file_alone_is_the_whole_database() {
    let dir = dir("alone");
    let path = dir.join("board.db");
    let (_tick, source) = seeded();
    assert!(migrate_to_file(&source, &path).expect("migrate").exact());
    // Only the main file travels: no log beside it.
    let elsewhere = dir.join("moved");
    std::fs::create_dir_all(&elsewhere).expect("dir");
    std::fs::copy(&path, elsewhere.join("board.db")).expect("copy");
    let copy = SqliteStore::open(elsewhere.join("board.db")).expect("open");
    assert_eq!(copy.list(), source.list());
    assert_eq!(copy.moves().expect("moves").len(), 3);
    assert!(
        SqliteStore::open_in_memory()
            .expect("open")
            .checkpoint()
            .is_ok()
    );
    drop(copy); // Windows will not delete an open database
    std::fs::remove_dir_all(dir).expect("cleanup");
}

#[test]
fn verification_compares_every_move_and_reservation_not_just_who_moved() {
    use rusty_mill_board::migrate::verify;
    use rusty_mill_board::sqlite::Import;
    use rusty_sqlite::rusqlite::Connection;
    let (_tick, source) = seeded();
    let cards = source.list().expect("list");
    let (a, b) = (cards[0].id, cards[1].id);
    let moves = [(a, 100), (a, 200), (b, 150)];
    let numbers = [(1, a), (2, b)];
    let expected = Import {
        cards: &cards,
        numbers: &numbers,
        moves: &moves,
        milestones: &[],
        source: &[],
    };
    let corruptions = [
        "DELETE FROM moves WHERE card = '{a}' AND at_ms = 200",
        "UPDATE moves SET at_ms = at_ms + 1 WHERE card = '{b}'",
        "DELETE FROM numbers WHERE number = 1",
        "UPDATE numbers SET card = '{b}' WHERE number = 1",
    ];
    for sql in corruptions {
        let dir = dir("verify");
        let path = dir.join("board.db");
        let target = SqliteStore::open(&path).expect("open");
        target.import(&expected).expect("import");
        assert_eq!(verify(&expected, &target), Ok(vec![]), "a faithful copy");
        drop(target);
        let raw = Connection::open(&path).expect("raw");
        raw.execute(
            &sql.replace("{a}", &a.to_string())
                .replace("{b}", &b.to_string()),
            [],
        )
        .expect("corrupt");
        drop(raw);
        let target = SqliteStore::open(&path).expect("reopen");
        let found = verify(&expected, &target).expect("verify");
        assert_eq!(found.len(), 1, "{sql}: {found:?}");
        drop(target); // Windows will not delete an open database
        std::fs::remove_dir_all(dir).expect("cleanup");
    }
}

#[test]
fn a_milestone_task_with_an_unreadable_version_is_reported_not_dropped() {
    let source = altered(|path, body| {
        if path.starts_with("/tags/milestone/tasks") {
            body["tasks"].as_array_mut().expect("tasks").push(json!({
                "id": "00000000-0000-4000-8000-000000000003", "title": "bad version",
                "tags": ["milestone"], "status": "open", "etag": "x", "dueMs": 5,
                "listId": "00000000-0000-0000-0000-000000000000", "notes": "",
                "priority": 0, "sortOrder": 0, "createdMs": 1, "updatedMs": 1,
                "deletedMs": null
            }));
        }
    });
    let target = SqliteStore::open_in_memory().expect("open");
    let report = migrate(&source, &target).expect("report");
    assert!(
        report
            .mismatches
            .iter()
            .any(|m| m.contains("milestone task")),
        "{:?}",
        report.mismatches
    );
    assert!(target.list().expect("list").is_empty(), "nothing written");
}

#[test]
fn a_log_left_beside_a_missing_destination_is_refused() {
    for sidecar in ["board.db-wal", "board.db-shm", "board.db-journal"] {
        let dir = dir("stray");
        let path = dir.join("board.db");
        std::fs::write(dir.join(sidecar), "stale").expect("write");
        let (_tick, source) = seeded();
        let err = migrate_to_file(&source, &path).expect_err("refused");
        assert!(err.contains(sidecar), "{err}");
        assert!(!path.exists());
        assert_eq!(
            std::fs::read_to_string(dir.join(sidecar)).expect("read"),
            "stale"
        );
        std::fs::remove_dir_all(dir).expect("cleanup");
    }
}

#[test]
fn an_edit_that_keeps_the_column_adds_no_move_to_a_migrated_card() {
    let (_tick, source) = seeded();
    let target = SqliteStore::open_in_memory().expect("open");
    migrate(&source, &target).expect("migrate");
    let before = target.moves().expect("moves").len();
    let human = Roles::default().actor("nano", "human").expect("role");
    let card = target.list().expect("list")[0].clone();
    let edited = card.comment(&human, "later", 9_999).expect("comment");
    target
        .replace(&edited, Event::of(&edited, EventKind::Commented, "nano"))
        .expect("replace");
    assert_eq!(
        target.moves().expect("moves").len(),
        before,
        "no new column entry"
    );
    // A real move still records one.
    let moved = Card {
        rev: edited.rev + 1,
        column: Column::DONE,
        ..edited.clone()
    };
    target
        .replace(&moved, Event::of(&moved, EventKind::Moved, "nano"))
        .expect("replace");
    assert_eq!(target.moves().expect("moves").len(), before + 1);
}

/// rusty_tick that creates `file` the first time the move history is read.
struct SidecarDuring {
    tick: FakeTick,
    file: std::path::PathBuf,
}

impl TickApi for SidecarDuring {
    fn send(&self, r: &TickRequest<'_>) -> Result<(u16, serde_json::Value), BoardError> {
        if r.path.starts_with("/moves?") && !self.file.exists() {
            std::fs::write(&self.file, "stale").expect("write");
        }
        self.tick.send(r)
    }
}

#[test]
fn a_sidecar_created_during_the_copy_stops_publication_and_is_left_alone() {
    for sidecar in ["board.db-wal", "board.db-shm", "board.db-journal"] {
        let dir = dir("sidecar_race");
        let path = dir.join("board.db");
        let source = TickStore::new(SidecarDuring {
            tick: seeded().0,
            file: dir.join(sidecar),
        });
        let err = migrate_to_file(&source, &path).expect_err("refused");
        assert!(err.contains(sidecar), "{err}");
        assert!(!path.exists(), "nothing was published");
        assert_eq!(
            std::fs::read_to_string(dir.join(sidecar)).expect("read"),
            "stale"
        );
        assert_eq!(
            std::fs::read_dir(&dir).expect("dir").count(),
            1,
            "staging removed"
        );
        std::fs::remove_dir_all(dir).expect("cleanup");
    }
}

/// rusty_tick with one more member on a card's board document, as a newer board would write it.
struct WithExtra {
    tick: FakeTick,
    card: uuid::Uuid,
}

impl TickApi for WithExtra {
    fn send(&self, r: &TickRequest<'_>) -> Result<(u16, serde_json::Value), BoardError> {
        let (status, mut body) = self.tick.send(r)?;
        if r.path.starts_with("/docs/board") && !r.path.starts_with("/docs/board_number") {
            let docs = body["docs"].as_array_mut().expect("docs");
            docs.retain(|d| d["id"] != json!(self.card));
            // `title` is also a member of the card itself.
            docs.push(json!({"id": self.card, "body": {
                "v": 1, "prompt": "p", "futureField": {"a": [1, 2]}, "title": "tick's own"
            }}));
        }
        Ok((status, body))
    }
}

#[test]
fn source_only_data_is_kept_apart_from_the_card_and_survives_an_edit() {
    use rusty_mill_board::sqlite::SOURCE_KEY;
    let (tick, plain) = seeded();
    let first = plain.list().expect("list")[0].clone();
    let source = TickStore::new(WithExtra {
        tick,
        card: first.id,
    });
    let target = SqliteStore::open_in_memory().expect("open");
    let report = migrate(&source, &target).expect("migrate");
    assert!(report.exact(), "{:?}", report.mismatches);
    assert_eq!(report.unknown_fields, 2);
    let doc = target.document(first.id).expect("doc").expect("some");
    let kept = &doc[SOURCE_KEY];
    assert_eq!(kept["unknown_fields"]["futureField"], json!({"a": [1, 2]}));
    assert_eq!(
        kept["unknown_fields"]["title"],
        json!("tick's own"),
        "a colliding key too"
    );
    assert_eq!(
        doc["title"],
        json!(first.title),
        "the card's own title is the card's"
    );
    assert_eq!(
        kept["comment_ids"].as_array().map(Vec::len),
        Some(first.comments.len())
    );
    // An edit writes the card back and all of it is still there.
    let card = target.get(first.id).expect("get").expect("card");
    let edited = Card {
        rev: card.rev + 1,
        title: "edited".into(),
        ..card
    };
    target
        .replace(&edited, Event::of(&edited, EventKind::Edited, "x"))
        .expect("replace");
    let after = target.document(first.id).expect("doc").expect("some");
    assert_eq!(after[SOURCE_KEY], *kept);
    assert_eq!(after["title"], json!("edited"));
}

#[test]
fn verification_notices_lost_source_data() {
    use rusty_mill_board::migrate::verify;
    let (_tick, source) = seeded();
    let cards = source.list().expect("list");
    let held = serde_json::Map::from_iter([("comment_ids".to_string(), json!(["x"]))]);
    let found = verify(
        &Import {
            cards: &cards,
            numbers: &[],
            moves: &[],
            milestones: &[],
            source: &[(cards[0].id, held)],
        },
        &{
            let target = SqliteStore::open_in_memory().expect("open");
            target
                .import(&Import {
                    cards: &cards,
                    numbers: &[],
                    moves: &[],
                    milestones: &[],
                    source: &[],
                })
                .expect("import");
            target
        },
    )
    .expect("verify");
    assert!(
        found
            .iter()
            .any(|m| m.contains("lost what the source held")),
        "{found:?}"
    );
}

#[test]
fn typed_fields_are_copied_from_tick_into_sqlite() {
    use rusty_mill_board::domain::Edit;
    let tick = FakeTick::default();
    let board = Board::new(TickStore::new(tick.clone()), Roles::default());
    let source = TickStore::new(tick);
    let human = board.roles().actor("nano", "human").expect("role");
    let card = board.add(&human, "typed", "", vec![]).expect("add");
    let set = Edit {
        acceptance: Some("a"),
        scope: Some("s"),
        why: Some("w"),
        summary: Some("m"),
        ..Edit::default()
    };
    board.edit_at(card.id, &human, None, &set).expect("set");
    let target = SqliteStore::open_in_memory().expect("open");
    let report = migrate(&source, &target).expect("migrate");
    assert!(report.exact(), "{:?}", report.mismatches);
    let copied = target.get(card.id).expect("get").expect("card");
    assert_eq!(
        [
            &copied.acceptance,
            &copied.scope,
            &copied.why,
            &copied.summary
        ],
        ["a", "s", "w", "m"]
    );
}
