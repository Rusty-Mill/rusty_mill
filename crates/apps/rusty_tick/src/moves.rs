//! Column history for Mill Board: one append-only record each time a task's
//! board column changes, however it changed (a board `PATCH`, the tick UI, a
//! tag rename). The service appends it in the same locked operation as the
//! task write. A failed append (a torn log tail cannot be appended past in
//! process) refuses every later task write until the store is reopened, as the
//! task store does, so a later move never hides the gap; on open,
//! [`MoveStore::repair`] writes the missing move. Nothing is ever deleted, so a
//! `seq` is never reused. History starts when this shipped: there is no backfill.

use crate::store::TickError;
use crate::table::Table;
use crate::task::{Status, Task};
use rusty_multimodal_db_engine::generic::insert_log;
use rusty_multimodal_db_engine::generic::traits::{
    IndexedField, Record, ScannableField, SchemaTag,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use uuid::Uuid;

/// The tag that puts a task on Mill Board.
pub const BOARD_TAG: &str = "agent";
/// The tag prefix that names a task's board column.
pub const COLUMN_PREFIX: &str = "board:";
/// Mill Board's columns (`rusty_mill_board::domain::Column`, snake case).
pub const COLUMNS: [&str; 7] = [
    "backlog", "ready", "building", "qa", "review", "done", "blocked",
];
/// The column of an open card whose `board:` tag is missing or unknown.
pub const DEFAULT_COLUMN: &str = "backlog";
/// The column of a closed card.
pub const DONE_COLUMN: &str = "done";
/// Who a move is recorded as when the request names nobody.
pub const DEFAULT_ACTOR: &str = "tick";
/// Longest accepted actor, in bytes.
pub const MAX_ACTOR_BYTES: usize = 64;
/// Most moves one page returns.
pub const MAX_PAGE: usize = 1000;

/// The board column `task` is in, exactly as Mill Board's `card_of` reads it:
/// `None` off the board (no [`BOARD_TAG`]), `done` when closed, else its first
/// `board:` tag if that names a column, else `backlog`.
pub fn column_of(task: &Task) -> Option<&'static str> {
    if !task.tags.iter().any(|t| t == BOARD_TAG) {
        return None;
    }
    if task.status != Status::Open {
        return Some(DONE_COLUMN);
    }
    let tag = task.tags.iter().find_map(|t| t.strip_prefix(COLUMN_PREFIX));
    Some(
        COLUMNS
            .into_iter()
            .find(|c| Some(*c) == tag)
            .unwrap_or(DEFAULT_COLUMN),
    )
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

pub struct MoveStore {
    table: Table<Move, ByTask, AtMs>,
    next_seq: u64,
    /// Moves queued by [`MoveStore::queue`], written by [`MoveStore::flush`].
    pending: Vec<Move>,
    /// An append failed: refuse writes until reopened.
    failed: bool,
}

impl MoveStore {
    pub fn open(dir: &Path) -> Result<Self, TickError> {
        let path = dir.join("moves.mmap");
        // A log whose creation was interrupted (empty, or a prefix of its header)
        // holds no move, and history is repaired from the tasks, so clearing it
        // loses nothing and lets the store open.
        if path.exists() {
            insert_log::clear_interrupted_creation::<Move>(&path)?;
        }
        let table: Table<Move, ByTask, AtMs> = Table::open(dir, "moves.mmap")?;
        let next_seq = table.all().iter().map(|m| m.seq + 1).max().unwrap_or(1);
        Ok(Self {
            table,
            next_seq,
            pending: Vec::new(),
            failed: false,
        })
    }

    /// Append the move from `old` to `new` (`None` for a task being created),
    /// if the column changed; returns whether it did.
    pub fn record(
        &mut self,
        old: Option<&Task>,
        new: &Task,
        actor: &str,
    ) -> Result<bool, TickError> {
        let changed = self.queue(old, new, actor);
        self.flush()?;
        Ok(changed)
    }

    /// Queue the move from `old` to `new` without writing it, if the column
    /// changed; returns whether it did.
    pub fn queue(&mut self, old: Option<&Task>, new: &Task, actor: &str) -> bool {
        let from = old.and_then(column_of);
        let to = column_of(new);
        if from == to {
            return false;
        }
        self.pending
            .push(Move::new(new.id, from, to, new.updated_ms, actor));
        true
    }

    /// Write every queued move, in order. After a failed append nothing more is
    /// written until the store is reopened.
    pub fn flush(&mut self) -> Result<(), TickError> {
        self.require_writable()?;
        for entry in std::mem::take(&mut self.pending) {
            let entry = Move {
                seq: self.next_seq,
                ..entry
            };
            if let Err(error) = self.table.insert(entry) {
                self.failed = true;
                return Err(error);
            }
            self.next_seq += 1;
        }
        Ok(())
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

    /// Make the history agree with `tasks` after a crash between a task write and
    /// its move: a task whose last recorded column is not its column now gets the
    /// missing move, at its `updated_ms`, as [`DEFAULT_ACTOR`]. Tasks with no
    /// history are left alone (no backfill). Returns the number of moves added.
    pub fn repair(&mut self, tasks: &[Task]) -> Result<usize, TickError> {
        let mut all = self.table.all();
        all.sort_by_key(|m| m.seq);
        // In seq order, so each task keeps its latest move.
        let last: BTreeMap<Uuid, Move> = all.into_iter().map(|m| (m.task_id, m)).collect();
        for task in tasks {
            let Some(recorded) = last.get(&task.id) else {
                continue;
            };
            let now = column_of(task);
            if recorded.to.as_deref() != now {
                self.pending.push(Move::new(
                    task.id,
                    recorded.to.as_deref(),
                    now,
                    task.updated_ms,
                    DEFAULT_ACTOR,
                ));
            }
        }
        let added = self.pending.len();
        self.flush()?;
        Ok(added)
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

    fn path(moves: &[Move]) -> Vec<(u64, Option<&str>, Option<&str>)> {
        moves
            .iter()
            .map(|m| (m.seq, m.from.as_deref(), m.to.as_deref()))
            .collect()
    }

    #[test]
    fn the_column_is_read_as_mill_board_reads_it() {
        let column = |tags: &[&str], status| column_of(&task(tags, status));
        assert_eq!(column(&["agent", "board:qa"], Status::Open), Some("qa"));
        assert_eq!(column(&["agent", "board:qa"], Status::WontDo), Some("done"));
        assert_eq!(column(&["agent"], Status::Open), Some("backlog"));
        assert_eq!(
            column(&["agent", "board:typo"], Status::Open),
            Some("backlog")
        );
        assert_eq!(
            column(&["agent", "board:typo", "board:qa"], Status::Open),
            Some("backlog")
        );
        assert_eq!(column(&["agent"], Status::Done), Some("done"));
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
    fn only_a_column_change_is_recorded_and_seq_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = MoveStore::open(dir.path()).unwrap();
        let ready = task(&["agent", "board:ready"], Status::Open);
        let mut edited = ready.clone();
        edited.title = "renamed".into();
        let mut building = ready.clone();
        building.tags = vec!["agent".into(), "board:building".into()];
        building.updated_ms = 20;

        assert!(store.record(None, &ready, "a").unwrap());
        assert!(!store.record(Some(&ready), &edited, "a").unwrap());
        assert!(store.record(Some(&ready), &building, "b").unwrap());
        drop(store);

        let mut store = MoveStore::open(dir.path()).unwrap();
        let mut done = building.clone();
        done.status = Status::Done;
        store.record(Some(&building), &done, "c").unwrap();
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
    }

    #[test]
    fn pages_filter_by_time_and_continue_after_a_seq() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = MoveStore::open(dir.path()).unwrap();
        for at in [5, 10, 15, 20] {
            let mut t = task(&["agent"], Status::Open);
            t.updated_ms = at;
            store.record(None, &t, "a").unwrap();
        }
        let page = store.page(10, 0, 2);
        assert_eq!(page.iter().map(|m| m.at_ms).collect::<Vec<_>>(), [10, 15]);
        let rest = store.page(10, page[1].seq, 2);
        assert_eq!(rest.iter().map(|m| m.at_ms).collect::<Vec<_>>(), [20]);
    }

    #[test]
    fn a_failed_append_refuses_writes_and_reopen_repairs_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = MoveStore::open(dir.path()).unwrap();
        let ready = task(&["agent", "board:ready"], Status::Open);
        store.record(None, &ready, "a").unwrap();
        let mut qa = ready.clone();
        qa.tags = vec!["agent".into(), "board:qa".into()];

        arm_fault(Fault::InsertWritePrefix(3));
        assert!(store.record(Some(&ready), &qa, "a").is_err());
        assert!(matches!(
            store.require_writable(),
            Err(TickError::RecoveryRequired)
        ));
        assert!(matches!(
            store.record(Some(&qa), &ready, "a"),
            Err(TickError::RecoveryRequired)
        ));
        drop(store);

        let mut store = MoveStore::open(dir.path()).unwrap();
        assert_eq!(store.repair(&[qa.clone()]).unwrap(), 1);
        assert_eq!(
            path(&store.of_task(ready.id)),
            [(1, None, Some("ready")), (2, Some("ready"), Some("qa"))]
        );
    }

    #[test]
    fn an_interrupted_log_creation_does_not_stop_the_store_opening() {
        let dir = tempfile::tempdir().unwrap();
        drop(MoveStore::open(dir.path()).unwrap());
        let log = insert_log::log_path(&dir.path().join("moves.mmap"));
        std::fs::write(&log, b"").unwrap();
        let mut store = MoveStore::open(dir.path()).unwrap();
        store
            .record(None, &task(&["agent"], Status::Open), "a")
            .unwrap();
        drop(store);
        assert_eq!(MoveStore::open(dir.path()).unwrap().page(0, 0, 10).len(), 1);
    }

    #[test]
    fn repair_adds_a_missing_move_but_never_backfills() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = MoveStore::open(dir.path()).unwrap();
        let tracked = task(&["agent", "board:ready"], Status::Open);
        store.record(None, &tracked, "a").unwrap();
        // The task write landed; the move did not.
        let mut moved = tracked.clone();
        moved.tags = vec!["agent".into(), "board:qa".into()];
        moved.updated_ms = 99;
        let untracked = task(&["agent", "board:review"], Status::Open);

        assert_eq!(
            store.repair(&[moved.clone(), untracked.clone()]).unwrap(),
            1
        );
        let last = store.of_task(moved.id).pop().unwrap();
        assert_eq!(
            (
                last.from.as_deref(),
                last.to.as_deref(),
                last.at_ms,
                last.actor.as_str()
            ),
            (Some("ready"), Some("qa"), 99, DEFAULT_ACTOR)
        );
        assert!(store.of_task(untracked.id).is_empty());
        assert_eq!(store.repair(&[moved, untracked]).unwrap(), 0);
    }
}
