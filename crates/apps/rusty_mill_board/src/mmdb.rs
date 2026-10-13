//! Durable store on `rusty_multimodal_db_engine`.
//!
//! **One durable record per commit.** Each commit is a single [`Row`] holding the
//! event *and* the card as it is after the event, so there is no moment at which the
//! card is saved but its event is not (or the reverse): the engine's single-record
//! insert is the atomic step. The board's state (cards, event log) lives in memory
//! and is rebuilt from the records at open, in event-id order. The commit lock
//! covers the revision check, the event id assignment, the durable write and the
//! in-memory update, so ids are in commit order and a failed write changes nothing.
//! The cost is that the whole log is held in memory, which suits a board.

use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use rusty_multimodal_db_engine::generic::mmap_store::GenericMmapStore;
use rusty_multimodal_db_engine::generic::production::GenericProductionStore;
use rusty_multimodal_db_engine::generic::traits::{
    IndexedField, Record, ScannableField, SchemaTag,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::{BoardError, Card, Event};
use crate::store::{Log, Store};

const COMMIT: &str = "commit";

/// What a commit stores: the event and the card after it.
#[derive(Serialize, Deserialize)]
struct Commit {
    event: Event,
    card: Card,
}

#[derive(Clone, Serialize, Deserialize)]
struct Row {
    id: Uuid,
    kind: String,
    rev: i64,
    json: String,
}

struct Kind;
struct Rev;

impl Record for Row {
    type Id = Uuid;
    fn id(&self) -> Uuid {
        self.id
    }
}

// On-disk format name: changing the row layout means bumping the suffix.
// `@2`: one `commit` record (event + card) replaced separate card and event records.
impl SchemaTag for Row {
    const SCHEMA_TAG: &'static str = "mill_board::Row@2";
}

impl IndexedField<Kind> for Row {
    type IndexValue = String;
    fn indexed_value(&self) -> &String {
        &self.kind
    }
}

impl ScannableField<Rev> for Row {
    type ScanValue = i64;
    fn scannable_value(&self) -> i64 {
        self.rev
    }
    fn set_scannable_value(&mut self, value: i64) {
        self.rev = value;
    }
}

type Db = GenericProductionStore<GenericMmapStore<Row, Kind, Rev>>;

/// A [`Store`] persisted at a path (two files: `<path>` and `<path>.records`).
///
/// A failed durable append is *ambiguous*: the record may or may not have reached the log
/// (the engine can fail after writing it). The store cannot tell, so it stops accepting
/// writes (`BoardError::Storage`) rather than guess; reads keep serving the last state it
/// knows. Reopening replays the log and settles the question either way.
pub struct MmdbStore {
    db: Db,
    log: Mutex<Log>,
    /// Set by a failed append; cleared only by reopening.
    unsettled: AtomicBool,
}

fn storage(e: impl std::fmt::Display) -> BoardError {
    BoardError::Storage(e.to_string())
}

fn poisoned<T>(_: T) -> BoardError {
    BoardError::Storage("lock poisoned".into())
}

impl MmdbStore {
    /// Open the store at `path`, creating it if absent, and rebuild the
    /// board's state from its commit records.
    ///
    /// # Errors
    /// If the files cannot be created or read back, or a record does not decode.
    pub fn open(path: &Path) -> Result<Self, BoardError> {
        let core = if path.exists() {
            GenericMmapStore::open_portable(path)
        } else {
            GenericMmapStore::create(Vec::new(), path)
        }
        .map_err(storage)?;
        let db = GenericProductionStore::new(core);

        let mut commits = db
            .filter_eq::<Row, Kind>(&COMMIT.to_owned())
            .into_iter()
            .filter_map(|id| db.get::<Row>(id))
            .map(|row| serde_json::from_str::<Commit>(&row.json).map_err(storage))
            .collect::<Result<Vec<_>, _>>()?;
        commits.sort_by_key(|c| c.event.id);
        let mut log = Log::default();
        for c in commits {
            log.replay(c.card, c.event);
        }
        Ok(Self {
            db,
            log: Mutex::new(log),
            unsettled: AtomicBool::new(false),
        })
    }

    fn commit(&self, card: &Card, event: Event, is_insert: bool) -> Result<(), BoardError> {
        let mut log = self.log.lock().map_err(poisoned)?;
        if self.unsettled.load(Ordering::SeqCst) {
            return Err(storage(
                "an earlier write failed with an unknown outcome; reopen the store to settle it",
            ));
        }
        log.check(card, is_insert)?;
        let event = log.stamp(event);
        let row = Row {
            id: event.id,
            kind: COMMIT.to_owned(),
            rev: i64::try_from(card.rev).map_err(storage)?,
            json: serde_json::to_string(&Commit {
                event: event.clone(),
                card: card.clone(),
            })
            .map_err(storage)?,
        };
        // The one durable step. An error here does not prove nothing was written.
        if let Err(e) = self.db.insert(row) {
            self.unsettled.store(true, Ordering::SeqCst);
            return Err(storage(format!(
                "append failed, outcome unknown ({e}); the store refuses writes until reopened"
            )));
        }
        log.apply(card, event);
        Ok(())
    }
}

impl Store for MmdbStore {
    fn get(&self, id: Uuid) -> Result<Option<Card>, BoardError> {
        Ok(self.log.lock().map_err(poisoned)?.get(id))
    }

    fn list(&self) -> Result<Vec<Card>, BoardError> {
        Ok(self.log.lock().map_err(poisoned)?.list())
    }

    fn insert(&self, card: &Card, event: Event) -> Result<(), BoardError> {
        self.commit(card, event, true)
    }

    fn replace(&self, card: &Card, event: Event) -> Result<(), BoardError> {
        self.commit(card, event, false)
    }

    fn events_since(&self, after: Option<Uuid>, limit: usize) -> Result<Vec<Event>, BoardError> {
        Ok(self.log.lock().map_err(poisoned)?.since(after, limit))
    }

    fn last_event_id(&self) -> Result<Option<Uuid>, BoardError> {
        Ok(self.log.lock().map_err(poisoned)?.last_id())
    }
}
