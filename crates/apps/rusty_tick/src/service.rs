//! The application rules: validation, the Inbox, trash and restore, tag
//! entities, smart lists.
//!
//! No I/O beyond the stores, and no HTTP: the router in [`crate::api`]
//! translates requests into calls on [`Service`].

use crate::docs::{Doc, DocStore, KINDS, MAX_DOC_BYTES};
use crate::lists::{ListStore, TaskList, ViewMode, INBOX_ID};
use crate::store::{TaskStore, TickError, SORT_STEP};
use crate::tags::{tag_name, Tag, TagStore};
use crate::task::{ChecklistItem, Priority, Status, Task, TaskKind, NO_DUE};
use std::collections::HashSet;
use uuid::Uuid;

pub const MAX_TITLE_CHARS: usize = 500;
pub const MAX_NOTES_CHARS: usize = 100_000;
pub const MAX_LIST_NAME_CHARS: usize = 200;
pub const MAX_TAGS: usize = 20;
pub const MAX_TAG_CHARS: usize = 64;
pub const MAX_ITEMS: usize = 200;
pub const MAX_REMINDERS: usize = 10;
pub const MAX_EX_DATES: usize = 1000;
const MAX_SHORT_TEXT: usize = 200;
const DAY_MS: i64 = 86_400_000;

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("{0} not found")]
    NotFound(&'static str),
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Conflict(String),
    #[error(transparent)]
    Storage(TickError),
}

