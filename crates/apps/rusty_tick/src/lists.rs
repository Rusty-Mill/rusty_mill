//! Task lists (TickTick's "projects"), stored on the engine like tasks.

use crate::store::TickError;
use rusty_multimodal_db_engine::generic::query::{AllIds, GetById};
use rusty_multimodal_db_engine::generic::traits::{
    IndexedField, Record, ScannableField, SchemaTag,
};
use rusty_multimodal_db_engine::generic::{
    DeleteError, GenericMmapStore, InsertError, ReplaceError,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskList {
    pub id: Uuid,
    pub name: String,
    pub archived: bool,
    pub sort_order: i64,
    pub updated_ms: i64,
}

impl Record for TaskList {
    type Id = Uuid;
    fn id(&self) -> Uuid {
        self.id
    }
}

impl SchemaTag for TaskList {
    const SCHEMA_TAG: &'static str = "rusty_tick::TaskList@1";
}

/// Index marker: lists by archived flag.
pub struct ByArchived;
impl IndexedField<ByArchived> for TaskList {
    type IndexValue = bool;
    fn indexed_value(&self) -> &bool {
        &self.archived
    }
}

/// Slot marker: the manual sort order, durable and updatable in place.
pub struct ListOrder;
impl ScannableField<ListOrder> for TaskList {
    type ScanValue = i64;
    fn scannable_value(&self) -> i64 {
        self.sort_order
    }
    fn set_scannable_value(&mut self, value: i64) {
        self.sort_order = value;
    }
}

type Core = GenericMmapStore<TaskList, ByArchived, ListOrder>;

pub struct ListStore {
    core: Core,
}

impl ListStore {
    pub fn open(dir: &Path) -> Result<Self, TickError> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join("lists.mmap");
        let core = if path.exists() {
            GenericMmapStore::open_portable(&path)?
        } else {
            GenericMmapStore::create(Vec::new(), &path)?
        };
        Ok(Self { core })
    }

    pub fn insert(&mut self, list: TaskList) -> Result<(), TickError> {
        let id = list.id;
        match self.core.insert(list) {
            Ok(()) => Ok(()),
            Err(InsertError::Duplicate(_)) => Err(TickError::Duplicate(id)),
            Err(InsertError::Durability(e)) => Err(TickError::Storage(e)),
        }
    }

    pub fn replace(&mut self, list: TaskList) -> Result<(), TickError> {
        let id = list.id;
        match self.core.replace(list) {
            Ok(()) => Ok(()),
            Err(ReplaceError::NotFound(_)) => Err(TickError::NotFound(id)),
            Err(ReplaceError::Durability(e)) => Err(TickError::Storage(e)),
        }
    }

    pub fn delete(&mut self, id: Uuid) -> Result<(), TickError> {
        match self.core.delete(id) {
            Ok(()) => Ok(()),
            Err(DeleteError::NotFound(_)) => Err(TickError::NotFound(id)),
            Err(DeleteError::Durability(e)) => Err(TickError::Storage(e)),
        }
    }

    pub fn get(&self, id: Uuid) -> Option<TaskList> {
        self.core.get(id)
    }

    /// Every list, in manual order (ties by id).
    pub fn all(&self) -> Vec<TaskList> {
        let mut lists: Vec<TaskList> = self
            .core
            .all_ids()
            .into_iter()
            .filter_map(|id| self.core.get(id))
            .collect();
        lists.sort_by_key(|l| (l.sort_order, l.id));
        lists
    }
}
