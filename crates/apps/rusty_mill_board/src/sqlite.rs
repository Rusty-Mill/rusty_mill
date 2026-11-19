//! The SQLite adapter of the storage port (ADR-0007): the board owns one file.
//!
//! One `BEGIN IMMEDIATE` transaction per commit writes the card and its event, so they become
//! visible together, a commit is atomic across a crash, and `PartlySaved` cannot happen here.
//! The event log is durable, so a restart no longer forces every poller to resync.
//!
//! A card is stored whole as JSON (`doc`), beside the few columns the board queries by. A new
//! card field therefore needs no migration, and what the domain wrote is what is read back.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rusty_sqlite::rusqlite::{self, OptionalExtension, TransactionBehavior, params};
use rusty_sqlite::{Connection, Migrations};
use serde_json::Value;
use uuid::Uuid;

use crate::domain::{BoardError, Card, Column, Event, Span, next_event_id};
use crate::milestones::{Milestone, MilestoneStore};
use crate::store::Store;

/// The schema, by `PRAGMA user_version`. Add a step; never edit one that has shipped.
fn migrations() -> Migrations {
    Migrations::new().add(
        1,
        "cards, events, moves, numbers, milestones",
        "CREATE TABLE cards (
            id TEXT PRIMARY KEY,
            number INTEGER,
            rev INTEGER NOT NULL,
            col TEXT NOT NULL,
            project TEXT NOT NULL,
            created_ms INTEGER NOT NULL,
            updated_ms INTEGER NOT NULL,
            doc TEXT NOT NULL
        );
        CREATE UNIQUE INDEX cards_number ON cards (number);
        CREATE INDEX cards_col ON cards (col);
        CREATE INDEX cards_project_col ON cards (project, col);
        CREATE INDEX cards_created ON cards (created_ms, id);
        CREATE TABLE events (
            id TEXT PRIMARY KEY,
            card TEXT NOT NULL,
            col TEXT NOT NULL,
            at_ms INTEGER NOT NULL,
            doc TEXT NOT NULL
        );
        CREATE INDEX events_card ON events (card, id);
        -- Column entries, kept apart from events so pruning or migrating events never loses them.
        CREATE TABLE moves (
            card TEXT NOT NULL,
            from_col TEXT,
            col TEXT NOT NULL,
            at_ms INTEGER NOT NULL
        );
        CREATE INDEX moves_card ON moves (card);
        CREATE INDEX moves_at ON moves (at_ms);
        CREATE TABLE numbers (number INTEGER PRIMARY KEY, card TEXT NOT NULL);
        CREATE TABLE milestones (
            id TEXT PRIMARY KEY,
            project TEXT NOT NULL,
            title TEXT NOT NULL,
            target_ms INTEGER NOT NULL,
            done_ms INTEGER,
            rev INTEGER NOT NULL
        );",
    )
}

/// Every card in the board file at `path`, read without changing anything: the file is opened
/// read-only (never created, no journal mode or migration), and one that is not at exactly this
/// build's schema is refused rather than brought up to date. For plans that must write nothing.
///
/// # Errors
/// [`BoardError::Storage`] if the file is missing, cannot be read, is not at this build's
/// schema version, or has a `-wal` or `-shm` file beside it (a board is running).
pub fn read_cards(path: impl AsRef<Path>) -> Result<Vec<Card>, BoardError> {
    let path = path.as_ref();
    // A `-wal` or `-shm` file means a board has the file open (or stopped uncleanly), and even a
    // read-only reader updates the shared `-shm` index. Refuse rather than touch it.
    for suffix in ["-wal", "-shm"] {
        let mut side = path.as_os_str().to_owned();
        side.push(suffix);
        if Path::new(&side).exists() {
            return Err(storage(format!(
                "{} exists: stop the board (let it exit cleanly) before reading its file",
                Path::new(&side).display()
            )));
        }
    }
    // `immutable=1`: no locks and no sidecars, so even a WAL-mode file is read without a `-shm`
    // or `-wal` appearing beside it. Safe here because no sidecar exists to be ignored.
    let conn = rusqlite::Connection::open_with_flags(
        immutable_uri(path)?,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX
            | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(storage)?;
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(storage)?;
    let want = migrations().latest_version();
    if version != want {
        return Err(storage(format!(
            "the board file is at schema {version}, this build reads {want}"
        )));
    }
    let mut stmt = conn
        .prepare("SELECT doc FROM cards ORDER BY created_ms, id")
        .map_err(storage)?;
    let docs = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    docs.iter().map(|d| card_of(d)).collect()
}

/// `path` as a SQLite `file:` URI that opens it immutable, percent-encoding all but the
/// characters a URI path keeps.
fn immutable_uri(path: &Path) -> Result<String, BoardError> {
    let text = path
        .to_str()
        .ok_or_else(|| storage("the path is not valid UTF-8"))?
        .replace('\\', "/");
    let mut uri = String::from("file:");
    if !text.starts_with('/') {
        uri.push('/');
    }
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'.' | b'_' | b'-' | b'~' | b':' => {
                uri.push(char::from(b));
            }
            _ => uri.push_str(&format!("%{b:02X}")),
        }
    }
    uri.push_str("?immutable=1");
    Ok(uri)
}