impl From<TickError> for ServiceError {
    fn from(error: TickError) -> Self {
        match error {
            TickError::Duplicate(id) => Self::Conflict(format!("{id} already exists")),
            TickError::NotFound(_) => Self::NotFound("record"),
            other => Self::Storage(other),
        }
    }
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

/// A task to create. `id` lets a client that generates its own ids (for
/// optimistic updates and an offline queue) make the create idempotent.
#[derive(Debug, Clone, Default)]
pub struct NewTask {
    pub id: Option<Uuid>,
    pub list_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub title: String,
    pub notes: String,
    pub kind: Option<TaskKind>,
    pub priority: Option<Priority>,
    pub start_ms: Option<i64>,
    pub due_ms: Option<i64>,
    pub is_all_day: bool,
    pub time_zone: String,
    pub reminders: Vec<String>,
    pub repeat_flag: String,
    pub items: Vec<ChecklistItem>,
    pub tags: Vec<String>,
    /// Where to put it; `None` appends to the end of the list.
    pub sort_order: Option<i64>,
}

/// `Option<Option<_>>` fields: outer `None` leaves the field alone,
/// `Some(None)` clears it.
#[derive(Debug, Clone, Default)]
pub struct TaskPatch {
    pub title: Option<String>,
    pub notes: Option<String>,
    pub kind: Option<TaskKind>,
    pub status: Option<Status>,
    pub priority: Option<Priority>,
    pub start_ms: Option<Option<i64>>,
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

#[derive(Debug, Clone, Default)]
pub struct NewList {
    pub id: Option<Uuid>,
    pub name: String,
    pub color: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ListPatch {
    pub name: Option<String>,
    pub color: Option<Option<String>>,
    pub archived: Option<bool>,
    pub view_mode: Option<ViewMode>,
    pub sort_type: Option<String>,
    pub sort_order: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct TagPatch {
    pub color: Option<Option<String>>,
    pub parent: Option<Option<String>>,
    pub sort_order: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListOrderBy {
    Manual,
    Due,
}

/// Everything a client needs to boot, in one read.
pub struct Snapshot {
    pub inbox_id: Uuid,
    pub server_time_ms: i64,
    pub lists: Vec<TaskList>,
    pub tasks: Vec<Task>,
    pub tags: Vec<Tag>,
}

pub struct Service {
    tasks: TaskStore,
    lists: ListStore,
    tags: TagStore,
    docs: DocStore,
    clock: Clock,
}

impl Service {
    pub fn open(dir: &std::path::Path, clock: Clock) -> Result<Self> {
        let mut service = Self {
            tasks: TaskStore::open(dir)?,
            lists: ListStore::open(dir)?,
            tags: TagStore::open(dir)?,
            docs: DocStore::open(dir)?,
            clock,
        };
        service.ensure_inbox()?;
        service.realign_subtasks()?;
        service.sweep_orphan_comments()?;
        Ok(service)
    }

    /// A subtask always lives in its parent's list (`patch_task` refuses to
    /// move one alone). A move saves the children, then the parent, one
    /// record at a time, so a crash between them leaves some children in the
    /// new list and the parent in the old. Put every such child back in its
    /// parent's list: the parent is the record of truth, and an interrupted
    /// move was never acknowledged (design review 2.9).
    fn realign_subtasks(&mut self) -> Result<()> {
        let stray: Vec<Task> = self
            .tasks
            .all()
            .into_iter()
            .filter(|child| {
                child
                    .parent_id
                    .and_then(|p| self.tasks.get(p))
                    .is_some_and(|parent| parent.list_id != child.list_id)
            })
            .collect();
        for mut child in stray {
            if let Some(parent) = child.parent_id.and_then(|p| self.tasks.get(p)) {
                child.list_id = parent.list_id;
                self.save_task(child)?;
            }
        }
        Ok(())
    }

    /// Drop comments whose task is gone for good (a trashed task still counts:
    /// it can come back). Purging cleans up after itself; this catches comments
    /// left by versions that did not, and is a no-op otherwise.
    fn sweep_orphan_comments(&mut self) -> Result<usize> {
        let live: std::collections::HashSet<Uuid> =
            self.tasks.all().into_iter().map(|t| t.id).collect();
        Ok(self
            .docs
            .delete_comments_where(|owner| !owner.is_some_and(|id| live.contains(&id)))?)
    }

    fn now(&self) -> i64 {
        (self.clock)()
    }

    fn ensure_inbox(&mut self) -> Result<()> {
        if self.lists.get(INBOX_ID).is_none() {
            let inbox = TaskList::new(INBOX_ID, "Inbox".into(), i64::MIN / 2, self.now());
            self.lists.insert(inbox)?;
        }
        Ok(())
    }

    // ---- lists -------------------------------------------------------

    pub fn create_list(&mut self, new: NewList) -> Result<TaskList> {
        let sort_order = self
            .lists
            .all()
            .last()
            .map_or(0, |l| l.sort_order.saturating_add(SORT_STEP));
        let mut list = TaskList::new(
            new.id.unwrap_or_else(Uuid::now_v7),
            clean_name(&new.name)?,
            sort_order,
            self.now(),
        );
        list.color = clean_color(new.color)?;
        self.lists.insert(list.clone())?;
        Ok(list)
    }

    pub fn lists(&self) -> Vec<TaskList> {
        self.lists.all()
    }

    pub fn list(&self, id: Uuid) -> Result<TaskList> {
        self.lists.get(id).ok_or(ServiceError::NotFound("list"))
    }

    pub fn patch_list(&mut self, id: Uuid, patch: ListPatch) -> Result<TaskList> {
        let mut list = self.list(id)?;
        if id == INBOX_ID && patch.archived == Some(true) {
            return invalid("the Inbox cannot be archived");
        }
        if let Some(name) = patch.name {
            list.name = clean_name(&name)?;
        }
        if let Some(color) = patch.color {
            list.color = clean_color(color)?;
        }
        if let Some(archived) = patch.archived {
            list.archived = archived;
        }
        if let Some(view_mode) = patch.view_mode {
            list.view_mode = view_mode;
        }
        if let Some(sort_type) = patch.sort_type {
            list.sort_type = clean_short(sort_type, "sortType")?;
        }
        if let Some(sort_order) = patch.sort_order {
            list.sort_order = sort_order;
        }
        list.updated_ms = self.now();
        list.version = list.version.wrapping_add(1);
        self.lists.replace(list.clone())?;
        Ok(list)
    }

    /// Delete a list; its tasks go to the trash.
    pub fn delete_list(&mut self, id: Uuid) -> Result<()> {
        if id == INBOX_ID {
            return invalid("the Inbox cannot be deleted");
        }
        self.list(id)?;
        let now = self.now();
        for mut task in self.tasks.in_list(id) {
            if !task.is_deleted() {
                task.deleted_ms = Some(now);
                self.save_task(task)?;
            }
        }
        self.lists.delete(id)?;
        Ok(())
    }

    // ---- tasks -------------------------------------------------------

    pub fn create_task(&mut self, new: NewTask) -> Result<Task> {
        self.live_list(new.list_id)?;
        if let Some(parent) = new.parent_id {
            let parent = self.task(parent)?;
            if parent.list_id != new.list_id {
                return invalid("a subtask must be in its parent's list");
            }
            if parent.parent_id.is_some() {
                return invalid("subtasks nest one level deep");
            }
        }
        let now = self.now();
        let mut task = Task::new(
            new.id.unwrap_or_else(Uuid::now_v7),
            new.list_id,
            &clean_title(&new.title)?,
            new.sort_order
                .unwrap_or_else(|| self.tasks.next_sort_order(new.list_id)),
            now,
        );
        task.parent_id = new.parent_id;
        task.notes = clean_notes(new.notes)?;
        task.kind = new.kind.unwrap_or(TaskKind::Text);
        task.priority = new.priority.unwrap_or(Priority::None);
        task.start_ms = clean_due(new.start_ms)?;
        task.due_ms = clean_due(new.due_ms)?;
        task.is_all_day = new.is_all_day;
        task.time_zone = clean_short(new.time_zone, "timeZone")?;
        task.reminders = clean_reminders(new.reminders)?;
        task.repeat_flag = clean_short(new.repeat_flag, "repeatFlag")?;
        task.items = clean_items(new.items)?;
        task.tags = self.clean_tags(new.tags)?;
        self.tasks.insert(task.clone())?;
        Ok(task)
    }

    /// A task, including one in the trash.
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
        if let Some(kind) = patch.kind {
            task.kind = kind;
        }
        if let Some(status) = patch.status {
            apply_status(&mut task, status, self.now());
        }
        if let Some(priority) = patch.priority {
            task.priority = priority;
        }
        if let Some(start) = patch.start_ms {
            task.start_ms = clean_due(start)?;
        }
        if let Some(due) = patch.due_ms {
            task.due_ms = clean_due(due)?;
        }
        if let Some(all_day) = patch.is_all_day {
            task.is_all_day = all_day;
        }
        if let Some(zone) = patch.time_zone {
            task.time_zone = clean_short(zone, "timeZone")?;
        }
        if let Some(reminders) = patch.reminders {
            task.reminders = clean_reminders(reminders)?;
        }
        if let Some(rule) = patch.repeat_flag {
            task.repeat_flag = clean_short(rule, "repeatFlag")?;
        }
        if let Some(ex) = patch.ex_dates {
            if ex.len() > MAX_EX_DATES {
                return invalid(format!("more than {MAX_EX_DATES} exDates"));
            }
            task.ex_dates = ex;
        }
        if let Some(items) = patch.items {
            task.items = clean_items(items)?;
        }
        if let Some(tags) = patch.tags {
            task.tags = self.clean_tags(tags)?;
        }
        if let Some(order) = patch.sort_order {
            task.sort_order = order;
        }
        let moved_to = patch.list_id.filter(|l| *l != task.list_id);
        if let Some(list) = moved_to {
            self.live_list(list)?;
            if task.parent_id.is_some() {
                return invalid("a subtask moves with its parent");
            }
            self.move_with_children(&task, list)?;
            task.list_id = list;
        }
        self.save_task(task)
    }

    /// Move `parent`'s subtasks to `list`; the parent itself is saved by the caller.
    fn move_with_children(&mut self, parent: &Task, list: Uuid) -> Result<()> {
        for mut child in self.children(parent) {
            child.list_id = list;
            self.save_task(child)?;
        }
        Ok(())
    }

    fn children(&self, parent: &Task) -> Vec<Task> {
        self.tasks
            .in_list(parent.list_id)
            .into_iter()
            .filter(|t| t.parent_id == Some(parent.id))
            .collect()
    }

    fn save_task(&mut self, mut task: Task) -> Result<Task> {
        task.updated_ms = self.now();
        task.version = task.version.wrapping_add(1);
        self.tasks.replace(task.clone())?;
        Ok(task)
    }

    pub fn set_status(&mut self, id: Uuid, status: Status) -> Result<Task> {
        self.patch_task(
            id,
            TaskPatch {
                status: Some(status),
                ..TaskPatch::default()
            },
        )
    }

    /// Move a task and its subtasks to the trash.
    pub fn trash_task(&mut self, id: Uuid) -> Result<Task> {
        let mut task = self.task(id)?;
        let now = self.now();
        for mut child in self.children(&task) {
            child.deleted_ms.get_or_insert(now);
            self.save_task(child)?;
        }
        task.deleted_ms.get_or_insert(now);
        self.save_task(task)
    }

    /// Bring a task and the subtasks trashed with it back. A task whose list
    /// is gone (or itself trashed) is restored into the Inbox.
    pub fn restore_task(&mut self, id: Uuid) -> Result<Task> {
        let mut task = self.task(id)?;
        let trashed_at = task.deleted_ms;
        if self.live_list(task.list_id).is_err() {
            let target = INBOX_ID;
            for mut child in self.children(&task) {
                child.list_id = target;
                self.save_task(child)?;
            }
            task.list_id = target;
        }
        for mut child in self.children(&task) {
            if child.deleted_ms == trashed_at {
                child.deleted_ms = None;
                self.save_task(child)?;
            }
        }
        task.deleted_ms = None;
        self.save_task(task)
    }

    /// Delete a task and its subtasks for good.
    pub fn purge_task(&mut self, id: Uuid) -> Result<()> {
        let task = self.task(id)?;
        let mut gone = vec![id];
        for child in self.children(&task) {
            self.tasks.delete(child.id)?;
            gone.push(child.id);
        }
        self.tasks.delete(id)?;
        self.docs.delete_comments_of(&gone)?;
        Ok(())
    }

    /// Permanently delete everything in the trash; returns how many tasks went.
    pub fn empty_trash(&mut self) -> Result<usize> {
        let trashed: Vec<Uuid> = self
            .tasks
            .all()
            .into_iter()
            .filter(Task::is_deleted)
            .map(|t| t.id)
            .collect();
        for id in &trashed {
            self.tasks.delete(*id)?;
        }
        self.docs.delete_comments_of(&trashed)?;
        Ok(trashed.len())
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
        tasks.retain(|t| !t.is_deleted());
        if let Some(status) = status {
            tasks.retain(|t| t.status == status);
        }
        Ok(tasks)
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            inbox_id: INBOX_ID,
            server_time_ms: self.now(),
            lists: self.lists.all(),
            tasks: self.tasks.all(),
            tags: self.tags.all(),
        }
    }

    // ---- search and smart lists --------------------------------------

    /// Tasks matching any of the whitespace-separated `terms` in title or
    /// notes; each term also matches as a prefix (type-ahead).
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
            .with_tag(&tag_name(tag))
            .into_iter()
            .filter_map(|id| self.tasks.get(id))
            .collect()
    }

