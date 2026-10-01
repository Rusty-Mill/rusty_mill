//! The `Task` record and the engine markers that index it.

use rusty_multimodal_db_engine::generic::traits::{
    IndexedField, OrderedField, Record, ScannableField, SchemaTag,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Sentinel `due_ms` key for a task with no due date: sorts after every real date.
pub const NO_DUE: i64 = i64::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
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

impl Priority {
    /// The wire value (TickTick's scale: 0, 1, 3, 5).
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    /// `None` for a value outside the scale.
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::None),
            1 => Some(Self::Low),
            3 => Some(Self::Medium),
            5 => Some(Self::High),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskKind {
    Text,
    Checklist,
    Note,
}

/// One line of a task's checklist. Also its wire shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChecklistItem {
    pub id: Uuid,
    pub title: String,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub sort_order: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: Uuid,
    pub list_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub title: String,
    pub notes: String,
    pub kind: TaskKind,
    pub status: Status,
    pub priority: Priority,
    pub start_ms: Option<i64>,
    pub due_ms: Option<i64>,
    pub is_all_day: bool,
    /// IANA zone the dates were entered in (`""` when floating).
    pub time_zone: String,
    /// Reminder triggers, e.g. `TRIGGER:PT0S` (at the due time).
    pub reminders: Vec<String>,
    /// An RFC 5545 `RRULE`, or empty for a one-off task.
    pub repeat_flag: String,
    /// Occurrences (Unix ms) skipped from the repeat rule.
    pub ex_dates: Vec<i64>,
    pub items: Vec<ChecklistItem>,
    pub tags: Vec<String>,
    pub sort_order: i64,
    pub created_ms: i64,
    pub updated_ms: i64,
    pub completed_ms: Option<i64>,
    /// Set while the task is in the trash.
    pub deleted_ms: Option<i64>,
    /// Bumped on every write; the API's ETag.
    pub version: u32,
}

impl Task {
    /// An open, undated task with every optional field empty.
    pub fn new(id: Uuid, list_id: Uuid, title: &str, sort_order: i64, now_ms: i64) -> Self {
        Self {
            id,
            list_id,
            parent_id: None,
            title: title.to_string(),
            notes: String::new(),
            kind: TaskKind::Text,
            status: Status::Open,
            priority: Priority::None,
            start_ms: None,
            due_ms: None,
            is_all_day: false,
            time_zone: String::new(),
            reminders: Vec::new(),
            repeat_flag: String::new(),
            ex_dates: Vec::new(),
            items: Vec::new(),
            tags: Vec::new(),
            sort_order,
            created_ms: now_ms,
            updated_ms: now_ms,
            completed_ms: None,
            deleted_ms: None,
            version: 1,
        }
    }

    pub fn is_deleted(&self) -> bool {
        self.deleted_ms.is_some()
    }
}

impl Record for Task {
    type Id = Uuid;
    fn id(&self) -> Uuid {
        self.id
    }
}

impl SchemaTag for Task {
    const SCHEMA_TAG: &'static str = "rusty_tick::Task@2";
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
