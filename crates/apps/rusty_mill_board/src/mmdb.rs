//! Durable store on `rusty_multimodal_db_engine`.
//!
//! Cards and events share one mmap-backed table of [`Row`]s: the payload is
//! JSON (the engine never parses it), `kind` is the indexed field that tells
//! them apart, and `rev` is the scannable field. Atomic compare-and-swap is
//! the engine's `replace_if`.

use std::path::Path;

use rusty_multimodal_db_engine::generic::mmap_store::GenericMmapStore;
use rusty_multimodal_db_engine::generic::production::GenericProductionStore;
use rusty_multimodal_db_engine::generic::traits::{IndexedField, Record, ScannableField, SchemaTag};
use rusty_multimodal_db_engine::generic::{GuardedReplace, ReplaceError};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use uuid::Uuid;

use crate::domain::{BoardError, Card, Event};
use crate::store::Store;

const CARD: &str = "card";
const EVENT: &str = "event";

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
impl SchemaTag for Row {
    const SCHEMA_TAG: &'static str = "mill_board::Row@1";
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
pub struct MmdbStore {
    db: Db,
}

fn storage(e: impl std::fmt::Display) -> BoardError {
    BoardError::Storage(e.to_string())
}

fn row<T: Serialize>(id: Uuid, kind: &str, rev: u64, value: &T) -> Result<Row, BoardError> {
    Ok(Row {
        id,
        kind: kind.to_owned(),
        rev: i64::try_from(rev).map_err(storage)?,
        json: serde_json::to_string(value).map_err(storage)?,
    })
}

fn decode<T: DeserializeOwned>(row: &Row) -> Result<T, BoardError> {
    serde_json::from_str(&row.json).map_err(storage)
}

impl MmdbStore {
    /// Open the store at `path`, creating it if absent.
    ///
    /// # Errors
    /// If the files cannot be created or read back.
    pub fn open(path: &Path) -> Result<Self, BoardError> {
        let core = if path.exists() {
            GenericMmapStore::open_portable(path)
        } else {
            GenericMmapStore::create(Vec::new(), path)
        }
        .map_err(storage)?;
        Ok(Self { db: GenericProductionStore::new(core) })
    }

    fn rows(&self, kind: &str) -> Vec<Row> {
        self.db
            .filter_eq::<Row, Kind>(&kind.to_owned())
            .into_iter()
            .filter_map(|id| self.db.get::<Row>(id))
            .collect()
    }
}

impl Store for MmdbStore {
    fn get(&self, id: Uuid) -> Result<Option<Card>, BoardError> {
        self.db
            .get::<Row>(id)
            .filter(|r| r.kind == CARD)
            .map(|r| decode(&r))
            .transpose()
    }

    fn list(&self) -> Result<Vec<Card>, BoardError> {
        let mut cards = self.rows(CARD).iter().map(decode::<Card>).collect::<Result<Vec<_>, _>>()?;
        cards.sort_by_key(|c| (c.created_ms, c.id));
        Ok(cards)
    }

    fn insert(&self, card: &Card, event: &Event) -> Result<(), BoardError> {
        self.db
            .insert(row(card.id, CARD, card.rev, card)?)
            .map_err(|_| BoardError::Conflict)?;
        self.db.insert(row(event.id, EVENT, event.rev, event)?).map_err(storage)
    }

    fn replace(&self, card: &Card, event: &Event) -> Result<(), BoardError> {
        let expected = i64::try_from(card.rev).map_err(storage)? - 1;
        match self.db.replace_if(row(card.id, CARD, card.rev, card)?, |cur| cur.rev == expected) {
            Ok(GuardedReplace::Replaced) => {}
            Ok(GuardedReplace::Refused) => return Err(BoardError::Conflict),
            Err(ReplaceError::NotFound(id)) => return Err(BoardError::NotFound(id)),
            Err(e) => return Err(storage(e)),
        }
        self.db.insert(row(event.id, EVENT, event.rev, event)?).map_err(storage)
    }

    fn events_since(&self, after: Option<Uuid>, limit: usize) -> Result<Vec<Event>, BoardError> {
        let mut events = self
            .rows(EVENT)
            .iter()
            .filter(|r| after.is_none_or(|a| r.id > a))
            .map(decode::<Event>)
            .collect::<Result<Vec<_>, _>>()?;
        events.sort_by_key(|e| e.id);
        events.truncate(limit);
        Ok(events)
    }
}