    /// Open, live tasks due in `[from, to)` across every non-archived list,
    /// soonest first. One range query per list.
    fn due_window(&self, from: i64, to: i64) -> Vec<Task> {
        let mut tasks: Vec<Task> = self
            .lists
            .all()
            .into_iter()
            .filter(|l| !l.archived)
            .flat_map(|l| self.tasks.due_between(l.id, from, to))
            .filter(|t| t.status == Status::Open && !t.is_deleted())
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

    // ---- tags --------------------------------------------------------

    pub fn tags(&self) -> Vec<Tag> {
        self.tags.all()
    }

    pub fn tag(&self, name: &str) -> Result<Tag> {
        self.tags
            .get(&tag_name(name))
            .ok_or(ServiceError::NotFound("tag"))
    }

    pub fn create_tag(&mut self, label: &str, color: Option<String>) -> Result<Tag> {
        let label = clean_tag_label(label)?;
        if self.tags.get(&tag_name(&label)).is_some() {
            return Err(ServiceError::Conflict(format!(
                "tag {label:?} already exists"
            )));
        }
        let mut tag = Tag::new(&label, self.next_tag_order());
        tag.color = clean_color(color)?;
        self.tags.insert(tag.clone())?;
        Ok(tag)
    }

    pub fn patch_tag(&mut self, name: &str, patch: TagPatch) -> Result<Tag> {
        let mut tag = self.tag(name)?;
        if let Some(color) = patch.color {
            tag.color = clean_color(color)?;
        }
        if let Some(parent) = patch.parent {
            tag.parent = match parent {
                None => None,
                Some(p) => {
                    let p = tag_name(&p);
                    if p == tag.name || self.tags.get(&p).is_none() {
                        return invalid("parent must be another existing tag");
                    }
                    Some(p)
                }
            };
        }
        if let Some(order) = patch.sort_order {
            tag.sort_order = order;
        }
        tag.version = tag.version.wrapping_add(1);
        self.tags.replace(tag.clone())?;
        Ok(tag)
    }

    /// Rename a tag everywhere: the entity, its children's `parent`, and every
    /// task that carries it.
    pub fn rename_tag(&mut self, name: &str, new_label: &str) -> Result<Tag> {
        let old = self.tag(name)?;
        let label = clean_tag_label(new_label)?;
        let new_name = tag_name(&label);
        if new_name != old.name && self.tags.get(&new_name).is_some() {
            return Err(ServiceError::Conflict(format!(
                "tag {label:?} already exists"
            )));
        }
        let mut renamed = old.clone();
        renamed.label = label;
        renamed.name = new_name.clone();
        renamed.version = renamed.version.wrapping_add(1);
        if new_name != old.name {
            self.tags.delete(&old.name)?;
            self.tags.insert(renamed.clone())?;
            for mut child in self
                .tags
                .all()
                .into_iter()
                .filter(|t| t.parent.as_deref() == Some(&old.name))
            {
                child.parent = Some(new_name.clone());
                self.tags.replace(child)?;
            }
            self.retag(&old.name, Some(&new_name))?;
        } else {
            self.tags.replace(renamed.clone())?;
        }
        Ok(renamed)
    }

    /// Delete a tag and take it off every task.
    pub fn delete_tag(&mut self, name: &str) -> Result<()> {
        let tag = self.tag(name)?;
        self.tags.delete(&tag.name)?;
        for mut child in self
            .tags
            .all()
            .into_iter()
            .filter(|t| t.parent.as_deref() == Some(&tag.name))
        {
            child.parent = None;
            self.tags.replace(child)?;
        }
        self.retag(&tag.name, None)
    }

    /// Replace tag `from` with `to` (or drop it) on every task, trashed ones too.
    fn retag(&mut self, from: &str, to: Option<&str>) -> Result<()> {
        for mut task in self
            .tasks
            .all()
            .into_iter()
            .filter(|t| t.tags.iter().any(|g| g == from))
        {
            let mut seen = HashSet::new();
            task.tags = task
                .tags
                .iter()
                .filter_map(|g| {
                    if g == from {
                        to.map(str::to_string)
                    } else {
                        Some(g.clone())
                    }
                })
                .filter(|g| seen.insert(g.clone()))
                .collect();
            self.save_task(task)?;
        }
        Ok(())
    }

    fn next_tag_order(&self) -> i64 {
        self.tags
            .all()
            .last()
            .map_or(0, |t| t.sort_order.saturating_add(SORT_STEP))
    }

    /// Normalise `input` to tag names (trimmed, lowercased, de-duplicated,
    /// bounded) and create any that do not exist yet.
    fn clean_tags(&mut self, input: Vec<String>) -> Result<Vec<String>> {
        let mut names: Vec<String> = Vec::new();
        let mut labels: Vec<String> = Vec::new();
        for raw in input {
            let label = raw.trim().to_string();
            let name = tag_name(&label);
            if name.is_empty() || names.contains(&name) {
                continue;
            }
            if label.chars().count() > MAX_TAG_CHARS {
                return invalid(format!("a tag is longer than {MAX_TAG_CHARS} characters"));
            }
            names.push(name);
            labels.push(label);
        }
        if names.len() > MAX_TAGS {
            return invalid(format!("more than {MAX_TAGS} tags"));
        }
        for (name, label) in names.iter().zip(&labels) {
            if self.tags.get(name).is_none() {
                let order = self.next_tag_order();
                self.tags.insert(Tag::new(label, order))?;
            }
        }
        Ok(names)
    }

    // ---- client documents --------------------------------------------

    pub fn docs(&self, kind: &str) -> Result<Vec<Doc>> {
        check_kind(kind)?;
        Ok(self.docs.of_kind(kind))
    }

    /// Insert or replace a document; `body` must be JSON.
    pub fn put_doc(&mut self, kind: &str, id: Uuid, body: &str) -> Result<Doc> {
        check_kind(kind)?;
        if body.len() > MAX_DOC_BYTES {
            return invalid(format!("document is larger than {MAX_DOC_BYTES} bytes"));
        }
        if rusty_json::from_str::<rusty_json::Value>(body).is_err() {
            return invalid("document must be JSON");
        }
        if let Some(existing) = self.docs.get(id) {
            if existing.kind != kind {
                return Err(ServiceError::Conflict("id belongs to another kind".into()));
            }
        }
        let doc = Doc {
            id,
            kind: kind.to_string(),
            body: body.to_string(),
            updated_ms: self.now(),
        };
        self.docs.put(doc.clone())?;
        Ok(doc)
    }

    pub fn delete_doc(&mut self, kind: &str, id: Uuid) -> Result<()> {
        check_kind(kind)?;
        match self.docs.get(id) {
            Some(doc) if doc.kind == kind => Ok(self.docs.delete(id)?),
            _ => Err(ServiceError::NotFound("document")),
        }
    }

    // ---- shared helpers ----------------------------------------------

    /// A list that exists (trashed tasks do not keep lists alive).
    fn live_list(&self, id: Uuid) -> Result<TaskList> {
        self.list(id)
    }
}

/// Completing a task stamps `completed_ms`; reopening clears it.
fn apply_status(task: &mut Task, status: Status, now: i64) {
    if task.status == status {
        return;
    }
    task.status = status;
    task.completed_ms = (status == Status::Done).then_some(now);
}

/// `[start, end)` of the local day containing `now_ms`, in UTC milliseconds.
fn day_bounds(now_ms: i64, utc_offset_min: i32) -> (i64, i64) {
    let offset = i64::from(utc_offset_min) * 60_000;
    let local = now_ms.saturating_add(offset);
    let start = local - local.rem_euclid(DAY_MS) - offset;
    (start, start + DAY_MS)
}

fn check_kind(kind: &str) -> Result<()> {
    if KINDS.contains(&kind) {
        Ok(())
    } else {
        invalid(format!("unknown document kind {kind:?}"))
    }
}

/// `i64::MAX` is the store's "no due date" key, so it cannot be a real date.
fn clean_due(due: Option<i64>) -> Result<Option<i64>> {
    if due == Some(NO_DUE) {
        return invalid("date is out of range");
    }
    Ok(due)
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

fn clean_tag_label(label: &str) -> Result<String> {
    let label = label.trim();
    if label.is_empty() {
        return invalid("tag must not be empty");
    }
    if label.chars().count() > MAX_TAG_CHARS {
        return invalid(format!("tag is longer than {MAX_TAG_CHARS} characters"));
    }
    Ok(label.to_string())
}

/// `#rrggbb` or nothing.
fn clean_color(color: Option<String>) -> Result<Option<String>> {
    match color {
        None => Ok(None),
        Some(c) if is_hex_color(&c) => Ok(Some(c.to_lowercase())),
        Some(_) => invalid("color must look like #rrggbb"),
    }
}

fn is_hex_color(text: &str) -> bool {
    text.len() == 7 && text.starts_with('#') && text[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

fn clean_short(text: String, field: &str) -> Result<String> {
    if text.chars().count() > MAX_SHORT_TEXT {
        return invalid(format!(
            "{field} is longer than {MAX_SHORT_TEXT} characters"
        ));
    }
    Ok(text)
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

fn clean_reminders(reminders: Vec<String>) -> Result<Vec<String>> {
    if reminders.len() > MAX_REMINDERS {
        return invalid(format!("more than {MAX_REMINDERS} reminders"));
    }
    reminders
        .into_iter()
        .map(|r| clean_short(r, "reminder"))
        .collect()
}

fn clean_items(items: Vec<ChecklistItem>) -> Result<Vec<ChecklistItem>> {
    if items.len() > MAX_ITEMS {
        return invalid(format!("more than {MAX_ITEMS} checklist items"));
    }
    items
        .into_iter()
        .map(|mut item| {
            item.title = clean_title(&item.title)?;
            Ok(item)
        })
        .collect()
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
    fn opening_sweeps_comments_whose_task_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let clock = || Box::new(|| 1_000_i64) as Clock;
        let mut service = Service::open(dir.path(), clock()).unwrap();
        let task = service
            .create_task(NewTask {
                list_id: INBOX_ID,
                title: "alive".into(),
                ..NewTask::default()
            })
            .unwrap();
        let comment = |service: &mut Service, owner: &str| {
            let body = format!(r#"{{"v":1,"taskId":"{owner}","text":"x","createdMs":1}}"#);
            service.put_doc("comment", Uuid::now_v7(), &body).unwrap();
        };
        comment(&mut service, &task.id.to_string());
        comment(&mut service, &Uuid::now_v7().to_string()); // its task was purged long ago
        comment(&mut service, "not-a-task-id");
        assert_eq!(service.docs("comment").unwrap().len(), 3);
        drop(service);

        let service = Service::open(dir.path(), clock()).unwrap();
        let left = service.docs("comment").unwrap();
        assert_eq!(left.len(), 1, "only the live task's comment survives");
        assert!(left[0].body.contains(&task.id.to_string()));
    }

    #[test]
    fn every_client_document_kind_is_accepted() {
        for kind in KINDS {
            assert!(check_kind(kind).is_ok(), "{kind}");
        }
        assert!(KINDS.contains(&"comment"));
        assert!(check_kind("nope").is_err());
    }

    #[test]
    fn colors_must_be_hex() {
        assert_eq!(
            clean_color(Some("#ED70A5".into())).unwrap(),
            Some("#ed70a5".into())
        );
        assert!(clean_color(Some("red".into())).is_err());
        assert!(clean_color(Some("#12345".into())).is_err());
        assert_eq!(clean_color(None).unwrap(), None);
    }

    #[test]
    fn completing_stamps_and_reopening_clears() {
        let mut task = Task::new(Uuid::now_v7(), INBOX_ID, "t", 0, 0);
        apply_status(&mut task, Status::Done, 42);
        assert_eq!(task.completed_ms, Some(42));
        apply_status(&mut task, Status::Done, 99);
        assert_eq!(
            task.completed_ms,
            Some(42),
            "completing twice keeps the first stamp"
        );
        apply_status(&mut task, Status::Open, 100);
        assert_eq!(task.completed_ms, None);
    }

    /// Review 2.9: a move interrupted after some of its subtasks were saved
    /// (the parent still in its old list) is repaired on the next open:
    /// every subtask is back in its parent's list.
    #[test]
    fn an_interrupted_move_is_realigned_on_open() {
        let dir = tempfile::tempdir().unwrap();
        let clock = || -> Clock { Box::new(|| 1_000) };
        let (parent, child, other) = {
            let mut svc = Service::open(dir.path(), clock()).unwrap();
            let other = svc
                .create_list(NewList {
                    name: "Other".into(),
                    ..NewList::default()
                })
                .unwrap();
            let parent = svc
                .create_task(NewTask {
                    list_id: INBOX_ID,
                    title: "parent".into(),
                    ..NewTask::default()
                })
                .unwrap();
            let child = svc
                .create_task(NewTask {
                    list_id: INBOX_ID,
                    parent_id: Some(parent.id),
                    title: "child".into(),
                    ..NewTask::default()
                })
                .unwrap();
            // The crash: the child's save landed, the parent's did not.
            let mut moved = svc.task(child.id).unwrap();
            moved.list_id = other.id;
            svc.save_task(moved).unwrap();
            (parent.id, child.id, other.id)
        };
        let svc = Service::open(dir.path(), clock()).unwrap();
        assert_eq!(svc.task(parent).unwrap().list_id, INBOX_ID);
        assert_eq!(
            svc.task(child).unwrap().list_id,
            INBOX_ID,
            "back with its parent"
        );
        assert_ne!(other, INBOX_ID);
    }
}
