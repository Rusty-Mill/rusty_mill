//! Task lists (TickTick's "projects"), stored on the engine like tasks.

use crate::store::TickError;
use crate::table::Table;
use rusty_multimodal_db_engine::generic::traits::{
    IndexedField, Record, ScannableField, SchemaTag,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

/// The Inbox: every store has one, it cannot be deleted or archived, and
/// tasks whose list is gone are restored into it.
pub const INBOX_ID: Uuid = Uuid::from_u128(0x0000_0000_0000_7000_8000_0000_0000_0001);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewMode {
    List,
    Kanban,
    Timeline,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskList {
    pub id: Uuid,
    pub name: String,
    /// `#rrggbb`, or `None` for the default.
    pub color: Option<String>,
    pub archived: bool,
    pub view_mode: ViewMode,
    /// The client's sort/group choice for this list, opaque to the server.
    pub sort_type: String,
    pub sort_order: i64,
    pub updated_ms: i64,
    pub version: u32,
}

impl TaskList {
    pub fn new(id: Uuid, name: String, sort_order: i64, now_ms: i64) -> Self {
        Self {
            id,
            name,
            color: None,
            archived: false,
            view_mode: ViewMode::List,
            sort_type: String::new(),
            sort_order,
            updated_ms: now_ms,
            version: 1,
        }
    }
}

impl Record for TaskList {
    type Id = Uuid;
    fn id(&self) -> Uuid {
        self.id
    }
}

impl SchemaTag for TaskList {
    const SCHEMA_TAG: &'static str = "rusty_tick::TaskList@2";
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

pub struct ListStore {
    table: Table<TaskList, ByArchived, ListOrder>,
}

impl ListStore {
    pub fn open(dir: &Path) -> Result<Self, TickError> {
        Ok(Self {
            table: Table::open(dir, "lists.mmap")?,
        })
    }

    pub fn insert(&mut self, list: TaskList) -> Result<(), TickError> {
        self.table.insert(list)
    }

    pub fn replace(&mut self, list: TaskList) -> Result<(), TickError> {
        self.table.replace(list)
    }

    pub fn delete(&mut self, id: Uuid) -> Result<(), TickError> {
        self.table.delete(id)
    }

    pub fn get(&self, id: Uuid) -> Option<TaskList> {
        self.table.get(id)
    }

    /// Every list, in manual order (ties by id).
    pub fn all(&self) -> Vec<TaskList> {
        let mut lists = self.table.all();
        lists.sort_by_key(|l| (l.sort_order, l.id));
        lists
    }
}
