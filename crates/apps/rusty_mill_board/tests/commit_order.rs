//! Commit atomicity and event ordering, for every store adapter.
//!
//! The event id is assigned inside the commit, so a reader that has seen id N has
//! seen everything committed before it: no event can appear *behind* a cursor.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;

use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::domain::{BoardError, Card, Event, EventKind};
use rusty_mill_board::mmdb::MmdbStore;
use rusty_mill_board::store::{MemStore, Store};
use uuid::Uuid;

const WRITERS: usize = 8;
const PER_WRITER: usize = 40;

fn temp_store(name: &str) -> (MmdbStore, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("mill_order_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    (MmdbStore::open(&dir.join("b.mmap")).expect("open"), dir)
}

/// Writers commit concurrently while a reader polls with a cursor. The reader must
/// see every event exactly once, in increasing id order, whatever the interleaving.
fn no_event_is_skipped<S: Store + 'static>(store: S) {
    let board = Arc::new(Board::new(store, Roles::default()));
    let human = board.roles().actor("nano", "human").expect("role");
    let start = Arc::new(Barrier::new(WRITERS + 1));
    let done = Arc::new(AtomicBool::new(false));

    let writers: Vec<_> = (0..WRITERS)
        .map(|w| {
            let (board, human, start) = (Arc::clone(&board), human.clone(), Arc::clone(&start));
            thread::spawn(move || {
                start.wait();
                for i in 0..PER_WRITER {
                    board
                        .add(&human, &format!("w{w}-{i}"), "", vec![])
                        .expect("add");
                }
            })
        })
        .collect();

    let reader = {
        let (board, start, done) = (Arc::clone(&board), Arc::clone(&start), Arc::clone(&done));
        thread::spawn(move || {
            start.wait();
            let (mut cursor, mut seen) = (None::<Uuid>, Vec::new());
            loop {
                // Read the flag first: if the writers had finished before this poll and
                // it comes back empty, everything committed has been delivered.
                let finished = done.load(Ordering::SeqCst);
                let batch = board.events(cursor, 7).expect("events");
                for e in &batch {
                    assert!(
                        cursor.is_none_or(|c| e.id > c),
                        "ids must increase past the cursor"
                    );
                    cursor = Some(e.id);
                    seen.push(e.id);
                }
                if finished && batch.is_empty() {
                    break;
                }
            }
            seen
        })
    };

    for w in writers {
        w.join().expect("writer");
    }
    done.store(true, Ordering::SeqCst);
    let seen = reader.join().expect("reader");

    let all: Vec<Uuid> = board
        .events(None, usize::MAX)
        .expect("all")
        .iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(
        all.len(),
        WRITERS * PER_WRITER,
        "every add committed one event"
    );
    assert!(
        all.windows(2).all(|p| p[0] < p[1]),
        "the log is in strictly increasing id order"
    );
    let uniq: HashSet<_> = seen.iter().collect();
    assert_eq!(uniq.len(), seen.len(), "no event delivered twice");
    assert_eq!(
        seen, all,
        "the polling reader saw every event, none skipped, in order"
    );
}

#[test]
fn mem_store_never_skips_an_event() {
    no_event_is_skipped(MemStore::default());
}

#[test]
fn mmdb_store_never_skips_an_event() {
    let (store, dir) = temp_store("skip");
    no_event_is_skipped(store);
    let _ = std::fs::remove_dir_all(dir);
}

/// A store that fails the Nth commit, standing in for a disk error.
struct FailOnCommit<S> {
    inner: S,
    fail_at: usize,
    count: std::sync::atomic::AtomicUsize,
}

impl<S: Store> FailOnCommit<S> {
    fn check(&self) -> Result<(), BoardError> {
        if self.count.fetch_add(1, Ordering::SeqCst) + 1 == self.fail_at {
            return Err(BoardError::Storage("injected write failure".into()));
        }
        Ok(())
    }
}

