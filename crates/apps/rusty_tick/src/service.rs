//! The application rules: validation, cascading deletes, smart lists.
//!
//! No I/O beyond the two stores, and no HTTP: the router in [`crate::api`]
//! translates requests into calls on [`Service`].

use crate::lists::{ListStore, TaskList};
use crate::store::{TaskStore, TickError, SORT_STEP};
use crate::task::{Priority, Status, Task, NO_DUE};
use uuid::Uuid;

pub const MAX_TITLE_CHARS: usize = 500;
pub const MAX_NOTES_CHARS: usize = 100_000;
pub const MAX_LIST_NAME_CHARS: usize = 200;
pub const MAX_TAGS: usize = 20;
pub const MAX_TAG_CHARS: usize = 64;
const DAY_MS: i64 = 86_400_000;

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("{0} not found")]
    NotFound(&'static str),
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Storage(#[from] TickError),
}

pub type Result<T> = std::result::Result<T, ServiceError>;

fn invalid<T>(message: impl Into<String>) -> Result<T> {
    Err(ServiceError::Invalid(message.into()))
}

/// Milliseconds since the Unix epoch.
pub type Clock = Box<dyn Fn() -> i64 + Send>;

pub fn system_clock() -> Clock {
    Box::new(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
    })
}

#[derive(Debug, Clone, Default)]
pub struct NewTask {
    pub list_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub title: String,
    pub notes: String,
    pub priority: Option<Priority>,
    pub due_ms: Option<i64>,
    pub tags: Vec<String>,
}

/// `Option<Option<_>>` fields: outer `None` leaves the field alone,
/// `Some(None)` clears it.
#[derive(Debug, Clone, Default)]
pub struct TaskPatch {
    pub title: Option<String>,
    pub notes: Option<String>,
    pub priority: Option<Priority>,
    pub due_ms: Option<Option<i64>>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListOrderBy {
    Manual,
    Due,
}

pub struct Service {
    tasks: TaskStore,
    lists: ListStore,
    clock: Clock,
}

impl Service {
    pub fn open(dir: &std::path::Path, clock: Clock) -> Result<Self> {
        Ok(Self {
            tasks: TaskStore::open(dir)?,
            lists: ListStore::open(dir)?,
            clock,
        })
    }

    fn now(&self) -> i64 {
        (self.clock)()
    }

    // ---- lists -------------------------------------------------------

    pub fn create_list(&mut self, name: &str) -> Result<TaskList> {
        let name = clean_name(name)?;
        let sort_order = self
            .lists
            .all()
            .last()
            .map_or(0, |l| l.sort_order.saturating_add(SORT_STEP));
        let list = TaskList {
            id: Uuid::now_v7(),
            name,
            archived: false,
            sort_order,
            updated_ms: self.now(),
        };
        self.lists.insert(list.clone())?;
        Ok(list)
    }

    pub fn lists(&self) -> Vec<TaskList> {
        self.lists.all()
    }

    pub fn list(&self, id: Uuid) -> Result<TaskList> {
        self.lists.get(id).ok_or(ServiceError::NotFound("list"))
    }

    pub fn patch_list(
        &mut self,
        id: Uuid,
        name: Option<&str>,
        archived: Option<bool>,
    ) -> Result<TaskList> {
        let mut list = self.list(id)?;
        if let Some(name) = name {
            list.name = clean_name(name)?;
        }
        if let Some(archived) = archived {
            list.archived = archived;
        }
        list.updated_ms = self.now();
        self.lists.replace(list.clone())?;
        Ok(list)
    }

    /// Delete a list and every task in it.
    pub fn delete_list(&mut self, id: Uuid) -> Result<()> {
        self.list(id)?;
        for task in self.tasks.in_list(id) {
            self.tasks.delete(task.id)?;
        }
        self.lists.delete(id)?;
        Ok(())
    }

    // ---- tasks -------------------------------------------------------

