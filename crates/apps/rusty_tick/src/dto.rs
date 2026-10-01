//! The JSON shapes on the wire. Inputs reject unknown fields, so a typo in a
//! field name is an error rather than a silently ignored change.

use crate::docs::Doc;
use crate::lists::{TaskList, ViewMode};
use crate::service::Snapshot;
use crate::tags::Tag;
use crate::task::{ChecklistItem, Status, Task, TaskKind};
use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

// ---- responses ---------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDto {
    pub id: Uuid,
    pub list_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub title: String,
    pub notes: String,
    pub kind: TaskKind,
    pub status: Status,
    pub priority: u8,
    pub start_ms: Option<i64>,
    pub due_ms: Option<i64>,
    pub is_all_day: bool,
    pub time_zone: String,
    pub reminders: Vec<String>,
    pub repeat_flag: String,
    pub ex_dates: Vec<i64>,
    pub items: Vec<ChecklistItem>,
    pub tags: Vec<String>,
    pub sort_order: i64,
    pub created_ms: i64,
    pub updated_ms: i64,
    pub completed_ms: Option<i64>,
    pub deleted_ms: Option<i64>,
    pub etag: String,
}

impl From<Task> for TaskDto {
    fn from(t: Task) -> Self {
        Self {
            id: t.id,
            list_id: t.list_id,
            parent_id: t.parent_id,
            title: t.title,
            notes: t.notes,
            kind: t.kind,
            status: t.status,
            priority: t.priority.as_u8(),
            start_ms: t.start_ms,
            due_ms: t.due_ms,
            is_all_day: t.is_all_day,
            time_zone: t.time_zone,
            reminders: t.reminders,
            repeat_flag: t.repeat_flag,
            ex_dates: t.ex_dates,
            items: t.items,
            tags: t.tags,
            sort_order: t.sort_order,
            created_ms: t.created_ms,
            updated_ms: t.updated_ms,
            completed_ms: t.completed_ms,
            deleted_ms: t.deleted_ms,
            etag: t.version.to_string(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListDto {
    pub id: Uuid,
    pub name: String,
    pub color: Option<String>,
    pub archived: bool,
    pub view_mode: ViewMode,
    pub sort_type: String,
    pub sort_order: i64,
    pub updated_ms: i64,
    pub etag: String,
}

impl From<TaskList> for ListDto {
    fn from(l: TaskList) -> Self {
        Self {
            id: l.id,
            name: l.name,
            color: l.color,
            archived: l.archived,
            view_mode: l.view_mode,
            sort_type: l.sort_type,
            sort_order: l.sort_order,
            updated_ms: l.updated_ms,
            etag: l.version.to_string(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagDto {
    pub name: String,
    pub label: String,
    pub color: Option<String>,
    pub parent: Option<String>,
    pub sort_order: i64,
    pub etag: String,
}

impl From<Tag> for TagDto {
    fn from(t: Tag) -> Self {
        Self {
            name: t.name,
            label: t.label,
            color: t.color,
            parent: t.parent,
            sort_order: t.sort_order,
            etag: t.version.to_string(),
        }
    }
}

#[derive(Serialize)]
pub struct Tasks {
    pub tasks: Vec<TaskDto>,
}

impl Tasks {
    pub fn new(tasks: Vec<Task>) -> Self {
        Self {
            tasks: tasks.into_iter().map(TaskDto::from).collect(),
        }
    }
}

#[derive(Serialize)]
pub struct Lists {
    pub lists: Vec<ListDto>,
}

#[derive(Serialize)]
pub struct Tags {
    pub tags: Vec<TagDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotDto {
    pub inbox_id: Uuid,
    pub server_time_ms: i64,
    pub lists: Vec<ListDto>,
    pub tasks: Vec<TaskDto>,
    pub tags: Vec<TagDto>,
}

impl From<Snapshot> for SnapshotDto {
    fn from(s: Snapshot) -> Self {
        Self {
            inbox_id: s.inbox_id,
            server_time_ms: s.server_time_ms,
            lists: s.lists.into_iter().map(ListDto::from).collect(),
            tasks: s.tasks.into_iter().map(TaskDto::from).collect(),
            tags: s.tags.into_iter().map(TagDto::from).collect(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocDto {
    pub id: Uuid,
    pub kind: String,
    pub body: rusty_json::Value,
    pub updated_ms: i64,
}

impl DocDto {
    /// `None` if the stored body is no longer valid JSON (it was validated on write).
    pub fn from_doc(doc: Doc) -> Option<Self> {
        Some(Self {
            id: doc.id,
            kind: doc.kind,
            body: rusty_json::from_str(&doc.body).ok()?,
            updated_ms: doc.updated_ms,
        })
    }
}

#[derive(Serialize)]
pub struct Docs {
    pub docs: Vec<DocDto>,
}

#[derive(Serialize)]
pub struct Purged {
    pub purged: usize,
}

// ---- requests ----------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateTask {
    pub id: Option<Uuid>,
    pub list_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub title: String,
    #[serde(default)]
    pub notes: String,
    pub kind: Option<TaskKind>,
    pub priority: Option<u8>,
    pub start_ms: Option<i64>,
    pub due_ms: Option<i64>,
    #[serde(default)]
    pub is_all_day: bool,
    #[serde(default)]
    pub time_zone: String,
    #[serde(default)]
    pub reminders: Vec<String>,
    #[serde(default)]
    pub repeat_flag: String,
    #[serde(default)]
    pub items: Vec<ChecklistItem>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub sort_order: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PatchTask {
    pub title: Option<String>,
    pub notes: Option<String>,
    pub kind: Option<TaskKind>,
    pub status: Option<Status>,
    pub priority: Option<u8>,
    /// Absent leaves a date alone; `null` clears it.
    #[serde(default, deserialize_with = "present")]
    pub start_ms: Option<Option<i64>>,
    #[serde(default, deserialize_with = "present")]
    pub due_ms: Option<Option<i64>>,
    pub is_all_day: Option<bool>,
    pub time_zone: Option<String>,
    pub reminders: Option<Vec<String>>,
    pub repeat_flag: Option<String>,
    pub ex_dates: Option<Vec<i64>>,
    pub items: Option<Vec<ChecklistItem>>,
    pub tags: Option<Vec<String>>,
    pub list_id: Option<Uuid>,
    pub sort_order: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateList {
    pub id: Option<Uuid>,
    pub name: String,
    pub color: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PatchList {
    pub name: Option<String>,
    #[serde(default, deserialize_with = "present")]
    pub color: Option<Option<String>>,
    pub archived: Option<bool>,
    pub view_mode: Option<ViewMode>,
    pub sort_type: Option<String>,
    pub sort_order: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateTag {
    pub label: String,
    pub color: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PatchTag {
    #[serde(default, deserialize_with = "present")]
    pub color: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    pub parent: Option<Option<String>>,
    pub sort_order: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenameTag {
    pub label: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetOrder {
    pub sort_order: i64,
}

/// A field that is present (even as `null`) becomes `Some(..)`.
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}
