//! Column history for Mill Board: one append-only record each time a task's
//! board column changes, however it changed (a board `PATCH`, the tick UI, a
//! tag rename). The service appends it in the same locked operation as the
//! task write, as a small redo: the write's moves are made durable as an
//! intent first ([`MoveStore::begin`]), the task is written, then the moves are
//! appended and the intent retired ([`MoveStore::commit`]). A crash or failure
//! in between leaves the intent, which [`MoveStore::open`] replays against the
//! tasks; until then every task write is refused, as the task store does, so a
//! later move never hides a missing one. Nothing is ever deleted, so a `seq` is
//! never reused. History starts when this shipped: there is no backfill.

use crate::store::TickError;
use crate::table::Table;
use crate::task::{Status, Task};
use rusty_multimodal_db_engine::generic::insert_log;
use rusty_multimodal_db_engine::generic::traits::{
    IndexedField, Record, ScannableField, SchemaTag,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// The tag that puts a task on Mill Board.
pub const BOARD_TAG: &str = "agent";
/// The tag prefix that names a task's board column.
pub const COLUMN_PREFIX: &str = "board:";
/// Longest Mill Board column id, in bytes.
pub const MAX_COLUMN_BYTES: usize = 32;
/// The column of an open card whose `board:` tag is missing or not a column id.
pub const DEFAULT_COLUMN: &str = "backlog";
/// The column of a closed card.
pub const DONE_COLUMN: &str = "done";
/// Who a move is recorded as when the request names nobody.
pub const DEFAULT_ACTOR: &str = "tick";
/// Longest accepted actor, in bytes.
pub const MAX_ACTOR_BYTES: usize = 64;
/// Most moves one page returns.
pub const MAX_PAGE: usize = 1000;

/// Whether `id` is a Mill Board column id (`rusty_mill_board::domain::Column`): 1 to
/// [`MAX_COLUMN_BYTES`] of `a-z 0-9 _`, starting with a letter. Which columns exist is the
/// board's (an admin edits them), so any such id is recorded as given.
pub fn valid_column(id: &str) -> bool {
    id.len() <= MAX_COLUMN_BYTES
        && id.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// The board column `task` is in, exactly as Mill Board's `card_of` reads it:
/// `None` off the board (no [`BOARD_TAG`]), `done` when closed, else its first
/// `board:` tag if that is a column id ([`valid_column`]), else `backlog`.
pub fn column_of(task: &Task) -> Option<&str> {
    if !task.tags.iter().any(|t| t == BOARD_TAG) {
        return None;
    }
    if task.status != Status::Open {
        return Some(DONE_COLUMN);
    }
    let tag = task
        .tags
        .iter()
        .find_map(|t| t.strip_prefix(COLUMN_PREFIX))
        .filter(|c| valid_column(c));
    Some(tag.unwrap_or(DEFAULT_COLUMN))
}

/// Whether `actor` is a usable name: 1 to [`MAX_ACTOR_BYTES`] printable ASCII
/// characters, no spaces.
pub fn valid_actor(actor: &str) -> bool {
    !actor.is_empty()
        && actor.len() <= MAX_ACTOR_BYTES
        && actor.bytes().all(|b| b.is_ascii_graphic())
}

/// One column change. `seq` orders every move in a store (write order) and is
/// the paging cursor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Move {
    pub seq: u64,
    pub task_id: Uuid,
    pub from: Option<String>,
    pub to: Option<String>,
    /// The task write's `updated_ms`.
    pub at_ms: i64,
    pub actor: String,
}

impl Move {
    /// A move not yet written: its `seq` is assigned by [`MoveStore::flush`].
    fn new(task_id: Uuid, from: Option<&str>, to: Option<&str>, at_ms: i64, actor: &str) -> Self {
        Self {
            seq: 0,
            task_id,
            from: from.map(str::to_string),
            to: to.map(str::to_string),
            at_ms,
            actor: actor.to_string(),
        }
    }
}

impl Record for Move {
    type Id = Uuid;
    /// Derived from `seq`, which is unique, so a move needs no second identity.
    fn id(&self) -> Uuid {
        Uuid::from_u128(u128::from(self.seq))
    }
}

impl SchemaTag for Move {
    const SCHEMA_TAG: &'static str = "rusty_tick::Move@1";
}

/// Index marker: moves by task.
pub struct ByTask;
impl IndexedField<ByTask> for Move {
    type IndexValue = Uuid;
    fn indexed_value(&self) -> &Uuid {
        &self.task_id
    }
}

pub struct AtMs;
impl ScannableField<AtMs> for Move {
    type ScanValue = i64;
    fn scannable_value(&self) -> i64 {
        self.at_ms
    }
    fn set_scannable_value(&mut self, value: i64) {
        self.at_ms = value;
    }
}

/// The file holding the moves of the task write in progress (the intent).
const INTENT_FILE: &str = "moves.intent";

pub struct MoveStore {
    table: Table<Move, ByTask, AtMs>,
    intent: PathBuf,
    next_seq: u64,
    /// The moves of the task write in progress, durable in `intent` until
    /// [`MoveStore::commit`] appends them.
    pending: Vec<Move>,
    /// An append failed: refuse writes until reopened, when the intent replays.
    failed: bool,
}

impl MoveStore {
    /// Open the history in `dir`, replaying an intent left by a crash or failure:
    /// each of its moves is appended if `task` shows the task write landed (the
    /// task is in the move's `to` column) and the move is not already recorded.
    pub fn open(dir: &Path, task: impl Fn(Uuid) -> Option<Task>) -> Result<Self, TickError> {
        let path = dir.join("moves.mmap");
        // A log whose creation was interrupted (empty, or a prefix of its header)
        // holds no record; the move being written is in the intent, replayed below.
        if path.exists() {
            insert_log::clear_interrupted_creation::<Move>(&path)?;
        }
        let table: Table<Move, ByTask, AtMs> = Table::open(dir, "moves.mmap")?;
        let next_seq = table.all().iter().map(|m| m.seq + 1).max().unwrap_or(1);
        let mut store = Self {
            table,
            intent: dir.join(INTENT_FILE),
            next_seq,
            pending: Vec::new(),
            failed: false,
        };
        store.replay(task)?;
        Ok(store)
    }

    fn replay(&mut self, task: impl Fn(Uuid) -> Option<Task>) -> Result<(), TickError> {
        let text = match std::fs::read_to_string(&self.intent) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        let planned: Vec<Move> = rusty_json::from_str(&text).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{INTENT_FILE}: {e}"),
            )
        })?;
        self.pending = planned
            .into_iter()
            .filter(|m| {
                let landed = task(m.task_id).as_ref().and_then(column_of) == m.to.as_deref();
                let recorded = self
                    .of_task(m.task_id)
                    .last()
                    .is_some_and(|last| last.to == m.to && last.at_ms == m.at_ms);
                landed && !recorded
            })
            .collect();
        self.commit()?;
        // Nothing to append still retires the intent.
        remove_if_present(&self.intent)
    }

    /// The move from `old` to `new` (`None` for a task being created), if the
    /// column changed.
    pub fn plan(old: Option<&Task>, new: &Task, actor: &str) -> Option<Move> {
        let from = old.and_then(column_of);
        let to = column_of(new);
        (from != to).then(|| Move::new(new.id, from, to, new.updated_ms, actor))
    }

    /// Make `moves` durable as the intent of the task write about to happen.
    pub fn begin(&mut self, moves: Vec<Move>) -> Result<(), TickError> {
        self.require_writable()?;
        if moves.is_empty() {
            return Ok(());
        }
        let text = rusty_json::to_string(&moves).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{INTENT_FILE}: {e}"),
            )
        })?;
        rusty_atomic_file::write(&self.intent, text.as_bytes())?;
        self.pending = moves;
        Ok(())
    }

    /// The task write landed: append the intent's moves, in order, and retire it.
    /// On failure the intent stays and writes are refused until reopen replays it.
    pub fn commit(&mut self) -> Result<(), TickError> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let result = self.append_pending();
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn append_pending(&mut self) -> Result<(), TickError> {
        for entry in std::mem::take(&mut self.pending) {
            let entry = Move {
                seq: self.next_seq,
                ..entry
            };
            self.table.insert(entry)?;
            self.next_seq += 1;
        }
        remove_if_present(&self.intent)
    }

    /// The task write failed. If it may still have landed (`ambiguous`: the task
    /// store needs reopening), keep the intent for the replay and refuse writes;
    /// otherwise retire it.
    pub fn abort(&mut self, ambiguous: bool) -> Result<(), TickError> {
        if std::mem::take(&mut self.pending).is_empty() {
            return Ok(());
        }
        if ambiguous {
            self.failed = true;
            return Ok(());
        }
        let result = remove_if_present(&self.intent);
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    /// `Err(RecoveryRequired)` after a failed append: the service checks this
    /// before every task write.
    pub fn require_writable(&self) -> Result<(), TickError> {
        if self.failed {
            Err(TickError::RecoveryRequired)
        } else {
            Ok(())
        }
    }

    /// Moves at or after `since`, past `after`, oldest first, at most `limit`.
    pub fn page(&self, since: i64, after: u64, limit: usize) -> Vec<Move> {
        let mut moves: Vec<Move> = self
            .table
            .all()
            .into_iter()
            .filter(|m| m.seq > after && m.at_ms >= since)
            .collect();
        moves.sort_by_key(|m| m.seq);
        moves.truncate(limit);
        moves
    }

    /// Every move of `task`, oldest first.
    pub fn of_task(&self, task: Uuid) -> Vec<Move> {
        let mut moves = self.table.with_index(&task);
        moves.sort_by_key(|m| m.seq);
        moves
    }
}

