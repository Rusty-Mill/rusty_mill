//! The JSON shapes on the wire. Inputs reject unknown fields, so a typo in a
//! field name is an error rather than a silently ignored change.

use crate::lists::TaskList;
use crate::task::{Status, Task};
use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDto {
    pub id: Uuid,
    pub list_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub title: String,
    pub notes: String,
    pub status: Status,
    pub priority: u8,
    pub due_ms: Option<i64>,
    pub sort_order: i64,
    pub tags: Vec<String>,
    pub updated_ms: i64,
}

impl From<Task> for TaskDto {
    fn from(t: Task) -> Self {
        Self {
            id: t.id,
            list_id: t.list_id,
            parent_id: t.parent_id,
            title: t.title,
            notes: t.notes,
            status: t.status,
            priority: t.priority.as_u8(),
            due_ms: t.due_ms,
            sort_order: t.sort_order,
            tags: t.tags,
            updated_ms: t.updated_ms,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListDto {
    pub id: Uuid,
    pub name: String,
    pub archived: bool,
    pub sort_order: i64,
    pub updated_ms: i64,
}

impl From<TaskList> for ListDto {
    fn from(l: TaskList) -> Self {
        Self {
            id: l.id,
            name: l.name,
            archived: l.archived,
            sort_order: l.sort_order,
            updated_ms: l.updated_ms,
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateTask {
    pub list_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub title: String,
    #[serde(default)]
    pub notes: String,
    pub priority: Option<u8>,
    pub due_ms: Option<i64>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PatchTask {
    pub title: Option<String>,
    pub notes: Option<String>,
    pub priority: Option<u8>,
    /// Absent leaves the date alone; `null` clears it.
    #[serde(default, deserialize_with = "present")]
    pub due_ms: Option<Option<i64>>,
    pub tags: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateList {
    pub name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchList {
    pub name: Option<String>,
    pub archived: Option<bool>,
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