impl<S: Store> Store for FailOnCommit<S> {
    fn get(&self, id: Uuid) -> Result<Option<Card>, BoardError> {
        self.inner.get(id)
    }
    fn list(&self) -> Result<Vec<Card>, BoardError> {
        self.inner.list()
    }
    fn insert(&self, card: &Card, event: Event) -> Result<u64, BoardError> {
        self.check()?;
        self.inner.insert(card, event)
    }
    fn replace(&self, card: &Card, event: Event) -> Result<u64, BoardError> {
        self.check()?;
        self.inner.replace(card, event)
    }
    fn events_since(&self, after: Option<Uuid>, limit: usize) -> Result<Vec<Event>, BoardError> {
        self.inner.events_since(after, limit)
    }
    fn last_event_id(&self) -> Result<Option<Uuid>, BoardError> {
        self.inner.last_event_id()
    }
}

#[test]
fn a_failed_commit_changes_neither_the_card_nor_the_log() {
    let board = Board::new(
        FailOnCommit {
            inner: MemStore::default(),
            fail_at: 2,
            count: 0.into(),
        },
        Roles::default(),
    );
    let human = board.roles().actor("nano", "human").expect("role");
    let card = board
        .add(&human, "kept", "", vec![])
        .expect("first commit succeeds");
    let before = board.events(None, 100).expect("events");

    let err = board
        .comment(card.id, &human, "lost")
        .expect_err("second commit fails");
    assert_eq!(err, BoardError::Storage("injected write failure".into()));
    assert_eq!(
        board.get(card.id).expect("card"),
        card,
        "the card is exactly as it was"
    );
    assert_eq!(
        board.events(None, 100).expect("events"),
        before,
        "no event was logged"
    );

    // The board is still usable: the retry commits at the same revision.
    let ok = board.comment(card.id, &human, "again").expect("retry");
    assert_eq!(ok.rev, card.rev + 1);
}

#[test]
fn state_is_rebuilt_from_the_single_commit_records_after_a_restart() {
    let (store, dir) = temp_store("restart");
    let board = Board::new(store, Roles::default());
    let human = board.roles().actor("nano", "human").expect("role");
    let a = board.add(&human, "a", "", vec![]).expect("add");
    let b = board.add(&human, "b", "", vec![]).expect("add");
    board.comment(a.id, &human, "note").expect("comment");
    board.link_issue(b.id, 7, None, &human).expect("link");
    let (cards, events) = (
        board.list(None).expect("list"),
        board.events(None, 100).expect("events"),
    );
    assert_eq!(
        events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        [
            EventKind::Created,
            EventKind::Created,
            EventKind::Commented,
            EventKind::Linked
        ]
    );
    drop(board);

    let reopened = Board::new(
        MmdbStore::open(&dir.join("b.mmap")).expect("reopen"),
        Roles::default(),
    );
    assert_eq!(
        reopened.list(None).expect("list"),
        cards,
        "cards come back exactly"
    );
    assert_eq!(
        reopened.events(None, 100).expect("events"),
        events,
        "the log comes back exactly, in order"
    );
    // New commits continue the id order after a restart.
    let c = reopened.add(&human, "c", "", vec![]).expect("add");
    let tail = reopened
        .events(events.last().map(|e| e.id), 10)
        .expect("tail");
    assert_eq!(tail.len(), 1);
    assert_eq!(tail[0].card, c.id);
    assert!(tail[0].id > events.last().expect("last").id);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn stale_and_duplicate_commits_are_refused_without_effect() {
    let store = MemStore::default();
    let card = Card::new("a", "", vec![], 0).expect("card");
    store
        .insert(&card, Event::of(&card, EventKind::Created, "x"))
        .expect("insert");
    assert_eq!(
        store.insert(&card, Event::of(&card, EventKind::Created, "x")),
        Err(BoardError::Conflict),
        "duplicate id"
    );
    let skipped = card.link(1, None, 1).link(2, None, 2); // rev 2, but the store is at rev 0
    assert_eq!(
        store.replace(&skipped, Event::of(&skipped, EventKind::Linked, "x")),
        Err(BoardError::Conflict)
    );
    assert_eq!(
        store.events_since(None, 10).expect("events").len(),
        1,
        "refused commits log nothing"
    );
}
