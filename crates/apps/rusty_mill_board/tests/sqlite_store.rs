//! What only the SQLite adapter promises: a durable log and numbers, one atomic commit per
//! write, one winner among concurrent writers, and the same answers as the in-memory store.

use std::sync::Arc;

use rusty_mill_board::domain::{BoardError, Card, Column, Event, EventKind, Span};
use rusty_mill_board::milestones::{Milestone, MilestoneStore};
use rusty_mill_board::sqlite::SqliteStore;
use rusty_mill_board::store::{MemStore, Store};
use rusty_sqlite::rusqlite::Connection;
use uuid::Uuid;

fn file(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mill_sqlite_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    dir.join("board.db")
}

fn card(title: &str, at: i64) -> Card {
    Card::new(title, "", vec![], at).expect("card")
}

/// `card` one revision on, in `column`, at `at`.
fn moved(card: &Card, column: Column, at: i64) -> Card {
    Card {
        rev: card.rev + 1,
        column,
        updated_ms: at,
        ..card.clone()
    }
}

fn add<S: Store + ?Sized>(store: &S, card: &Card) {
    store
        .insert(card, Event::of(card, EventKind::Created, "t"))
        .expect("insert");
}

#[test]
fn events_numbers_and_cards_survive_a_reopen() {
    let path = file("reopen");
    let a = card("a", 1);
    let last = {
        let store = SqliteStore::open(&path).expect("open");
        add(&store, &a);
        assert_eq!(store.reserve_number(a.id, 0), Ok(1));
        store.last_event_id().expect("last").expect("an event")
    };
    let store = SqliteStore::open(&path).expect("reopen");
    assert_eq!(store.get(a.id), Ok(Some(a.clone())));
    assert_eq!(store.last_event_id(), Ok(Some(last)), "the log is durable");
    assert_eq!(
        store.holds_event(last),
        Ok(true),
        "a cursor from before a restart is still held"
    );
    assert_eq!(store.highest_reserved(), Ok(1));
    assert_eq!(
        store.reserve_number(a.id, 0),
        Ok(2),
        "a reserved number is never reused"
    );
    assert!(store.durable());
    let _ = std::fs::remove_dir_all(path.parent().expect("dir"));
}

#[test]
fn a_failed_event_write_leaves_no_card() {
    let path = file("atomic");
    let store = SqliteStore::open(&path).expect("open");
    // Another connection makes every event insert fail, as a full disk would.
    Connection::open(&path)
        .expect("second")
        .execute_batch(
            "CREATE TRIGGER no_events BEFORE INSERT ON events BEGIN SELECT RAISE(ABORT, 'disk full'); END;",
        )
        .expect("trigger");
    let a = card("a", 1);
    let out = store.insert(&a, Event::of(&a, EventKind::Created, "t"));
    assert!(matches!(out, Err(BoardError::Storage(_))), "{out:?}");
    assert_eq!(
        store.get(a.id),
        Ok(None),
        "the card rolled back with its event"
    );
    let _ = std::fs::remove_dir_all(path.parent().expect("dir"));
}

#[test]
fn concurrent_replaces_of_one_revision_have_exactly_one_winner() {
    let store = Arc::new(SqliteStore::open_in_memory().expect("open"));
    let a = card("a", 1);
    add(&*store, &a);
    let handles: Vec<_> = (0..8)
        .map(|i| {
            let (store, a) = (Arc::clone(&store), a.clone());
            std::thread::spawn(move || {
                let next = a.edit(Some(&format!("t{i}")), None, None, 2).expect("edit");
                store.replace(&next, Event::of(&next, EventKind::Edited, "t"))
            })
        })
        .collect();
    let results: Vec<_> = handles
        .into_iter()
        .map(|h| h.join().expect("join"))
        .collect();
    assert_eq!(
        results.iter().filter(|r| r.is_ok()).count(),
        1,
        "{results:?}"
    );
    assert!(
        results
            .iter()
            .filter(|r| r.is_err())
            .all(|r| *r == Err(BoardError::Conflict))
    );
}

