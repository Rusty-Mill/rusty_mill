//! What only the SQLite adapter promises: a durable log and numbers, one atomic commit per
//! write, one winner among concurrent writers, and the same answers as the in-memory store.

use std::sync::Arc;

use rusty_mill_board::domain::{
    Actor, BoardError, Capability, Card, Column, Event, EventKind, MERGE_WATCH, Span,
};
use rusty_mill_board::milestones::{Milestone, MilestoneStore};
use rusty_mill_board::sqlite::SqliteStore;
use rusty_mill_board::store::{MemStore, Store};
use rusty_mill_board::workflow::Workflow;
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
fn a_milestone_at_the_last_revision_is_kept_not_advanced() {
    let path = file("lastrev");
    let store = SqliteStore::open(&path).expect("open");
    let m = Milestone::new("rl", "Beta", 10).expect("milestone");
    store.add_milestone(&m).expect("add");
    Connection::open(&path)
        .expect("raw")
        .execute("UPDATE milestones SET rev = ?1", [i64::MAX])
        .expect("set");
    let at = store.milestone(m.id).expect("read").expect("there");
    let next = Milestone {
        title: "Beta 2".into(),
        ..at.clone()
    };
    assert!(matches!(
        store.save_milestone(&at, &next),
        Err(BoardError::Storage(_))
    ));
    let kept = store.milestone(m.id).expect("read").expect("there");
    assert_eq!((kept.title.as_str(), kept.rev), ("Beta", i64::MAX as u64));
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

#[test]
fn a_replacement_must_be_exactly_one_revision_on_as_in_the_in_memory_store() {
    let (sql, mem) = (
        SqliteStore::open_in_memory().expect("open"),
        MemStore::default(),
    );
    let a = card("a", 1);
    for s in [&sql as &dyn Store, &mem] {
        add(s, &a);
        // Revision 0 has no predecessor: it must not overwrite the card at revision 0.
        let same = Card {
            title: "changed".into(),
            ..a.clone()
        };
        assert_eq!(
            s.replace(&same, Event::of(&same, EventKind::Edited, "t")),
            Err(BoardError::Conflict)
        );
        let skip = Card {
            rev: a.rev + 2,
            ..a.clone()
        };
        assert_eq!(
            s.replace(&skip, Event::of(&skip, EventKind::Edited, "t")),
            Err(BoardError::Conflict)
        );
        assert_eq!(s.get(a.id).expect("get").expect("there").title, "a");
        let ghost = card("ghost", 2);
        assert_eq!(
            s.replace(&ghost, Event::of(&ghost, EventKind::Edited, "t")),
            Err(BoardError::NotFound(ghost.id))
        );
    }
    assert_eq!(
        sql.events_since(None, 10).expect("sql").len(),
        1,
        "a refused write leaves no event"
    );
}

#[test]
fn a_number_beyond_sqlites_range_is_an_error_not_a_clamp() {
    let store = SqliteStore::open_in_memory().expect("open");
    assert!(matches!(
        store.numbered(u64::MAX),
        Err(BoardError::Storage(_))
    ));
    assert!(matches!(
        store.reserve_number(Uuid::nil(), u64::MAX),
        Err(BoardError::Storage(_))
    ));
    let a = card("a", 1);
    let far = Card {
        number: Some(u64::MAX),
        ..a.clone()
    };
    assert!(matches!(
        store.insert(&far, Event::of(&far, EventKind::Created, "t")),
        Err(BoardError::Storage(_))
    ));
    assert_eq!(store.get(a.id), Ok(None));
}

#[test]
fn an_unreadable_event_id_is_an_error_not_an_empty_log() {
    let path = file("badid");
    let store = SqliteStore::open(&path).expect("open");
    add(&store, &card("a", 1));
    Connection::open(&path)
        .expect("raw")
        .execute("UPDATE events SET id = 'zzz'", [])
        .expect("corrupt");
    assert!(matches!(store.last_event_id(), Err(BoardError::Storage(_))));
    assert!(matches!(
        store.insert(
            &card("b", 2),
            Event::of(&card("b", 2), EventKind::Created, "t")
        ),
        Err(BoardError::Storage(_))
    ));
}

#[test]
fn a_card_number_is_unique_and_milestones_come_back_in_id_order() {
    let store = SqliteStore::open_in_memory().expect("open");
    let numbered = |t: &str, at| Card {
        number: Some(1),
        ..card(t, at)
    };
    let (a, b) = (numbered("a", 1), numbered("b", 2));
    add(&store, &a);
    assert!(
        store
            .insert(&b, Event::of(&b, EventKind::Created, "t"))
            .is_err(),
        "two cards cannot share a number"
    );
    assert_eq!(store.get(b.id), Ok(None));
    let ms: Vec<_> = (0..4)
        .map(|i| Milestone::new("rl", &format!("m{i}"), i + 1).expect("milestone"))
        .collect();
    for m in &ms {
        store.add_milestone(m).expect("add");
    }
    let ids: Vec<_> = store
        .milestones()
        .expect("all")
        .iter()
        .map(|m| m.id)
        .collect();
    let mut sorted = ids.clone();
    sorted.sort_by_key(|i| i.to_string());
    assert_eq!(ids, sorted);
}

#[test]
fn moves_outlive_the_events_they_came_from() {
    let path = file("moves");
    let store = SqliteStore::open(&path).expect("open");
    let a = card("a", 100);
    add(&store, &a);
    let span = Span {
        from: Some(0),
        to: Some(1000),
    };
    assert_eq!(store.moved_in(span).expect("moved"), [a.id].into());
    Connection::open(&path)
        .expect("raw")
        .execute("DELETE FROM events", [])
        .expect("prune");
    assert_eq!(
        store.moved_in(span).expect("moved"),
        [a.id].into(),
        "pruned events leave the move history intact"
    );
}

#[test]
fn a_field_this_board_does_not_know_survives_a_replace() {
    let path = file("unknown");
    let store = SqliteStore::open(&path).expect("open");
    let a = card("a", 1);
    add(&store, &a);
    let raw = Connection::open(&path).expect("raw");
    raw.execute(
        "UPDATE cards SET doc = json_set(doc, '$.laterField', json('[{\"kind\":\"newer\"}]'))",
        [],
    )
    .expect("a newer board's field");
    let next = moved(&a, Column::DONE, 2);
    store
        .replace(&next, Event::of(&next, EventKind::Moved, "t"))
        .expect("replace");
    let doc: String = raw
        .query_row("SELECT doc FROM cards", [], |r| r.get(0))
        .expect("doc");
    let doc: serde_json::Value = serde_json::from_str(&doc).expect("json");
    assert_eq!(doc["laterField"][0]["kind"], "newer", "written back");
    assert_eq!(
        store.get(a.id).expect("get").expect("there").column,
        Column::DONE
    );
}

#[test]
fn a_merge_close_record_survives_a_fresh_read_and_unclose_clears_it() {
    let path = file("unclose");
    let store = SqliteStore::open(&path).expect("open");
    let flow = Workflow::default();
    let actor = |name: &str, caps: &[Capability]| Actor {
        name: name.into(),
        caps: caps.iter().copied().collect(),
        projects: None,
    };
    let watch = actor(MERGE_WATCH, &[Capability::Record]);
    let human = actor("nano", &[Capability::Triage, Capability::Close]);
    let pr = "https://github.com/o/r/pull/7";
    let ready = card("a", 1)
        .move_to(&flow, &Column::READY, &human, 2)
        .expect("ready");
    add(&store, &ready);
    let closed = ready.closed_by_merge(&flow, pr, &watch, 3).expect("closed");
    store
        .replace(&closed, Event::of(&closed, EventKind::Moved, MERGE_WATCH))
        .expect("replace");
    let reread = SqliteStore::open(&path).expect("reopen");
    let back = reread.get(ready.id).expect("get").expect("there");
    assert!(back.merged.is_some(), "the record is stored with the card");
    let open = back.unclosed(&flow, &human, 4).expect("unclosed");
    reread
        .replace(&open, Event::of(&open, EventKind::Moved, "nano"))
        .expect("replace");
    let fresh = SqliteStore::open(&path).expect("reopen");
    let end = fresh.get(ready.id).expect("get").expect("there");
    assert_eq!((end.column, end.merged), (Column::READY, None));
}

/// Every file in `path`'s directory with its bytes, to compare before and after a read.
fn snapshot(path: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(path.parent().expect("dir"))
        .expect("dir")
        .map(|e| e.expect("entry").path())
        .map(|p| {
            (
                p.file_name().expect("name").to_string_lossy().into_owned(),
                std::fs::read(&p).expect("read"),
            )
        })
        .collect()
}

#[test]
fn read_cards_reads_a_stopped_board_and_refuses_a_running_one_without_touching_files() {
    let path = file("read_only");
    let store = SqliteStore::open(&path).expect("open");
    add(&store, &card("a", 1));
    add(&store, &card("b", 2));
    // Running: the WAL sidecars exist, and even a read-only reader would update the `-shm`.
    let before = snapshot(&path);
    let err = rusty_mill_board::sqlite::read_cards(&path).expect_err("running");
    assert!(err.to_string().contains("stop the board"), "{err}");
    assert_eq!(snapshot(&path), before, "refusing changed a file");
    drop(store);
    // Stopped: the sidecars are gone, and reading leaves every file as it was.
    let before = snapshot(&path);
    let cards = rusty_mill_board::sqlite::read_cards(&path).expect("read");
    assert_eq!(
        cards.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert!(
        snapshot(&path) == before,
        "reading changed a file or added one"
    );
    assert_eq!(
        before.len(),
        1,
        "only the database file remains: {:?}",
        before.keys()
    );
}

#[test]
fn read_cards_refuses_a_missing_file_and_a_schema_that_is_not_this_builds() {
    let missing = file("read_missing");
    assert!(rusty_mill_board::sqlite::read_cards(&missing).is_err());
    assert!(!missing.exists(), "a missing file must not be created");

    for version in [0, 99] {
        let path = file(&format!("read_schema_{version}"));
        let conn = Connection::open(&path).expect("open");
        conn.pragma_update(None, "user_version", version)
            .expect("set");
        drop(conn);
        let before = snapshot(&path);
        let err = rusty_mill_board::sqlite::read_cards(&path).expect_err("schema");
        assert!(err.to_string().contains("schema"), "{err}");
        assert_eq!(snapshot(&path), before, "refusing changed a file");
    }
}

#[test]
fn read_cards_resolves_a_relative_path_once_and_refuses_a_rollback_journal() {
    let path = file("read_relative");
    {
        let store = SqliteStore::open(&path).expect("open");
        add(&store, &card("a", 1));
    }
    // The same file by a relative path: the sidecar checks and the open must agree on it.
    let dir = path.parent().expect("dir");
    let relative = std::path::Path::new("board.db");
    let cwd = std::env::current_dir().expect("cwd");
    std::env::set_current_dir(dir).expect("cd");
    let read = rusty_mill_board::sqlite::read_cards(relative);
    let sidecar = std::path::Path::new("board.db-journal");
    std::fs::write(sidecar, b"hot").expect("journal");
    let refused = rusty_mill_board::sqlite::read_cards(relative);
    std::env::set_current_dir(cwd).expect("back");
    assert_eq!(read.expect("relative read").len(), 1);
    assert!(
        refused
            .expect_err("journal")
            .to_string()
            .contains("-journal")
    );
}