    pub fn create_task(&mut self, new: NewTask) -> Result<Task> {
        self.list(new.list_id)?;
        if let Some(parent) = new.parent_id {
            let parent = self.task(parent)?;
            if parent.list_id != new.list_id {
                return invalid("a subtask must be in its parent's list");
            }
            if parent.parent_id.is_some() {
                return invalid("subtasks nest one level deep");
            }
        }
        clean_due(new.due_ms)?;
        let task = Task {
            id: Uuid::now_v7(),
            list_id: new.list_id,
            parent_id: new.parent_id,
            title: clean_title(&new.title)?,
            notes: clean_notes(new.notes)?,
            status: Status::Open,
            priority: new.priority.unwrap_or(Priority::None),
            due_ms: new.due_ms,
            sort_order: self.tasks.next_sort_order(new.list_id),
            tags: clean_tags(new.tags)?,
            updated_ms: self.now(),
        };
        self.tasks.insert(task.clone())?;
        Ok(task)
    }

    pub fn task(&self, id: Uuid) -> Result<Task> {
        self.tasks.get(id).ok_or(ServiceError::NotFound("task"))
    }

    pub fn patch_task(&mut self, id: Uuid, patch: TaskPatch) -> Result<Task> {
        let mut task = self.task(id)?;
        if let Some(title) = patch.title {
            task.title = clean_title(&title)?;
        }
        if let Some(notes) = patch.notes {
            task.notes = clean_notes(notes)?;
        }
        if let Some(priority) = patch.priority {
            task.priority = priority;
        }
        if let Some(due) = patch.due_ms {
            clean_due(due)?;
            task.due_ms = due;
        }
        if let Some(tags) = patch.tags {
            task.tags = clean_tags(tags)?;
        }
        task.updated_ms = self.now();
        self.tasks.replace(task.clone())?;
        Ok(task)
    }

    pub fn set_status(&mut self, id: Uuid, status: Status) -> Result<Task> {
        let mut task = self.task(id)?;
        task.status = status;
        task.updated_ms = self.now();
        self.tasks.replace(task.clone())?;
        Ok(task)
    }

    /// Delete a task and its subtasks.
    pub fn delete_task(&mut self, id: Uuid) -> Result<()> {
        let task = self.task(id)?;
        for child in self
            .tasks
            .in_list(task.list_id)
            .into_iter()
            .filter(|t| t.parent_id == Some(id))
        {
            self.tasks.delete(child.id)?;
        }
        self.tasks.delete(id)?;
        Ok(())
    }

    /// Set a task's manual position. Rewrites one durable slot.
    pub fn reorder(&mut self, id: Uuid, sort_order: i64) -> Result<Task> {
        self.task(id)?;
        self.tasks.reorder(id, sort_order)?;
        self.task(id)
    }

    pub fn tasks_in_list(
        &self,
        list: Uuid,
        status: Option<Status>,
        order: ListOrderBy,
    ) -> Result<Vec<Task>> {
        self.list(list)?;
        let mut tasks = match order {
            ListOrderBy::Manual => self.tasks.in_manual_order(list),
            ListOrderBy::Due => {
                let mut all = self.tasks.in_list(list);
                all.sort_by_key(|t| (t.due_ms.unwrap_or(i64::MAX), t.sort_order, t.id));
                all
            }
        };
        if let Some(status) = status {
            tasks.retain(|t| t.status == status);
        }
        Ok(tasks)
    }

    // ---- search and smart lists --------------------------------------

    /// Tasks matching any of the whitespace-separated `terms` as whole
    /// words in title or notes (the engine has no prefix matching yet).
    pub fn search(&self, query: &str) -> Vec<Task> {
        let terms: Vec<&str> = query.split_whitespace().collect();
        if terms.is_empty() {
            return Vec::new();
        }
        self.tasks
            .search(&terms)
            .into_iter()
            .filter_map(|id| self.tasks.get(id))
            .collect()
    }

    pub fn with_tag(&self, tag: &str) -> Vec<Task> {
        self.tasks
            .with_tag(tag)
            .into_iter()
            .filter_map(|id| self.tasks.get(id))
            .collect()
    }

