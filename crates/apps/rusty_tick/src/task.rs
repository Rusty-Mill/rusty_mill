//! The `Task` record and the engine markers that index it.

use rusty_multimodal_db_engine::generic::traits::{
    IndexedField, OrderedField, Record, ScannableField, SchemaTag,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Sentinel `due_ms` key for a task with no due date: sorts after every real date.
pub const NO_DUE: i64 = i64::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Status {
    Open,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Priority {
    None = 0,
    Low = 1,
    Medium = 3,
    High = 5,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: Uuid,
    pub list_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub title: String,
    pub notes: String,
    pub status: Status,
    pub priority: Priority,
    pub due_ms: Option<i64>,
    pub sort_order: i64,
    pub tags: Vec<String>,
    pub updated_ms: i64,
}

impl Record for Task {
    type Id = Uuid;
    fn id(&self) -> Uuid {
        self.id
    }
}

impl SchemaTag for Task {
    const SCHEMA_TAG: &'static str = "rusty_tick::Task@1";
}

/// Index marker: tasks by list.
pub struct ByList;
impl IndexedField<ByList> for Task {
    type IndexValue = Uuid;
    fn indexed_value(&self) -> &Uuid {
        &self.list_id
    }
}

/// Marker for the manual sort order: durable and updatable in place
/// (`ScannableField`) and range-walkable (`OrderedField`) under one marker,
/// as the engine requires of a field that is both.
///
/// The ordered key is `(list_id, sort_order)`, so a walk is per list.
pub struct SortOrder;
impl ScannableField<SortOrder> for Task {
    type ScanValue = i64;
    fn scannable_value(&self) -> i64 {
        self.sort_order
    }
    fn set_scannable_value(&mut self, value: i64) {
        self.sort_order = value;
    }
}
impl OrderedField<SortOrder> for Task {
    type Key = (Uuid, i64);
    fn order_key(&self) -> (Uuid, i64) {
        (self.list_id, self.sort_order)
    }
}

/// Marker for the due date, ordered as `(list_id, due)`; tasks with no
/// due date carry [`NO_DUE`].
pub struct DueAt;
impl OrderedField<DueAt> for Task {
    type Key = (Uuid, i64);
    fn order_key(&self) -> (Uuid, i64) {
        (self.list_id, self.due_ms.unwrap_or(NO_DUE))
    }
}