/// What [`SqliteStore::import`] loads.
pub struct Import<'a> {
    /// Every card, whole.
    pub cards: &'a [Card],
    /// Every number ever reserved, and its card.
    pub numbers: &'a [(u64, Uuid)],
    /// Column entries: the card and when. A card not in `cards` is skipped.
    pub moves: &'a [(Uuid, i64)],
    /// Every milestone, at its revision.
    pub milestones: &'a [Milestone],
    /// What the source held for a card that the card itself does not: unknown document members,
    /// comment ids. It is stored under its own key in the card's document, so it can never
    /// collide with a card member, and an edit leaves it alone (see `with_unknown`).
    pub source: &'a [(Uuid, serde_json::Map<String, Value>)],
}

/// The key of a card's document that holds [`Import::source`].
pub const SOURCE_KEY: &str = "migrated_from_tick";

/// Cards, events, reserved numbers and milestones in one SQLite file.
pub struct SqliteStore {
    conn: Mutex<Connection>,
}

/// Parse a stored id; a malformed one is a storage error, never a skipped row.
fn uuid(s: &str) -> Result<Uuid, BoardError> {
    Uuid::parse_str(s).map_err(storage)
}

fn storage(e: impl Display) -> BoardError {
    BoardError::Storage(e.to_string())
}

/// A revision or number as SQLite's integer. Out of range is an error, never a clamp: two
/// distinct revisions must not alias.
fn int(n: u64) -> Result<i64, BoardError> {
    i64::try_from(n).map_err(|_| storage(format!("{n} is out of range")))
}

fn uint(n: i64) -> Result<u64, BoardError> {
    u64::try_from(n).map_err(|_| storage(format!("{n} is out of range")))
}

/// `card`'s document, with what the source held for it under [`SOURCE_KEY`].
fn doc_with(
    card: &Card,
    source: &[(Uuid, serde_json::Map<String, Value>)],
) -> Result<String, BoardError> {
    let mut doc = serde_json::to_value(card).map_err(storage)?;
    if let (Some(out), Some((_, held))) = (
        doc.as_object_mut(),
        source.iter().find(|(id, _)| *id == card.id),
    ) {
        out.insert(SOURCE_KEY.into(), Value::Object(held.clone()));
    }
    serde_json::to_string(&doc).map_err(storage)
}

fn card_of(doc: &str) -> Result<Card, BoardError> {
    serde_json::from_str(doc).map_err(storage)
}

