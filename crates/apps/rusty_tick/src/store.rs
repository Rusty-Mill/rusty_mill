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
    insert_log, DeleteError, GenericMmapStore, InsertError, ReplaceError,
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
    #[error("task store requires reopen after an interrupted atomic trash operation")]
    RecoveryRequired,
}

pub struct TaskStore {
    stack: Stack,
    search: FullTextIndex<Uuid, 2>,
    tags: BTreeMap<String, BTreeSet<Uuid>>,
    journal: Journal,
    // A complete pre-operation view retained after an ambiguous failure. It
    // prevents reads from observing a partly applied redo; all writes are
    // refused until the owner reopens and replays the journal.
    recovery_view: Option<BTreeMap<Uuid, Task>>,
}

const TASKS: &str = "tasks";

impl TaskStore {
    /// Open the store in `dir`, creating it when absent.
    pub fn open(dir: &Path) -> Result<Self, TickError> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join("tasks.mmap");
        let opened = Journal::open(&dir.join("tasks.journal"))?;
        if !opened.replay.is_empty() {
            insert_log::clear_interrupted_creation::<Task>(&path)?;
        }
        let core: Core = if path.exists() {
            GenericMmapStore::open_portable(&path)?
        } else {
            GenericMmapStore::create(Vec::new(), &path)?
        };
        let mut stack = Ordered::new(Ordered::new(core));
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
            recovery_view: None,
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
        self.require_writable()?;
        let before = self.snapshot();
        let id = task.id;
        match self.stack.insert(task.clone()) {
            Ok(()) => {}
            Err(InsertError::Duplicate(_)) => return Err(TickError::Duplicate(id)),
            Err(InsertError::Durability(e)) => {
                self.recovery_view = Some(before);
                return Err(TickError::Storage(e));
            }
        }
        self.index(&task);
        Ok(())
    }

    pub fn replace(&mut self, task: Task) -> Result<(), TickError> {
        self.require_writable()?;
        let id = task.id;
        let old = self.stack.get(id).ok_or(TickError::NotFound(id))?;
        let before = self.snapshot();
        match self.stack.replace(task.clone()) {
            Ok(()) => {}
            Err(ReplaceError::NotFound(_)) => return Err(TickError::NotFound(id)),
            Err(ReplaceError::Durability(e)) => {
                self.recovery_view = Some(before);
                return Err(TickError::Storage(e));
            }
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
    pub(crate) fn replace_batch(&mut self, tasks: Vec<Task>) -> Result<(), TickError> {
        self.require_writable()?;
        let mut ids = BTreeSet::new();
        for task in &tasks {
            if !ids.insert(task.id) {
                return Err(TickError::Duplicate(task.id));
            }
        }
        let before = self.snapshot();
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

        if let Err(error) = self.journal.commit(&batch) {
            self.recovery_view = Some(before);
            return Err(error.into());
        }
        self.stack.defer_sync();
        if let Err(error) = Self::apply_batch(&mut self.stack, &batch, usize::MAX) {
            self.recovery_view = Some(before);
            return Err(error);
        }
        if let Err(error) = self.stack.commit() {
            self.recovery_view = Some(before);
            return Err(error.into());
        }
        // Do not publish derived indexes until both the records and retirement
        // of their redo are known to have completed.
        if let Err(error) = self.journal.checkpoint() {
            self.recovery_view = Some(before);
            return Err(error.into());
        }
        for task in &old {
            self.unindex(task);
        }
        for task in &tasks {
            self.index(task);
        }
        Ok(())
    }

    pub(crate) fn require_writable(&self) -> Result<(), TickError> {
        if self.recovery_view.is_some() {
            Err(TickError::RecoveryRequired)
        } else {
            Ok(())
        }
    }

    fn snapshot(&self) -> BTreeMap<Uuid, Task> {
        self.stack
            .all_ids()
            .into_iter()
            .filter_map(|id| self.stack.get(id).map(|task| (id, task)))
            .collect()
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
        self.require_writable()?;
        let old = self.stack.get(id).ok_or(TickError::NotFound(id))?;
        let before = self.snapshot();
        match self.stack.delete(id) {
            Ok(()) => {}
            Err(DeleteError::NotFound(_)) => return Err(TickError::NotFound(id)),
            Err(DeleteError::Durability(e)) => {
                self.recovery_view = Some(before);
                return Err(TickError::Storage(e));
            }
        }
        self.unindex(&old);
        Ok(())
    }

    pub fn get(&self, id: Uuid) -> Option<Task> {
        if let Some(view) = &self.recovery_view {
            return view.get(&id).cloned();
        }
        self.stack.get(id)
    }

    /// Every task, trashed ones included, unspecified order.
    pub fn all(&self) -> Vec<Task> {
        if let Some(view) = &self.recovery_view {
            return view.values().cloned().collect();
        }
        self.stack
            .all_ids()
            .into_iter()
            .filter_map(|id| self.stack.get(id))
            .collect()
    }

    /// Every task in `list`, in the store's own (unspecified) order.
    pub fn in_list(&self, list: Uuid) -> Vec<Task> {
        if self.recovery_view.is_some() {
            return self
                .all()
                .into_iter()
                .filter(|task| task.list_id == list)
                .collect();
        }
        FilterEq::<Task, ByList>::filter_eq(&self.stack, &list)
            .into_iter()
            .filter_map(|id| self.stack.get(id))
            .collect()
    }

    /// Tasks of `list` due within `[from, to)`, soonest first (a smart list).
    pub fn due_between(&self, list: Uuid, from: i64, to: i64) -> Vec<Task> {
        if self.recovery_view.is_some() {
            let mut tasks: Vec<_> = self
                .all()
                .into_iter()
                .filter(|task| {
                    task.list_id == list && task.due_ms.is_some_and(|due| due >= from && due < to)
                })
                .collect();
            tasks.sort_by_key(|task| (task.due_ms, task.id));
            return tasks;
        }
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
        if self.recovery_view.is_some() {
            let mut tasks = self.in_list(list);
            tasks.sort_by_key(|task| (task.sort_order, task.id));
            return tasks;
        }
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
        self.require_writable()?;
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
        let (changed, _) = trash_batch(&originals, 50);
        let mut duplicate = changed.clone();
        duplicate.push(changed[0].clone());
        let mut store = TaskStore::open(dir.path()).unwrap();
        assert!(matches!(
            store.replace_batch(duplicate),
            Err(TickError::Duplicate(id)) if id == changed[0].id
        ));
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
    fn checkpoint_failure_fences_follow_up_writes_and_preserves_a_consistent_view() {
        let (dir, originals, unrelated) = fixture();
        let (changed, _) = trash_batch(&originals, 50);
        let mut store = TaskStore::open(dir.path()).unwrap();
        std::fs::create_dir(dir.path().join("tasks.journal.tmp")).unwrap();
        assert!(matches!(
            store.replace_batch(changed.clone()),
            Err(TickError::Journal(_))
        ));

        // Reads retain the complete view from before acceptance, never a
        // prefix, while every kind of later mutation is explicitly refused.
        for original in &originals {
            assert_eq!(store.get(original.id).unwrap(), *original);
        }
        assert!(matches!(
            store.replace(unrelated.clone()),
            Err(TickError::RecoveryRequired)
        ));
        assert!(matches!(
            store.delete(unrelated.id),
            Err(TickError::RecoveryRequired)
        ));
        assert!(matches!(
            store.reorder(unrelated.id, 7),
            Err(TickError::RecoveryRequired)
        ));
        assert!(matches!(
            store.replace_batch(changed),
            Err(TickError::RecoveryRequired)
        ));

        drop(store);
        std::fs::remove_dir(dir.path().join("tasks.journal.tmp")).unwrap();
        assert_replayed(dir.path(), &originals, &unrelated, 50);
    }

    #[test]
    fn ordinary_persistence_failure_fences_the_store_without_poisoning_not_found() {
        let (dir, originals, unrelated) = fixture();
        let mut store = TaskStore::open(dir.path()).unwrap();
        assert!(matches!(
            store.replace(task(999, unrelated.list_id, "missing")),
            Err(TickError::NotFound(_))
        ));
        assert!(
            store.get(unrelated.id).is_some(),
            "validation is non-poisoning"
        );

        let log = insert_log::log_path(&dir.path().join("tasks.mmap"));
        std::fs::create_dir(&log).unwrap();
        let mut changed = unrelated.clone();
        changed.title = "must not become visible".into();
        assert!(matches!(store.replace(changed), Err(TickError::Storage(_))));
        assert_eq!(store.get(unrelated.id).unwrap(), unrelated);
        assert!(matches!(
            store.insert(task(5, unrelated.list_id, "refused")),
            Err(TickError::RecoveryRequired)
        ));
        assert!(matches!(
            store.delete(originals[0].id),
            Err(TickError::RecoveryRequired)
        ));

        drop(store);
        std::fs::remove_dir(log).unwrap();
        let reopened = TaskStore::open(dir.path()).unwrap();
        assert_eq!(reopened.get(unrelated.id).unwrap(), unrelated);
        for original in originals {
            assert_eq!(reopened.get(original.id).unwrap(), original);
        }
    }

    #[test]
    fn accepted_redo_recovers_recognized_insert_log_creation_prefixes_only() {
        for prefix_len in [0, 1, 7, 8, 11, 27] {
            let (dir, originals, unrelated) = fixture();
            let (_, batch) = trash_batch(&originals, 50);
            let mut store = TaskStore::open(dir.path()).unwrap();
            store.journal.commit(&batch).unwrap();
            drop(store);

            let mmap = dir.path().join("tasks.mmap");
            let log = insert_log::log_path(&mmap);
            insert_log::append(&log, &unrelated).unwrap();
            let header = std::fs::read(&log).unwrap();
            std::fs::write(&log, &header[..prefix_len]).unwrap();
            assert_replayed(dir.path(), &originals, &unrelated, 50);
        }

        let (dir, originals, _) = fixture();
        let (_, batch) = trash_batch(&originals, 50);
        let mut store = TaskStore::open(dir.path()).unwrap();
        store.journal.commit(&batch).unwrap();
        drop(store);
        std::fs::write(
            insert_log::log_path(&dir.path().join("tasks.mmap")),
            b"not a header",
        )
        .unwrap();
        assert!(TaskStore::open(dir.path()).is_err());
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