    /// Open tasks due in `[from, to)` across every non-archived list,
    /// soonest first. One range query per list.
    fn due_window(&self, from: i64, to: i64) -> Vec<Task> {
        let mut tasks: Vec<Task> = self
            .lists
            .all()
            .into_iter()
            .filter(|l| !l.archived)
            .flat_map(|l| self.tasks.due_between(l.id, from, to))
            .filter(|t| t.status == Status::Open)
            .collect();
        tasks.sort_by_key(|t| (t.due_ms, t.id));
        tasks
    }

    /// Open tasks due before `now`.
    pub fn overdue(&self) -> Vec<Task> {
        self.due_window(i64::MIN, self.now())
    }

    /// Open tasks due today (the caller's day, `utc_offset_min` from UTC)
    /// plus everything overdue, as TickTick's Today does.
    pub fn today(&self, utc_offset_min: i32) -> Vec<Task> {
        let (_, end) = day_bounds(self.now(), utc_offset_min);
        self.due_window(i64::MIN, end)
    }

    /// Open tasks due from the start of today through the next 7 days.
    pub fn next_7_days(&self, utc_offset_min: i32) -> Vec<Task> {
        let (start, _) = day_bounds(self.now(), utc_offset_min);
        self.due_window(start, start + 7 * DAY_MS)
    }
}

/// `[start, end)` of the local day containing `now_ms`, in UTC milliseconds.
fn day_bounds(now_ms: i64, utc_offset_min: i32) -> (i64, i64) {
    let offset = i64::from(utc_offset_min) * 60_000;
    let local = now_ms.saturating_add(offset);
    let start = local - local.rem_euclid(DAY_MS) - offset;
    (start, start + DAY_MS)
}

/// `i64::MAX` is the store's "no due date" key, so it cannot be a real date.
fn clean_due(due: Option<i64>) -> Result<()> {
    if due == Some(NO_DUE) {
        return invalid("dueMs is out of range");
    }
    Ok(())
}

fn clean_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return invalid("name must not be empty");
    }
    if name.chars().count() > MAX_LIST_NAME_CHARS {
        return invalid(format!(
            "name is longer than {MAX_LIST_NAME_CHARS} characters"
        ));
    }
    Ok(name.to_string())
}

fn clean_title(title: &str) -> Result<String> {
    let title = title.trim();
    if title.is_empty() {
        return invalid("title must not be empty");
    }
    if title.chars().count() > MAX_TITLE_CHARS {
        return invalid(format!("title is longer than {MAX_TITLE_CHARS} characters"));
    }
    Ok(title.to_string())
}

fn clean_notes(notes: String) -> Result<String> {
    if notes.chars().count() > MAX_NOTES_CHARS {
        return invalid(format!(
            "notes are longer than {MAX_NOTES_CHARS} characters"
        ));
    }
    Ok(notes)
}

/// Trim, drop empties, de-duplicate (first occurrence wins), and bound.
fn clean_tags(tags: Vec<String>) -> Result<Vec<String>> {
    let mut clean: Vec<String> = Vec::new();
    for tag in tags {
        let tag = tag.trim().to_string();
        if tag.is_empty() || clean.contains(&tag) {
            continue;
        }
        if tag.chars().count() > MAX_TAG_CHARS {
            return invalid(format!("a tag is longer than {MAX_TAG_CHARS} characters"));
        }
        clean.push(tag);
    }
    if clean.len() > MAX_TAGS {
        return invalid(format!("more than {MAX_TAGS} tags"));
    }
    Ok(clean)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_bounds_follow_the_callers_offset() {
        // 2026-09-29T23:30:00Z is already the 30th at UTC+2.
        let now = 1_790_724_600_000;
        let (utc_start, utc_end) = day_bounds(now, 0);
        assert!(utc_start <= now && now < utc_end);
        let (plus2_start, _) = day_bounds(now, 120);
        assert_eq!(plus2_start, utc_end - 2 * 3_600_000);
    }

    #[test]
    fn tags_are_trimmed_deduplicated_and_bounded() {
        let tags = clean_tags(vec![" a ".into(), "a".into(), "".into(), "b".into()]).unwrap();
        assert_eq!(tags, vec!["a", "b"]);
        assert!(clean_tags((0..21).map(|i| i.to_string()).collect()).is_err());
        assert!(clean_tags(vec!["x".repeat(65)]).is_err());
    }
}