/// `card`'s document with every member of `stored` this board does not know written back, so a
/// newer board's fields survive an edit here (ADR-0007, S5a). A member is unknown when
/// re-serializing the stored card does not produce it; a known one the edit cleared stays gone.
fn with_unknown(card: &Card, stored: &str) -> Result<String, BoardError> {
    let mut doc = serde_json::to_value(card).map_err(storage)?;
    let raw: Value = serde_json::from_str(stored).map_err(storage)?;
    let known = serde_json::to_value(card_of(stored)?).map_err(storage)?;
    if let (Some(out), Some(raw), Some(known)) =
        (doc.as_object_mut(), raw.as_object(), known.as_object())
    {
        for (key, value) in raw.iter().filter(|(k, _)| !known.contains_key(*k)) {
            out.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
    serde_json::to_string(&doc).map_err(storage)
}

impl SqliteStore {
    /// Open (creating if needed) the board's file and bring its schema up to date.
    ///
    /// # Errors
    /// [`BoardError::Storage`] if the file cannot be opened, or holds a newer schema than this
    /// build knows.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, BoardError> {
        Self::with(Connection::open(path).map_err(storage)?)
    }

    /// A private in-memory database: for tests.
    ///
    /// # Errors
    /// [`BoardError::Storage`] if SQLite fails.
    pub fn open_in_memory() -> Result<Self, BoardError> {
        Self::with(Connection::open_in_memory().map_err(storage)?)
    }

    fn with(mut conn: Connection) -> Result<Self, BoardError> {
        conn.migrate(&migrations()).map_err(storage)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn db(&self) -> Result<MutexGuard<'_, Connection>, BoardError> {
        self.conn
            .lock()
            .map_err(|_| BoardError::Storage("lock poisoned".into()))
    }

    /// One card or event commit: `card` at `card.rev` and its `event`, in one transaction.
    fn commit(&self, card: &Card, mut event: Event, insert: bool) -> Result<u64, BoardError> {
        let doc = serde_json::to_string(card).map_err(storage)?;
        let mut db = self.db()?;
        let tx = db
            .as_raw_mut()
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let id = card.id.to_string();
        let before: Option<String> = tx
            .query_row("SELECT col FROM cards WHERE id = ?1", [&id], |r| r.get(0))
            .optional()
            .map_err(storage)?;
        let number = card.number.map(int).transpose()?;
        let rev = int(card.rev)?;
        let col = card.column.to_string();
        if insert {
            let n = tx
                .execute(
                    "INSERT OR IGNORE INTO cards (id, number, rev, col, project, created_ms, updated_ms, doc)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![id, number, rev, col, card.project, card.created_ms, card.updated_ms, doc],
                )
                .map_err(storage)?;
            if n == 0 {
                return Err(BoardError::Conflict);
            }
        } else {
            // A replacement is exactly one revision on: revision 0 has no predecessor, so it
            // never matches (as MemStore), only tells a missing card from a stale one below.
            let stored: Option<String> = tx
                .query_row("SELECT doc FROM cards WHERE id = ?1", [&id], |r| r.get(0))
                .optional()
                .map_err(storage)?;
            let doc = match &stored {
                Some(old) => with_unknown(card, old)?,
                None => doc,
            };
            let n = match card.rev.checked_sub(1) {
                None => 0,
                Some(prev) => tx
                    .execute(
                        "UPDATE cards SET number = ?2, rev = ?3, col = ?4, project = ?5, updated_ms = ?6, doc = ?7
                         WHERE id = ?1 AND rev = ?8",
                        params![id, number, rev, col, card.project, card.updated_ms, doc, int(prev)?],
                    )
                    .map_err(storage)?,
            };
            if n == 0 {
                let exists = tx
                    .query_row("SELECT 1 FROM cards WHERE id = ?1", [&id], |_| Ok(()))
                    .optional()
                    .map_err(storage)?;
                return Err(match exists {
                    Some(()) => BoardError::Conflict,
                    None => BoardError::NotFound(card.id),
                });
            }
        }
        let last: Option<String> = tx
            .query_row("SELECT MAX(id) FROM events", [], |r| r.get(0))
            .map_err(storage)?;
        event.id = next_event_id(last.as_deref().map(uuid).transpose()?);
        tx.execute(
            "INSERT INTO events (id, card, col, at_ms, doc) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                event.id.to_string(),
                id,
                event.column.to_string(),
                event.at_ms,
                serde_json::to_string(&event).map_err(storage)?
            ],
        )
        .map_err(storage)?;
        // A card enters a column on creation and whenever its column differs from the one it
        // had: compared with the stored card, not the last recorded move, so a card whose
        // history is older than the move table (a migrated one) gains no row from an edit.
        let entered = event.column.to_string();
        if before.as_deref() != Some(entered.as_str()) {
            tx.execute(
                "INSERT INTO moves (card, from_col, col, at_ms) VALUES (?1, ?2, ?3, ?4)",
                params![id, before, entered, event.at_ms],
            )
            .map_err(storage)?;
        }
        tx.commit().map_err(storage)?;
        Ok(card.rev)
    }

    /// Load a whole board into an empty file in one transaction: the move from rusty_tick
    /// (ADR-0007). Revisions, times, numbers and comments are kept as given. The event log
    /// starts empty, so every poller resyncs once.
    ///
    /// # Errors
    /// [`BoardError::Conflict`] if the file already holds cards, numbers or milestones;
    /// [`BoardError::Storage`] if SQLite fails (nothing is written).
    pub fn import(&self, data: &Import<'_>) -> Result<(), BoardError> {
        let mut db = self.db()?;
        let tx = db
            .as_raw_mut()
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let rows: i64 = tx
            .query_row(
                "SELECT (SELECT COUNT(*) FROM cards) + (SELECT COUNT(*) FROM numbers)
                        + (SELECT COUNT(*) FROM milestones)",
                [],
                |r| r.get(0),
            )
            .map_err(storage)?;
        if rows != 0 {
            return Err(BoardError::Conflict);
        }
        for card in data.cards {
            tx.execute(
                "INSERT INTO cards (id, number, rev, col, project, created_ms, updated_ms, doc)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    card.id.to_string(),
                    card.number.map(int).transpose()?,
                    int(card.rev)?,
                    card.column.to_string(),
                    card.project,
                    card.created_ms,
                    card.updated_ms,
                    doc_with(card, data.source)?
                ],
            )
            .map_err(storage)?;
        }
        for (number, card) in data.numbers {
            tx.execute(
                "INSERT INTO numbers (number, card) VALUES (?1, ?2)",
                params![int(*number)?, card.to_string()],
            )
            .map_err(storage)?;
        }
        let columns: BTreeMap<Uuid, String> = data
            .cards
            .iter()
            .map(|c| (c.id, c.column.to_string()))
            .collect();
        for (card, at_ms) in data.moves {
            // The move history has no column: the card's own stands in.
            let Some(entered) = columns.get(card) else {
                continue;
            };
            tx.execute(
                "INSERT INTO moves (card, from_col, col, at_ms) VALUES (?1, NULL, ?2, ?3)",
                params![card.to_string(), entered, at_ms],
            )
            .map_err(storage)?;
        }
        for m in data.milestones {
            tx.execute(
                "INSERT INTO milestones (id, project, title, target_ms, done_ms, rev)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    m.id.to_string(),
                    m.project,
                    m.title,
                    m.target_ms,
                    m.done_ms,
                    int(m.rev)?
                ],
            )
            .map_err(storage)?;
        }
        tx.commit().map_err(storage)
    }

    /// Every column entry as `(card, at_ms)`, sorted, repeats kept: what a migration compares.
    ///
    /// # Errors
    /// [`BoardError::Storage`] if SQLite fails or a stored id is malformed.
    pub fn moves(&self) -> Result<Vec<(Uuid, i64)>, BoardError> {
        let db = self.db()?;
        let mut stmt = db
            .as_raw()
            .prepare_cached("SELECT card, at_ms FROM moves ORDER BY card, at_ms")
            .map_err(storage)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        rows.into_iter()
            .map(|(id, at)| Ok((uuid(&id)?, at)))
            .collect()
    }

    /// A card's stored document as JSON, members this board does not know included.
    ///
    /// # Errors
    /// [`BoardError::Storage`] if SQLite fails or the document is not JSON.
    pub fn document(&self, id: Uuid) -> Result<Option<Value>, BoardError> {
        let doc: Option<String> = self
            .db()?
            .as_raw()
            .query_row(
                "SELECT doc FROM cards WHERE id = ?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()
            .map_err(storage)?;
        doc.map(|d| serde_json::from_str(&d).map_err(storage))
            .transpose()
    }

    /// Every reserved number and its card, by number.
    ///
    /// # Errors
    /// [`BoardError::Storage`] if SQLite fails or a stored value is malformed.
    pub fn numbers(&self) -> Result<Vec<(u64, Uuid)>, BoardError> {
        let db = self.db()?;
        let mut stmt = db
            .as_raw()
            .prepare_cached("SELECT number, card FROM numbers ORDER BY number")
            .map_err(storage)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        rows.into_iter()
            .map(|(n, card)| Ok((uint(n)?, uuid(&card)?)))
            .collect()
    }

    /// Move everything the write-ahead log holds into the main file and empty the log, so the
    /// main file alone is the whole database.
    ///
    /// # Errors
    /// [`BoardError::Storage`] if the checkpoint fails or could not complete (a reader or
    /// writer held it up): the main file may then lack committed data.
    pub fn checkpoint(&self) -> Result<(), BoardError> {
        let (busy, log, done): (i64, i64, i64) = self
            .db()?
            .as_raw()
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .map_err(storage)?;
        // An in-memory or non-WAL database reports -1 for both counts: nothing to move.
        if busy != 0 || (log >= 0 && log != done) {
            return Err(storage(format!(
                "checkpoint incomplete: busy {busy}, {done} of {log} frames moved"
            )));
        }
        Ok(())
    }

    /// The cards a query selects, parsed from their documents.
    fn cards(&self, sql: &str, params: impl rusqlite::Params) -> Result<Vec<Card>, BoardError> {
        let db = self.db()?;
        let mut stmt = db.as_raw().prepare_cached(sql).map_err(storage)?;
        let docs = stmt
            .query_map(params, |r| r.get::<_, String>(0))
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        docs.iter().map(|d| card_of(d)).collect()
    }
}

