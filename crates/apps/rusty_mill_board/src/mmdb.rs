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

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use rusty_multimodal_db_engine::dir_lock::DirLock;
use rusty_multimodal_db_engine::generic::insert_log::log_path;
use rusty_multimodal_db_engine::generic::mmap_store::GenericMmapStore;
use rusty_multimodal_db_engine::generic::production::GenericProductionStore;
use rusty_multimodal_db_engine::generic::record_blob::blob_path;
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

/// A [`Store`] persisted at a path (`<path>`, `<path>.records` and the append log
/// `<path>.inserts`), owned by one process at a time: it holds `<path>.lock` while open.
///
/// A failed durable append is *ambiguous*: the record may or may not have reached the log
/// (the engine can fail after writing it). The store cannot tell, so it stops accepting
/// writes (`BoardError::Storage`) rather than guess; reads keep serving the last state it
/// knows. Reopening replays the log and settles the question either way.
pub struct MmdbStore {
    path: PathBuf,
    /// Never read: the lock lasts as long as the store.
    _lock: DirLock,
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

/// Take `<path>.lock`, so a second process cannot open the same board and diverge from this one.
fn lock(path: &Path) -> Result<DirLock, BoardError> {
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| storage(format!("{} names no file", path.display())))?;
    DirLock::acquire(dir, &format!("{}.lock", name.to_string_lossy())).map_err(storage)
}

/// Copy `from` into `dir` under its own name and flush the copy. Fails if `from` is missing.
fn copy_synced(from: &Path, dir: &Path) -> Result<(), BoardError> {
    let name = from
        .file_name()
        .ok_or_else(|| storage(format!("{} names no file", from.display())))?;
    let to = dir.join(name);
    std::fs::copy(from, &to).map_err(|e| storage(format!("copy {}: {e}", from.display())))?;
    // Opened for writing: Windows refuses to flush a handle that cannot write.
    std::fs::OpenOptions::new()
        .write(true)
        .open(&to)
        .and_then(|f| f.sync_all())
        .map_err(|e| storage(format!("sync {}: {e}", to.display())))
}

impl MmdbStore {
    /// Open the store at `path`, creating it if absent, and rebuild the
    /// board's state from its commit records.
    ///
    /// # Errors
    /// If another process has the store open, the files cannot be created or read back,
    /// or a record does not decode.
    pub fn open(path: &Path) -> Result<Self, BoardError> {
        let lock = lock(path)?;
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
            path: path.to_path_buf(),
            _lock: lock,
            db,
            log: Mutex::new(log),
            unsettled: AtomicBool::new(false),
        })
    }

    /// `Err` after a failed append of unknown outcome. The durable log may then hold an event the
    /// in-memory state lacks; only a reopen replays it, so nothing may rewrite or retire it first.
    fn settled(&self) -> Result<(), BoardError> {
        if self.unsettled.load(Ordering::SeqCst) {
            return Err(storage(
                "an earlier write failed with an unknown outcome; restart the service to reopen the store and settle it",
            ));
        }
        Ok(())
    }

    fn commit(&self, card: &Card, event: Event, is_insert: bool) -> Result<(), BoardError> {
        let mut log = self.log.lock().map_err(poisoned)?;
        self.settled()?;
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

    fn insert(&self, card: &Card, event: Event) -> Result<u64, BoardError> {
        self.commit(card, event, true).map(|()| card.rev)
    }

    fn replace(&self, card: &Card, event: Event) -> Result<u64, BoardError> {
        self.commit(card, event, false).map(|()| card.rev)
    }

    fn events_since(&self, after: Option<Uuid>, limit: usize) -> Result<Vec<Event>, BoardError> {
        Ok(self.log.lock().map_err(poisoned)?.since(after, limit))
    }

    fn last_event_id(&self) -> Result<Option<Uuid>, BoardError> {
        Ok(self.log.lock().map_err(poisoned)?.last_id())
    }

    fn health(&self) -> Result<(), BoardError> {
        self.settled()
    }

    fn backup(&self, dir: &Path) -> Result<OsString, BoardError> {
        // Holding the commit lock keeps every append out while the files are copied. Not while
        // unsettled: the copy would hold an event the state lacks, and a rotation of backups
        // could retire the last copies from before the failure.
        let _log = self.log.lock().map_err(poisoned)?;
        self.settled()?;
        // The main file and its record blob always exist; a store without them is damaged (the
        // mapping can outlive a file removed from under it), and a backup of that is no backup.
        // Only the append log is optional: it is absent until the first commit and after a compaction.
        for required in [self.path.clone(), blob_path(&self.path)] {
            copy_synced(&required, dir)?;
        }
        let log = log_path(&self.path);
        if log.exists() {
            copy_synced(&log, dir)?;
        }
        self.path
            .file_name()
            .map(OsString::from)
            .ok_or_else(|| storage(format!("{} names no file", self.path.display())))
    }

    fn compact(&self) -> Result<(), BoardError> {
        // Under the commit lock, and refused while unsettled: compaction writes the in-memory
        // records and clears the append log, which would erase the one event a reopen recovers.
        let _log = self.log.lock().map_err(poisoned)?;
        self.settled()?;
        self.db.compact().map(drop).map_err(storage)
    }
}
