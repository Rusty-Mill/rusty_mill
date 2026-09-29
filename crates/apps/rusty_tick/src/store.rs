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
}

pub struct TaskStore {
    stack: Stack,
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
            search: FullTextIndex::new(),
            tags: BTreeMap::new(),
        };
        for id in store.stack.all_ids() {
            if let Some(task) = store.stack.get(id) {
                store.index(&task);
            }
        }
        Ok(store)
    }

    fn index(&mut self, task: &Task) {
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

    /// Ids matching any of `phrases` in title or notes, best match first.
    pub fn search(&self, phrases: &[&str]) -> Vec<Uuid> {
        let mut hits = self.search.search(&Query::any_of(phrases.iter().copied()));
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