impl Store for SqliteStore {
    fn get(&self, id: Uuid) -> Result<Option<Card>, BoardError> {
        self.cards("SELECT doc FROM cards WHERE id = ?1", [id.to_string()])
            .map(|mut c| c.pop())
    }

    fn list(&self) -> Result<Vec<Card>, BoardError> {
        self.cards("SELECT doc FROM cards ORDER BY created_ms, id", [])
    }

    fn numbered(&self, number: u64) -> Result<Vec<Card>, BoardError> {
        self.cards(
            "SELECT doc FROM cards WHERE number = ?1 ORDER BY created_ms, id",
            [int(number)?],
        )
    }

    fn in_columns(&self, columns: &[Column]) -> Result<Vec<Card>, BoardError> {
        let names: Vec<String> = columns.iter().map(ToString::to_string).collect();
        self.cards(
            "SELECT doc FROM cards WHERE col IN (SELECT value FROM json_each(?1)) ORDER BY created_ms, id",
            [serde_json::to_string(&names).map_err(storage)?],
        )
    }

    fn open_among(&self, ids: &[Uuid]) -> Result<BTreeSet<Uuid>, BoardError> {
        let ids: Vec<String> = ids.iter().map(ToString::to_string).collect();
        let db = self.db()?;
        let mut stmt = db
            .as_raw()
            .prepare_cached(
                "SELECT id FROM cards WHERE col <> ?2 AND id IN (SELECT value FROM json_each(?1))",
            )
            .map_err(storage)?;
        let rows = stmt
            .query_map(
                params![
                    serde_json::to_string(&ids).map_err(storage)?,
                    Column::DONE.to_string()
                ],
                |r| r.get::<_, String>(0),
            )
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        rows.iter().map(|s| uuid(s)).collect()
    }