#[test]
fn a_missing_card_is_not_found_and_a_duplicate_conflicts() {
    let store = SqliteStore::open_in_memory().expect("open");
    let a = card("a", 1);
    let next = a.edit(Some("b"), None, None, 2).expect("edit");
    assert_eq!(
        store.replace(&next, Event::of(&next, EventKind::Edited, "t")),
        Err(BoardError::NotFound(a.id))
    );
    add(&store, &a);
    assert_eq!(
        store.insert(&a, Event::of(&a, EventKind::Created, "t")),
        Err(BoardError::Conflict)
    );
}

#[test]
fn the_queries_answer_as_the_in_memory_store_does() {
    let (sql, mem) = (
        SqliteStore::open_in_memory().expect("open"),
        MemStore::default(),
    );
    let cards: Vec<Card> = (0..5).map(|i| card(&format!("c{i}"), 10 + i)).collect();
    for s in [&sql as &dyn Store, &mem] {
        for c in &cards {
            add(s, c);
        }
        let now = moved(&cards[1], Column::READY, 100);
        s.replace(&now, Event::of(&now, EventKind::Moved, "t"))
            .expect("move");
        let done = moved(&cards[2], Column::DONE, 200);
        s.replace(&done, Event::of(&done, EventKind::Moved, "t"))
            .expect("done");
    }
    let strip = |v: Vec<Event>| {
        v.into_iter()
            .map(|e| (e.card, e.kind, e.rev, e.column))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        strip(sql.events_since(None, 100).expect("sql")),
        strip(mem.events_since(None, 100).expect("mem"))
    );
    let ids = |v: Vec<Card>| {
        v.into_iter()
            .map(|c| (c.id, c.rev, c.column))
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(sql.list().expect("sql")), ids(mem.list().expect("mem")));
    let cols = [Column::READY, Column::DONE];
    assert_eq!(
        ids(sql.in_columns(&cols).expect("sql")),
        ids(mem.in_columns(&cols).expect("mem"))
    );
    let all: Vec<Uuid> = cards.iter().map(|c| c.id).chain([Uuid::new_v4()]).collect();
    assert_eq!(sql.open_among(&all), mem.open_among(&all));
    let spans = [
        Span {
            from: None,
            to: None,
        },
        Span {
            from: Some(100),
            to: None,
        },
        Span {
            from: Some(150),
            to: Some(300),
        },
        Span {
            from: Some(0),
            to: Some(20),
        },
    ];
    for span in spans {
        assert_eq!(sql.moved_in(span), mem.moved_in(span), "{span:?}");
    }
    assert_eq!(sql.numbered(1), mem.numbered(1));
    let after = sql.events_since(None, 3).expect("sql")[2].id;
    assert_eq!(sql.events_since(Some(after), 100).expect("sql").len(), 4);
}

#[test]
fn milestones_save_by_compare_and_swap() {
    let store = SqliteStore::open_in_memory().expect("open");
    let m = Milestone::new("rl", "Beta", 10).expect("milestone");
    assert_eq!(store.add_milestone(&m), Ok(1));
    assert_eq!(store.add_milestone(&m), Err(BoardError::Conflict));
    let read = store.milestone(m.id).expect("read").expect("there");
    let next = Milestone {
        title: "Beta 2".into(),
        done_ms: Some(20),
        ..read.clone()
    };
    assert_eq!(store.save_milestone(&read, &next), Ok(2));
    assert_eq!(
        store.save_milestone(&read, &next),
        Err(BoardError::Conflict),
        "stale"
    );
    let gone = Milestone {
        id: Uuid::new_v4(),
        ..read.clone()
    };
    assert_eq!(
        store.save_milestone(&gone, &next),
        Err(BoardError::NotFound(gone.id))
    );
    let now = store.milestones().expect("all");
    assert_eq!(
        (now[0].title.as_str(), now[0].done_ms, now[0].rev),
        ("Beta 2", Some(20), 2)
    );
}

#[test]
fn a_newer_schema_is_refused_not_downgraded() {
    let path = file("newer");
    drop(SqliteStore::open(&path).expect("open"));
    Connection::open(&path)
        .expect("raw")
        .pragma_update(None, "user_version", 99)
        .expect("version");
    assert!(matches!(
        SqliteStore::open(&path),
        Err(BoardError::Storage(_))
    ));
    let _ = std::fs::remove_dir_all(path.parent().expect("dir"));
}
