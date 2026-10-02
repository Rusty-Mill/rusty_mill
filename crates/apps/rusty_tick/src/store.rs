//! `TaskStore`: tasks on the engine, with the indexes a task manager needs.
//!
//! The durable core is a `GenericMmapStore` indexed by list; two `Ordered`
//! layers add sorted `(list, sort_order)` and `(list, due)` indexes. Full text
//! and tags are derived in memory at open, as `rusty_remind_me` does.

use crate::task::{ByList, DueAt, SortOrder, Task};
use rusty_multimodal_db_engine::durability::DurabilityError;
use rusty_multimodal_db_engine::fulltext::{FullTextIndex, Query};
use rusty_multimodal_db_engine::generic::query::{
    AllIds, Delete, FilterEq, GetById, Insert, RangeBy, Replace, UpdateField,
};
use rusty_multimodal_db_engine::generic::store::Ordered;
use rusty_multimodal_db_engine::generic::{
    DeleteError, GenericMmapStore, InsertError, ReplaceError,
};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Gap between appended tasks, leaving room to drop one in between.
pub const SORT_STEP: i64 = 1024;

type Core = GenericMmapStore<Task, ByList, SortOrder>;
type Stack = Ordered<Ordered<Core, Task, SortOrder>, Task, DueAt>;

#[derive(Debug, thiserror::Error)]
pub enum TickError {
    #[error("storage: {0}")]
    Storage(#[from] DurabilityError),
    #[error("task {0} already exists")]
    Duplicate(Uuid),
    #[error("task {0} not found")]
    NotFound(Uuid),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("unreadable pending batch {0}: {1}")]
    Pending(PathBuf, String),
}

/// A batch of whole-task writes not yet known to have landed (see
/// [`TaskStore::replace_all`]).
const PENDING: &str = "tasks.pending.json";

pub struct TaskStore {
    stack: Stack,
    pending: PathBuf,
    search: FullTextIndex<Uuid, 2>,
    tags: BTreeMap<String, BTreeSet<Uuid>>,
}