    fn insert(&self, card: &Card, event: Event) -> Result<u64, BoardError> {
        self.commit(card, event, true)
    }

    fn replace(&self, card: &Card, event: Event) -> Result<u64, BoardError> {
        self.commit(card, event, false)
    }

    fn events_since(&self, after: Option<Uuid>, limit: usize) -> Result<Vec<Event>, BoardError> {
        let db = self.db()?;
        let mut stmt = db
            .as_raw()
            .prepare_cached(
                "SELECT doc FROM events WHERE (?1 IS NULL OR id > ?1) ORDER BY id LIMIT ?2",
            )
            .map_err(storage)?;
        let docs = stmt
            .query_map(
                params![
                    after.map(|a| a.to_string()),
                    i64::try_from(limit).unwrap_or(i64::MAX)
                ],
                |r| r.get::<_, String>(0),
            )
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        docs.iter()
            .map(|d| serde_json::from_str(d).map_err(storage))
            .collect()
    }

    fn last_event_id(&self) -> Result<Option<Uuid>, BoardError> {
        let last: Option<String> = self
            .db()?
            .as_raw()
            .query_row("SELECT MAX(id) FROM events", [], |r| r.get(0))
            .map_err(storage)?;
        last.as_deref().map(uuid).transpose()
    }

    fn holds_event(&self, id: Uuid) -> Result<bool, BoardError> {
        Ok(self
            .db()?
            .as_raw()
            .query_row(
                "SELECT 1 FROM events WHERE id = ?1",
                [id.to_string()],
                |_| Ok(()),
            )
            .optional()
            .map_err(storage)?
            .is_some())
    }

    fn reserve_number(&self, card: Uuid, floor: u64) -> Result<u64, BoardError> {
        let mut db = self.db()?;
        let tx = db
            .as_raw_mut()
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let top: i64 = tx
            .query_row("SELECT COALESCE(MAX(number), 0) FROM numbers", [], |r| {
                r.get(0)
            })
            .map_err(storage)?;
        let next = int(uint(top)?.max(floor))?
            .checked_add(1)
            .ok_or_else(|| storage("card numbers are exhausted"))?;
        tx.execute(
            "INSERT INTO numbers (number, card) VALUES (?1, ?2)",
            params![next, card.to_string()],
        )
        .map_err(storage)?;
        tx.commit().map_err(storage)?;
        uint(next)
    }

