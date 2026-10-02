//! `TaskStore`: tasks on the engine, with the indexes a task manager needs.
//!
//! The durable core is a `GenericMmapStore` indexed by list; two `Ordered`
//! layers add sorted `(list, sort_order)` and `(list, due)` indexes. Full text
//! and tags are derived in memory at open, as `rusty_remind_me` does.

use crate::task::{ByList, DueAt, SortOrder, Task};
use rusty_multimodal_db_engine::codec;
use rusty_multimodal_db_engine::durability::DurabilityError;
use rusty_multimodal_db_engine::fulltext::{FullTextIndex, Query};
use rusty_multimodal_db_engine::generic::query::{
    AllIds, Delete, FilterEq, GetById, Insert, RangeBy, Replace, UpdateField,
};
use rusty_multimodal_db_engine::generic::store::GroupCommit;
use rusty_multimodal_db_engine::generic::store::Ordered;
use rusty_multimodal_db_engine::generic::{
    DeleteError, GenericMmapStore, InsertError, ReplaceError,
};
use rusty_multimodal_db_engine::journal::{Batch, Journal, JournalError};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;
use std::path::Path;
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
    #[error("task journal: {0}")]
    Journal(#[from] JournalError),
}

pub struct TaskStore {
    stack: Stack,
    search: FullTextIndex<Uuid, 2>,
    tags: BTreeMap<String, BTreeSet<Uuid>>,
    journal: Journal,
}