impl TaskStore {
    /// Open the store in `dir`, creating it when absent.
    pub fn open(dir: &Path) -> Result<Self, TickError> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join("tasks.mmap");
        let core: Core = if path.exists() {
            GenericMmapStore::open_portable(&path)?
        } else {
            GenericMmapStore::create(Vec::new(), &path)?
        };
        let stack = Ordered::new(Ordered::new(core));
        let mut store = Self {
            stack,
            pending: dir.join(PENDING),
            search: FullTextIndex::new(),
            tags: BTreeMap::new(),
        };
        for id in store.stack.all_ids() {
            if let Some(task) = store.stack.get(id) {
                store.index(&task);
            }
        }
        store.finish_pending()?;
        Ok(store)
    }

    /// Complete a batch a crash interrupted. A task whose version moved on
    /// since (a later write landed and the batch file outlived it) or that is
    /// gone is left alone, so replaying never rolls anything back.
    fn finish_pending(&mut self) -> Result<(), TickError> {
        let text = match std::fs::read_to_string(&self.pending) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        let batch: Vec<Task> = rusty_json::from_str(&text)
            .map_err(|e| TickError::Pending(self.pending.clone(), e.to_string()))?;
        let unfinished: Vec<Task> = batch
            .into_iter()
            .filter(|task| {
                self.stack
                    .get(task.id)
                    .is_some_and(|stored| stored.version == task.version.wrapping_sub(1))
            })
            .collect();
        self.apply(&unfinished)?;
        std::fs::remove_file(&self.pending)?;
        Ok(())
    }

    /// Trashed tasks are searchable and taggable nowhere: they drop out of both
    /// derived indexes and come back when restored.
    fn index(&mut self, task: &Task) {
        if task.is_deleted() {
            return;
        }
        self.search.upsert(task.id, [&task.title, &task.notes]);
        for tag in &task.tags {
            self.tags.entry(tag.clone()).or_default().insert(task.id);
        }
    }

    fn unindex(&mut self, task: &Task) {
        self.search.remove(&task.id);
        for tag in &task.tags {
            if let Some(ids) = self.tags.get_mut(tag) {
                ids.remove(&task.id);
            }
        }
    }

    pub fn insert(&mut self, task: Task) -> Result<(), TickError> {
        let id = task.id;
        match self.stack.insert(task.clone()) {
            Ok(()) => {}
            Err(InsertError::Duplicate(_)) => return Err(TickError::Duplicate(id)),
            Err(InsertError::Durability(e)) => return Err(TickError::Storage(e)),
        }
        self.index(&task);
        Ok(())
    }

    pub fn replace(&mut self, task: Task) -> Result<(), TickError> {
        let id = task.id;
        let old = self.stack.get(id).ok_or(TickError::NotFound(id))?;
        match self.stack.replace(task.clone()) {
            Ok(()) => {}
            Err(ReplaceError::NotFound(_)) => return Err(TickError::NotFound(id)),
            Err(ReplaceError::Durability(e)) => return Err(TickError::Storage(e)),
        }
        self.unindex(&old);
        self.index(&task);
        Ok(())
    }

    /// Replace several tasks as one write: after a crash, either none landed
    /// or the next [`open`](Self::open) lands the rest. The batch is saved
    /// (fsynced) beside the store first and removed once every task is in.
    /// Each task's `version` must be one past the stored one, as a save
    /// leaves it; that is how a replay tells an unfinished write from one a
    /// later write superseded.
    pub fn replace_all(&mut self, tasks: &[Task]) -> Result<(), TickError> {
        match tasks {
            [] => return Ok(()),
            [task] => return self.replace(task.clone()),
            _ => {}
        }
        if let Some(missing) = tasks.iter().find(|t| self.stack.get(t.id).is_none()) {
            return Err(TickError::NotFound(missing.id));
        }
        let json = rusty_json::to_string(&tasks)
            .map_err(|e| TickError::Pending(self.pending.clone(), e.to_string()))?;
        rusty_atomic_file::write(&self.pending, json.as_bytes())?;
        self.apply(tasks)?;
        std::fs::remove_file(&self.pending)?;
        Ok(())
    }

    fn apply(&mut self, tasks: &[Task]) -> Result<(), TickError> {
        for task in tasks {
            self.replace(task.clone())?;
        }
        Ok(())
    }

    pub fn delete(&mut self, id: Uuid) -> Result<(), TickError> {
        let old = self.stack.get(id).ok_or(TickError::NotFound(id))?;
        match self.stack.delete(id) {
            Ok(()) => {}
            Err(DeleteError::NotFound(_)) => return Err(TickError::NotFound(id)),
            Err(DeleteError::Durability(e)) => return Err(TickError::Storage(e)),
        }
        self.unindex(&old);
        Ok(())
    }

    pub fn get(&self, id: Uuid) -> Option<Task> {
        self.stack.get(id)
    }

    /// Every task, trashed ones included, unspecified order.
    pub fn all(&self) -> Vec<Task> {
        self.stack
            .all_ids()
            .into_iter()
            .filter_map(|id| self.stack.get(id))
            .collect()
    }

    /// Every task in `list`, in the store's own (unspecified) order.
    pub fn in_list(&self, list: Uuid) -> Vec<Task> {
        FilterEq::<Task, ByList>::filter_eq(&self.stack, &list)
            .into_iter()
            .filter_map(|id| self.stack.get(id))
            .collect()
    }

    /// Tasks of `list` due within `[from, to)`, soonest first (a smart list).
    pub fn due_between(&self, list: Uuid, from: i64, to: i64) -> Vec<Task> {
        RangeBy::<Task, DueAt>::range_by(
            &self.stack,
            Bound::Included(((list, from), Uuid::nil())),
            Bound::Excluded(((list, to), Uuid::nil())),
        )
        .into_iter()
        .filter_map(|id| self.stack.get(id))
        .collect()
    }

    /// Tasks of `list` in manual order.
    pub fn in_manual_order(&self, list: Uuid) -> Vec<Task> {
        // Only the outermost `Ordered` answers `RangeBy`; the inner layer's
        // index is reached through `inner()` (finding F2 in SPIKE-FINDINGS.md).
        RangeBy::<Task, SortOrder>::range_by(
            self.stack.inner(),
            Bound::Included(((list, i64::MIN), Uuid::nil())),
            Bound::Included(((list, i64::MAX), Uuid::max())),
        )
        .into_iter()
        .filter_map(|id| self.stack.get(id))
        .collect()
    }

    /// The sort order that appends a task after every task in `list`.
    pub fn next_sort_order(&self, list: Uuid) -> i64 {
        self.in_manual_order(list)
            .last()
            .map_or(0, |t| t.sort_order.saturating_add(SORT_STEP))
    }

    /// Move one task to `sort_order`: rewrites one 8-byte slot, not the record.
    pub fn reorder(&mut self, id: Uuid, sort_order: i64) -> Result<(), TickError> {
        UpdateField::<Task, SortOrder>::update(&mut self.stack, id, sort_order)
            .map_err(|_| TickError::NotFound(id))
    }

    /// Ids matching any of `phrases` in title or notes, best match first. The
    /// last word of each phrase matches as a prefix, so `grocer` finds
    /// `groceries`: search as you type.
    pub fn search(&self, phrases: &[&str]) -> Vec<Uuid> {
        let mut hits = self
            .search
            .search(&Query::any_of_prefix(phrases.iter().copied()));
        hits.sort_by(|a, b| a.score.total_cmp(&b.score));
        hits.into_iter().map(|h| h.key).collect()
    }

    pub fn with_tag(&self, tag: &str) -> Vec<Uuid> {
        self.tags
            .get(tag)
            .map(|ids| ids.iter().copied().collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(title: &str) -> Task {
        Task::new(Uuid::now_v7(), Uuid::nil(), title, 0, 0)
    }

    /// `tasks` as a save leaves them: trashed, one version on.
    fn trashed(tasks: &[Task]) -> Vec<Task> {
        tasks
            .iter()
            .cloned()
            .map(|mut t| {
                t.deleted_ms = Some(7);
                t.version += 1;
                t
            })
            .collect()
    }

    /// What a crash inside `replace_all` leaves: the batch file written and
    /// only the first `landed` tasks in.
    fn crash_mid_batch(store: &mut TaskStore, batch: &[Task], landed: usize) {
        let json = rusty_json::to_string(&batch).unwrap();
        rusty_atomic_file::write(&store.pending, json.as_bytes()).unwrap();
        store.apply(&batch[..landed]).unwrap();
    }

    fn seeded(dir: &Path, n: usize) -> (TaskStore, Vec<Task>) {
        let mut store = TaskStore::open(dir).unwrap();
        let tasks: Vec<Task> = (0..n).map(|i| task(&format!("t{i}"))).collect();
        for t in &tasks {
            store.insert(t.clone()).unwrap();
        }
        (store, tasks)
    }

    #[test]
    fn a_batch_lands_whole_and_leaves_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, tasks) = seeded(dir.path(), 3);
        store.replace_all(&trashed(&tasks)).unwrap();
        assert!(store.all().iter().all(Task::is_deleted));
        assert!(!dir.path().join(PENDING).exists());
    }

    #[test]
    fn an_interrupted_batch_is_finished_on_open() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, tasks) = seeded(dir.path(), 3);
        crash_mid_batch(&mut store, &trashed(&tasks), 1);
        drop(store);

        let store = TaskStore::open(dir.path()).unwrap();
        assert!(
            store.all().iter().all(Task::is_deleted),
            "every task trashed, not just the first"
        );
        assert!(!dir.path().join(PENDING).exists());
    }

    #[test]
    fn a_batch_file_left_behind_never_rolls_back_a_later_write() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, tasks) = seeded(dir.path(), 2);
        let batch = trashed(&tasks);
        // Every task landed, but the file was not removed.
        crash_mid_batch(&mut store, &batch, batch.len());
        let mut later = batch[0].clone();
        later.deleted_ms = None;
        later.version += 1;
        store.replace(later.clone()).unwrap();
        store.delete(batch[1].id).unwrap();
        drop(store);

        let store = TaskStore::open(dir.path()).unwrap();
        assert_eq!(store.get(later.id), Some(later), "the later restore stands");
        assert_eq!(store.get(batch[1].id), None, "the later delete stands");
        assert!(!dir.path().join(PENDING).exists());
    }

    #[test]
    fn a_batch_naming_a_missing_task_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, tasks) = seeded(dir.path(), 1);
        let mut batch = trashed(&tasks);
        batch.push(task("never inserted"));
        assert!(matches!(
            store.replace_all(&batch),
            Err(TickError::NotFound(_))
        ));
        assert_eq!(store.get(tasks[0].id), Some(tasks[0].clone()));
        assert!(!dir.path().join(PENDING).exists());
    }

    #[test]
    fn an_unreadable_batch_file_fails_the_open() {
        let dir = tempfile::tempdir().unwrap();
        drop(seeded(dir.path(), 1));
        std::fs::write(dir.path().join(PENDING), "{not json").unwrap();
        assert!(matches!(
            TaskStore::open(dir.path()),
            Err(TickError::Pending(..))
        ));
    }
}