    fn highest_reserved(&self) -> Result<u64, BoardError> {
        let top: i64 = self
            .db()?
            .as_raw()
            .query_row("SELECT COALESCE(MAX(number), 0) FROM numbers", [], |r| {
                r.get(0)
            })
            .map_err(storage)?;
        uint(top)
    }

    fn health(&self) -> Result<(), BoardError> {
        self.db()?
            .as_raw()
            .query_row("SELECT 1", [], |_| Ok(()))
            .map_err(storage)
    }

    fn moved_in(&self, span: Span) -> Result<BTreeSet<Uuid>, BoardError> {
        // A card moved when it entered a column, the board included: one row per entry.
        let db = self.db()?;
        let mut stmt = db
            .as_raw()
            .prepare_cached(
                "SELECT DISTINCT card FROM moves
                 WHERE (?1 IS NULL OR at_ms >= ?1) AND (?2 IS NULL OR at_ms < ?2)",
            )
            .map_err(storage)?;
        let rows = stmt
            .query_map(params![span.from, span.to], |r| r.get::<_, String>(0))
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        rows.iter().map(|s| uuid(s)).collect()
    }

    fn durable(&self) -> bool {
        true
    }
}

fn milestone_of(r: &rusqlite::Row<'_>) -> rusqlite::Result<(String, Milestone, i64)> {
    let id: String = r.get(0)?;
    Ok((
        id,
        Milestone {
            id: Uuid::nil(),
            project: r.get(1)?,
            title: r.get(2)?,
            target_ms: r.get(3)?,
            done_ms: r.get(4)?,
            rev: 0,
        },
        r.get(5)?,
    ))
}

impl MilestoneStore for SqliteStore {
    fn milestones(&self) -> Result<Vec<Milestone>, BoardError> {
        let db = self.db()?;
        let mut stmt = db
            .as_raw()
            .prepare_cached(
                "SELECT id, project, title, target_ms, done_ms, rev FROM milestones ORDER BY id",
            )
            .map_err(storage)?;
        let rows = stmt
            .query_map([], milestone_of)
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        rows.into_iter()
            .map(|(id, m, rev)| {
                Ok(Milestone {
                    id: uuid(&id)?,
                    rev: uint(rev)?,
                    ..m
                })
            })
            .collect()
    }

    fn milestone(&self, id: Uuid) -> Result<Option<Milestone>, BoardError> {
        let row = self
            .db()?
            .as_raw()
            .query_row(
                "SELECT id, project, title, target_ms, done_ms, rev FROM milestones WHERE id = ?1",
                [id.to_string()],
                milestone_of,
            )
            .optional()
            .map_err(storage)?;
        row.map(|(_, m, rev)| {
            Ok(Milestone {
                id,
                rev: uint(rev)?,
                ..m
            })
        })
        .transpose()
    }

    fn add_milestone(&self, m: &Milestone) -> Result<u64, BoardError> {
        let n = self
            .db()?
            .as_raw()
            .execute(
                "INSERT OR IGNORE INTO milestones (id, project, title, target_ms, done_ms, rev)
                 VALUES (?1, ?2, ?3, ?4, ?5, 1)",
                params![m.id.to_string(), m.project, m.title, m.target_ms, m.done_ms],
            )
            .map_err(storage)?;
        if n == 0 {
            return Err(BoardError::Conflict);
        }
        Ok(1)
    }

    fn save_milestone(&self, current: &Milestone, next: &Milestone) -> Result<u64, BoardError> {
        let at = int(current.rev)?;
        let successor = current
            .rev
            .checked_add(1)
            .ok_or_else(|| BoardError::Storage("milestone revision exhausted".into()))?;
        let to = int(successor)?;
        let db = self.db()?;
        let n = db
            .as_raw()
            .execute(
                "UPDATE milestones SET project = ?2, title = ?3, target_ms = ?4, done_ms = ?5, rev = ?7
                 WHERE id = ?1 AND rev = ?6",
                params![
                    current.id.to_string(),
                    next.project,
                    next.title,
                    next.target_ms,
                    next.done_ms,
                    at,
                    to
                ],
            )
            .map_err(storage)?;
        if n == 1 {
            return Ok(successor);
        }
        let exists = db
            .as_raw()
            .query_row(
                "SELECT 1 FROM milestones WHERE id = ?1",
                [current.id.to_string()],
                |_| Ok(()),
            )
            .optional()
            .map_err(storage)?;
        Err(match exists {
            Some(()) => BoardError::Conflict,
            None => BoardError::NotFound(current.id),
        })
    }
}