fn remove_if_present(path: &Path) -> Result<(), TickError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_multimodal_db_engine::test_support::{arm_fault, Fault};

    fn task(tags: &[&str], status: Status) -> Task {
        let mut task = Task::new(Uuid::now_v7(), Uuid::nil(), "t", 0, 10);
        task.tags = tags.iter().map(|t| t.to_string()).collect();
        task.status = status;
        task
    }

    fn with_tags(task: &Task, tags: &[&str], at: i64) -> Task {
        let mut next = task.clone();
        next.tags = tags.iter().map(|t| t.to_string()).collect();
        next.updated_ms = at;
        next
    }

    fn path(moves: &[Move]) -> Vec<(u64, Option<&str>, Option<&str>)> {
        moves
            .iter()
            .map(|m| (m.seq, m.from.as_deref(), m.to.as_deref()))
            .collect()
    }

    fn open(dir: &Path, landed: &[&Task]) -> MoveStore {
        MoveStore::open(dir, |id| {
            landed.iter().find(|t| t.id == id).map(|t| (*t).clone())
        })
        .unwrap()
    }

    /// The service's sequence for a write that lands: intent, (task write), commit.
    fn write(store: &mut MoveStore, old: Option<&Task>, new: &Task, actor: &str) {
        store
            .begin(MoveStore::plan(old, new, actor).into_iter().collect())
            .unwrap();
        store.commit().unwrap();
    }

    #[test]
    fn the_column_is_read_as_mill_board_reads_it() {
        let column = |tags: &[&str], status| column_of(&task(tags, status)).map(str::to_owned);
        assert_eq!(
            column(&["agent", "board:qa"], Status::Open),
            Some("qa".to_owned())
        );
        assert_eq!(
            column(&["agent", "board:qa"], Status::WontDo),
            Some("done".to_owned())
        );
        assert_eq!(column(&["agent"], Status::Open), Some("backlog".to_owned()));
        assert_eq!(
            column(&["agent", "board:design"], Status::Open),
            Some("design".to_owned()),
            "a column an admin added is recorded as given"
        );
        assert_eq!(
            column(&["agent", "board:Not A Column"], Status::Open),
            Some("backlog".to_owned())
        );
        assert_eq!(
            column(&["agent", "board:", "board:qa"], Status::Open),
            Some("backlog".to_owned()),
            "only the first board: tag counts, as on the board"
        );
        assert_eq!(column(&["agent"], Status::Done), Some("done".to_owned()));
        assert_eq!(
            column(&["board:qa"], Status::Open),
            None,
            "not on the board"
        );
    }

    #[test]
    fn actors_are_short_printable_names() {
        assert!(valid_actor("claude-builder"));
        assert!(!valid_actor(""));
        assert!(!valid_actor("two words"));
        assert!(!valid_actor(&"a".repeat(MAX_ACTOR_BYTES + 1)));
    }

    #[test]
    fn only_a_column_change_is_planned_and_seq_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path(), &[]);
        let ready = task(&["agent", "board:ready"], Status::Open);
        let mut edited = ready.clone();
        edited.title = "renamed".into();
        let building = with_tags(&ready, &["agent", "board:building"], 20);

        write(&mut store, None, &ready, "a");
        assert!(MoveStore::plan(Some(&ready), &edited, "a").is_none());
        write(&mut store, Some(&ready), &building, "b");
        drop(store);

        let mut store = open(dir.path(), &[]);
        let mut done = building.clone();
        done.status = Status::Done;
        write(&mut store, Some(&building), &done, "c");
        let moves = store.of_task(ready.id);
        assert_eq!(
            path(&moves),
            [
                (1, None, Some("ready")),
                (2, Some("ready"), Some("building")),
                (3, Some("building"), Some("done"))
            ]
        );
        assert_eq!((moves[1].at_ms, moves[1].actor.as_str()), (20, "b"));
        assert!(
            !dir.path().join(INTENT_FILE).exists(),
            "a commit retires the intent"
        );
    }

    #[test]
    fn pages_filter_by_time_and_continue_after_a_seq() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path(), &[]);
        for at in [5, 10, 15, 20] {
            let mut t = task(&["agent"], Status::Open);
            t.updated_ms = at;
            write(&mut store, None, &t, "a");
        }
        let page = store.page(10, 0, 2);
        assert_eq!(page.iter().map(|m| m.at_ms).collect::<Vec<_>>(), [10, 15]);
        let rest = store.page(10, page[1].seq, 2);
        assert_eq!(rest.iter().map(|m| m.at_ms).collect::<Vec<_>>(), [20]);
    }

    #[test]
    fn a_crash_after_the_task_write_replays_even_a_first_move() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path(), &[]);
        let card = task(&["agent", "board:ready"], Status::Open);
        store
            .begin(MoveStore::plan(None, &card, "a").into_iter().collect())
            .unwrap();
        drop(store); // the task landed; the process died before the commit

        let store = open(dir.path(), &[&card]);
        let moves = store.of_task(card.id);
        assert_eq!(path(&moves), [(1, None, Some("ready"))]);
        assert_eq!(moves[0].actor, "a");
        assert!(!dir.path().join(INTENT_FILE).exists());
    }

    #[test]
    fn a_crash_before_the_task_write_records_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path(), &[]);
        let ready = task(&["agent", "board:ready"], Status::Open);
        write(&mut store, None, &ready, "a");
        let qa = with_tags(&ready, &["agent", "board:qa"], 20);
        store
            .begin(
                MoveStore::plan(Some(&ready), &qa, "a")
                    .into_iter()
                    .collect(),
            )
            .unwrap();
        drop(store); // the task write never happened

        let store = open(dir.path(), &[&ready]);
        assert_eq!(path(&store.of_task(ready.id)), [(1, None, Some("ready"))]);
        assert!(!dir.path().join(INTENT_FILE).exists());
    }

    #[test]
    fn a_failed_append_refuses_writes_and_reopen_replays_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path(), &[]);
        let ready = task(&["agent", "board:ready"], Status::Open);
        write(&mut store, None, &ready, "a");
        let qa = with_tags(&ready, &["agent", "board:qa"], 20);

        store
            .begin(
                MoveStore::plan(Some(&ready), &qa, "b")
                    .into_iter()
                    .collect(),
            )
            .unwrap();
        arm_fault(Fault::InsertWritePrefix(3));
        assert!(store.commit().is_err());
        assert!(matches!(
            store.require_writable(),
            Err(TickError::RecoveryRequired)
        ));
        let review = with_tags(&qa, &["agent", "board:review"], 30);
        assert!(matches!(
            store.begin(
                MoveStore::plan(Some(&qa), &review, "b")
                    .into_iter()
                    .collect()
            ),
            Err(TickError::RecoveryRequired)
        ));
        drop(store);

        let store = open(dir.path(), &[&qa]);
        assert_eq!(
            path(&store.of_task(ready.id)),
            [(1, None, Some("ready")), (2, Some("ready"), Some("qa"))]
        );
    }

    #[test]
    fn replaying_an_intent_already_appended_does_not_duplicate_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path(), &[]);
        let card = task(&["agent", "board:ready"], Status::Open);
        let planned: Vec<Move> = MoveStore::plan(None, &card, "a").into_iter().collect();
        store.begin(planned.clone()).unwrap();
        store.commit().unwrap();
        drop(store);
        // Died after the append, before the intent was retired.
        std::fs::write(
            dir.path().join(INTENT_FILE),
            rusty_json::to_string(&planned).unwrap(),
        )
        .unwrap();

        let store = open(dir.path(), &[&card]);
        assert_eq!(store.of_task(card.id).len(), 1);
    }

    #[test]
    fn an_interrupted_first_log_header_is_cleared_and_its_move_replayed() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path(), &[]);
        let card = task(&["agent"], Status::Open);
        store
            .begin(MoveStore::plan(None, &card, "a").into_iter().collect())
            .unwrap();
        drop(store);
        // The first append died writing the log's header.
        let log = insert_log::log_path(&dir.path().join("moves.mmap"));
        std::fs::write(&log, b"").unwrap();

        let store = open(dir.path(), &[&card]);
        assert_eq!(path(&store.of_task(card.id)), [(1, None, Some("backlog"))]);
        drop(store);
        assert_eq!(open(dir.path(), &[]).page(0, 0, 10).len(), 1);
    }
}
