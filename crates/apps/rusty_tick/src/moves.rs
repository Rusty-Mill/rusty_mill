//! Column history for Mill Board: one append-only record each time a task's
//! board column changes, however it changed (a board `PATCH`, the tick UI, a
//! tag rename). The service appends it in the same locked operation as the
//! task write; [`MoveStore::repair`] closes the gap a crash between the two
//! leaves. History starts when this shipped: there is no backfill.

use crate::store::TickError;
use crate::table::Table;
use crate::task::{Status, Task};
use rusty_multimodal_db_engine::generic::traits::{
    IndexedField, Record, ScannableField, SchemaTag,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use uuid::Uuid;

/// The tag prefix that names a task's board column.
pub const COLUMN_PREFIX: &str = "board:";
/// The column of a closed task that is on the board.
pub const DONE_COLUMN: &str = "done";
/// Who a move is recorded as when the request names nobody.
pub const DEFAULT_ACTOR: &str = "tick";
/// Longest accepted actor, in bytes.
pub const MAX_ACTOR_BYTES: usize = 64;
/// Most moves one page returns.
pub const MAX_PAGE: usize = 1000;

/// The board column `task` is in, as Mill Board reads it: `done` when closed,
/// else its `board:` tag; `None` when it carries no `board:` tag.
pub fn column_of(task: &Task) -> Option<String> {
    let tag = task
        .tags
        .iter()
        .find_map(|t| t.strip_prefix(COLUMN_PREFIX))?;
    Some(match task.status {
        Status::Open => tag.to_string(),
        Status::Done | Status::WontDo => DONE_COLUMN.to_string(),
    })
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
}

impl MoveStore {
    pub fn open(dir: &Path) -> Result<Self, TickError> {
        let table: Table<Move, ByTask, AtMs> = Table::open(dir, "moves.mmap")?;
        let next_seq = table.all().iter().map(|m| m.seq + 1).max().unwrap_or(1);
        Ok(Self { table, next_seq })
    }

    /// Append the move from `old` to `new` (`None` for a task being created),
    /// if the column changed; returns whether it did.
    pub fn record(
        &mut self,
        old: Option<&Task>,
        new: &Task,
        actor: &str,
    ) -> Result<bool, TickError> {
        let from = old.and_then(column_of);
        let to = column_of(new);
        if from == to {
            return Ok(false);
        }
        self.append(new.id, from, to, new.updated_ms, actor)?;
        Ok(true)
    }

    fn append(
        &mut self,
        task_id: Uuid,
        from: Option<String>,
        to: Option<String>,
        at_ms: i64,
        actor: &str,
    ) -> Result<(), TickError> {
        let entry = Move {
            seq: self.next_seq,
            task_id,
            from,
            to,
            at_ms,
            actor: actor.to_string(),
        };
        self.table.insert(entry)?;
        self.next_seq += 1;
        Ok(())
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

    /// Delete the moves of `tasks` (purged for good); returns how many went.
    pub fn delete_of(&mut self, tasks: &[Uuid]) -> Result<usize, TickError> {
        let mut gone = 0;
        for task in tasks {
            for entry in self.table.with_index(task) {
                self.table.delete(entry.id())?;
                gone += 1;
            }
        }
        Ok(gone)
    }

    /// Make the history agree with `tasks` after a crash between a task write and
    /// its move: a task whose last recorded column is not its column now gets the
    /// missing move, at its `updated_ms`, as [`DEFAULT_ACTOR`]. Tasks with no
    /// history are left alone (no backfill), and the moves of tasks gone for good
    /// (a purge interrupted before its history) are deleted. Returns the number
    /// of moves added and deleted.
    pub fn repair(&mut self, tasks: &[Task]) -> Result<usize, TickError> {
        let mut all = self.table.all();
        all.sort_by_key(|m| m.seq);
        // In seq order, so each task keeps its latest move.
        let last: BTreeMap<Uuid, Move> = all.into_iter().map(|m| (m.task_id, m)).collect();
        let live: HashSet<Uuid> = tasks.iter().map(|t| t.id).collect();
        let orphans: Vec<Uuid> = last
            .keys()
            .copied()
            .filter(|id| !live.contains(id))
            .collect();
        let mut changed = self.delete_of(&orphans)?;
        for task in tasks {
            let Some(recorded) = last.get(&task.id) else {
                continue;
            };
            let now = column_of(task);
            if recorded.to != now {
                self.append(
                    task.id,
                    recorded.to.clone(),
                    now,
                    task.updated_ms,
                    DEFAULT_ACTOR,
                )?;
                changed += 1;
            }
        }
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(tags: &[&str], status: Status) -> Task {
        let mut task = Task::new(Uuid::now_v7(), Uuid::nil(), "t", 0, 10);
        task.tags = tags.iter().map(|t| t.to_string()).collect();
        task.status = status;
        task
    }

    #[test]
    fn the_column_is_the_board_tag_or_done_when_closed() {
        assert_eq!(
            column_of(&task(&["agent", "board:qa"], Status::Open)).as_deref(),
            Some("qa")
        );
        assert_eq!(
            column_of(&task(&["board:qa"], Status::Done)).as_deref(),
            Some("done")
        );
        assert_eq!(
            column_of(&task(&["board:qa"], Status::WontDo)).as_deref(),
            Some("done")
        );
        assert_eq!(column_of(&task(&["agent"], Status::Done)), None);
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
        let ready = task(&["board:ready"], Status::Open);
        let mut edited = ready.clone();
        edited.title = "renamed".into();
        let mut building = ready.clone();
        building.tags = vec!["board:building".into()];
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
        let path: Vec<_> = moves
            .iter()
            .map(|m| (m.seq, m.from.as_deref(), m.to.as_deref()))
            .collect();
        assert_eq!(
            path,
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
            let mut t = task(&["board:ready"], Status::Open);
            t.updated_ms = at;
            store.record(None, &t, "a").unwrap();
        }
        let page = store.page(10, 0, 2);
        assert_eq!(page.iter().map(|m| m.at_ms).collect::<Vec<_>>(), [10, 15]);
        let rest = store.page(10, page[1].seq, 2);
        assert_eq!(rest.iter().map(|m| m.at_ms).collect::<Vec<_>>(), [20]);
    }

    #[test]
    fn repair_adds_a_missing_move_and_drops_orphans_but_never_backfills() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = MoveStore::open(dir.path()).unwrap();
        let tracked = task(&["board:ready"], Status::Open);
        let purged = task(&["board:ready"], Status::Open);
        store.record(None, &tracked, "a").unwrap();
        store.record(None, &purged, "a").unwrap();
        // The task write landed; the move did not.
        let mut moved = tracked.clone();
        moved.tags = vec!["board:qa".into()];
        moved.updated_ms = 99;
        let untracked = task(&["board:review"], Status::Open);

        assert_eq!(
            store.repair(&[moved.clone(), untracked.clone()]).unwrap(),
            2
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
        assert!(store.of_task(purged.id).is_empty());
        assert!(store.of_task(untracked.id).is_empty());
        assert_eq!(store.repair(&[moved, untracked]).unwrap(), 0);
    }
}