const TASKS: &str = "tasks";

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
        let mut stack = Ordered::new(Ordered::new(core));
        let opened = Journal::open(&dir.join("tasks.journal"))?;
        for batch in &opened.replay {
            Self::apply_batch(&mut stack, batch, usize::MAX)?;
        }
        if !opened.replay.is_empty() {
            stack.commit()?;
        }
        let mut journal = opened.journal;
        journal.checkpoint()?;
        let mut store = Self {
            stack,
            search: FullTextIndex::new(),
            tags: BTreeMap::new(),
            journal,
        };
        for id in store.stack.all_ids() {
            if let Some(task) = store.stack.get(id) {
                store.index(&task);
            }
        }
        Ok(store)
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

    /// Durably replace a related set of tasks as one crash-atomic operation.
    ///
    /// The redo entry is synced before any record changes. Derived indexes are
    /// published only after all record writes are durable; an interruption
    /// before the checkpoint replays the complete set on the next open.
    pub fn replace_batch(&mut self, tasks: Vec<Task>) -> Result<(), TickError> {
        let mut old = Vec::with_capacity(tasks.len());
        let mut batch = Batch::default();
        for task in &tasks {
            let previous = self
                .stack
                .get(task.id)
                .ok_or(TickError::NotFound(task.id))?;
            old.push(previous);
            batch.put(
                TASKS,
                task.id.as_bytes().to_vec(),
                codec::encode(task).map_err(|e| JournalError::Encode(e.to_string()))?,
            );
        }

        self.journal.commit(&batch)?;
        self.stack.defer_sync();
        Self::apply_batch(&mut self.stack, &batch, usize::MAX)?;
        self.stack.commit()?;
        for task in &old {
            self.unindex(task);
        }
        for task in &tasks {
            self.index(task);
        }
        self.journal.checkpoint()?;
        Ok(())
    }

    fn apply_batch(stack: &mut Stack, batch: &Batch, limit: usize) -> Result<(), TickError> {
        for change in batch.changes.iter().take(limit) {
            if change.store != TASKS {
                return Err(JournalError::Encode(format!(
                    "unexpected store {:?} in task journal",
                    change.store
                ))
                .into());
            }
            let id = Uuid::from_slice(&change.key)
                .map_err(|e| JournalError::Encode(format!("invalid task id: {e}")))?;
            let bytes = change.value.as_ref().ok_or_else(|| {
                JournalError::Encode("task journal contains a deletion".to_string())
            })?;
            let task: Task = codec::decode(bytes)
                .map_err(|e| JournalError::Encode(format!("invalid task value: {e}")))?;
            if task.id != id {
                return Err(
                    JournalError::Encode("task journal key/value mismatch".to_string()).into(),
                );
            }
            match stack.replace(task) {
                Ok(()) => {}
                Err(ReplaceError::NotFound(_)) => return Err(TickError::NotFound(id)),
                Err(ReplaceError::Durability(e)) => return Err(TickError::Storage(e)),
            }
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
    use crate::task::Task;
    use tempfile::TempDir;

    fn task(n: u128, list: Uuid, title: &str) -> Task {
        let mut task = Task::new(Uuid::from_u128(n), list, title, n as i64 * SORT_STEP, 10);
        task.tags = vec![format!("tag-{n}")];
        task
    }

    fn trash_batch(tasks: &[Task], at: i64) -> (Vec<Task>, Batch) {
        let changed: Vec<Task> = tasks
            .iter()
            .cloned()
            .map(|mut task| {
                task.deleted_ms.get_or_insert(at);
                task.updated_ms = at;
                task.version += 1;
                task
            })
            .collect();
        let mut batch = Batch::default();
        for task in &changed {
            batch.put(
                TASKS,
                task.id.as_bytes().to_vec(),
                codec::encode(task).unwrap(),
            );
        }
        (changed, batch)
    }

    fn fixture() -> (TempDir, Vec<Task>, Task) {
        let dir = TempDir::new().unwrap();
        let list = Uuid::from_u128(99);
        let mut parent = task(1, list, "parent needle");
        let mut child = task(2, list, "first child");
        child.parent_id = Some(parent.id);
        let mut already_trashed = task(3, list, "old child");
        already_trashed.parent_id = Some(parent.id);
        already_trashed.deleted_ms = Some(7);
        let unrelated = task(4, list, "unrelated needle");
        parent.notes = "parent notes".into();
        let mut store = TaskStore::open(dir.path()).unwrap();
        for task in [&parent, &child, &already_trashed, &unrelated] {
            store.insert(task.clone()).unwrap();
        }
        drop(store);
        (dir, vec![child, already_trashed, parent], unrelated)
    }

    fn assert_replayed(dir: &Path, originals: &[Task], unrelated: &Task, at: i64) {
        let store = TaskStore::open(dir).unwrap();
        for original in originals {
            let changed = store.get(original.id).unwrap();
            assert_eq!(changed.deleted_ms, original.deleted_ms.or(Some(at)));
            assert_eq!(changed.version, original.version + 1);
            assert!(store.search(&[&original.title]).is_empty());
            assert!(store.with_tag(&original.tags[0]).is_empty());
        }
        assert_eq!(store.get(unrelated.id).unwrap(), *unrelated);
        assert_eq!(store.search(&["unrelated"]), vec![unrelated.id]);
        assert_eq!(store.with_tag(&unrelated.tags[0]), vec![unrelated.id]);
        assert_eq!(store.in_manual_order(unrelated.list_id).len(), 4);
    }

    #[test]
    fn every_accepted_crash_prefix_replays_the_whole_trash_batch() {
        for applied in 0..=3 {
            let (dir, originals, unrelated) = fixture();
            let (changed, batch) = trash_batch(&originals, 50);
            let mut store = TaskStore::open(dir.path()).unwrap();
            store.journal.commit(&batch).unwrap();
            store.stack.defer_sync();
            TaskStore::apply_batch(&mut store.stack, &batch, applied).unwrap();
            if applied == changed.len() {
                store.stack.commit().unwrap();
            }
            // Crash before checkpoint: no derived-index publication is
            // simulated, and the accepted redo entry remains authoritative.
            drop(store);
            assert_replayed(dir.path(), &originals, &unrelated, 50);
        }
    }

    #[test]
    fn an_injected_precommit_refusal_leaves_the_previous_state() {
        let (dir, originals, unrelated) = fixture();
        let (_changed, _uncommitted) = trash_batch(&originals, 50);
        // Inject the failure immediately before Journal::commit by dropping
        // the prepared operation. No record or derived index was published.
        let store = TaskStore::open(dir.path()).unwrap();
        for original in &originals {
            assert_eq!(store.get(original.id).unwrap(), *original);
            if !original.is_deleted() {
                assert_eq!(store.search(&[&original.title]), vec![original.id]);
            }
        }
        assert_eq!(store.get(unrelated.id).unwrap(), unrelated);
        drop(store);
        let reopened = TaskStore::open(dir.path()).unwrap();
        for original in &originals {
            assert_eq!(reopened.get(original.id).unwrap(), *original);
        }
    }

    #[test]
    fn committed_batches_are_replay_idempotent_and_support_no_children() {
        let dir = TempDir::new().unwrap();
        let list = Uuid::from_u128(99);
        let parent = task(1, list, "only task");
        let mut store = TaskStore::open(dir.path()).unwrap();
        store.insert(parent.clone()).unwrap();
        let (changed, batch) = trash_batch(std::slice::from_ref(&parent), 50);
        store.journal.commit(&batch).unwrap();
        store.stack.defer_sync();
        TaskStore::apply_batch(&mut store.stack, &batch, usize::MAX).unwrap();
        store.stack.commit().unwrap();
        // Leave the entry for replay after every change already landed.
        drop(store);
        let once = TaskStore::open(dir.path()).unwrap();
        assert_eq!(once.get(parent.id).unwrap(), changed[0]);
        drop(once);
        let twice = TaskStore::open(dir.path()).unwrap();
        assert_eq!(twice.get(parent.id).unwrap(), changed[0]);
    }
}
