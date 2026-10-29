//! rusty_tick adapter for the [`Store`] port (ADR-0012): the board's cards are
//! rusty_tick tasks tagged `agent`, so there is one task store.
//!
//! **Mapping.** A card is a task: title, notes (body), priority, sort order, and
//! tags carrying the board's state (`board:<column>`, `claim:<actor>`, `number:<n>`) beside
//! the card's own tags, which rusty_tick keeps lowercase. Its project is the
//! task's list, named like the project; the Inbox is the `default` project. A
//! closed task (done or won't do) is Done. The
//! task's version is the card's revision (`rev = version - 1`, so a new card is
//! rev 0). Dependencies, the forge link and the execution link live in one
//! `board` document per task (id = task id); comments are rusty_tick `comment`
//! documents, so the owner sees them.
//!
//! **Commit protocol.** Every write is one request fenced on the task version
//! the board read, and every write moves that version: a task `PATCH` under
//! `If-Match` (an empty one when nothing on the task changes), or a board
//! document or comment `PUT` under `If-Task-Match`, which rusty_tick checks and
//! then moves the version under one lock. So the version names the whole card
//! (task and board document), and any write since the board's read fails with
//! 412, the board's `Conflict`. A comment is an explicit append, so one the owner
//! adds meanwhile is kept. A card read takes the task, then its documents,
//! then the task's version again, and starts over if a write moved it, so a card
//! never pairs a newer document with an older revision. (`list` reads once: a
//! card in it may be that mix, and a write made from it is refused.) Only `card_plan` with both a priority and
//! dependencies needs two requests (task, then document), each fenced.
//!
//! **Events** are not durable: rusty_tick has no change log. They are kept in
//! memory for this process: the board's own commits, plus `Edited` (or
//! `Created`) by actor `tick` for a task changed or tagged in rusty_tick,
//! noticed when events are read.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::domain::{
    BoardError, Card, Column, Comment, DEFAULT_PROJECT, Event, EventKind, ExecutionLink,
    next_event_id, reserved_tag,
};
use crate::http::Http;
use crate::store::Store;

/// The tag that puts a task on the board.
pub const AGENT_TAG: &str = "agent";
/// rusty_tick's Inbox, which every store has: the `default` project.
const INBOX: Uuid = Uuid::from_u128(0x0000_0000_0000_7000_8000_0000_0000_0001);
/// Who a comment written in rusty_tick itself is shown as.
const OWNER: &str = "owner";
/// The actor of events noticed in rusty_tick rather than made by the board.
const TICK_ACTOR: &str = "tick";

/// Reads of one card before giving up on a task that keeps moving under it.
const READ_ATTEMPTS: usize = 8;

/// Document kind of a card-number reservation (one per number, id from [`number_doc`]).
const NUMBER_KIND: &str = "board_number";

/// Numbers tried past the highest reservation seen before giving up; each miss is another
/// board reserving at the same moment.
const RESERVE_ATTEMPTS: u64 = 64;

/// The id of number `n`'s reservation: the same on every board, so `If-None-Match: *` lets
/// exactly one reserve it.
fn number_doc(n: u64) -> Uuid {
    const NAMESPACE: Uuid = Uuid::from_u128(0x6d69_6c6c_2d62_6f61_7264_2d6e_756d_6265);
    Uuid::new_v5(&NAMESPACE, &n.to_be_bytes())
}

/// The rusty_tick HTTP API, as this adapter needs it: one request under
/// `/api/v1` (or `/health`), answered with its status and JSON body
/// (`Value::Null` when empty), whatever the status.
pub trait TickApi: Send + Sync {
    /// Send one request.
    ///
    /// # Errors
    /// [`BoardError::Storage`] when rusty_tick cannot be reached.
    fn send(&self, request: &TickRequest<'_>) -> Result<(u16, Value), BoardError>;
}

/// One request to rusty_tick.
#[derive(Debug, Default)]
pub struct TickRequest<'a> {
    /// `GET`, `POST`, `PATCH` or `PUT`.
    pub method: &'a str,
    /// Below `/api/v1`, or `/health`.
    pub path: String,
    /// A JSON body.
    pub body: Option<Value>,
    /// `If-Match`.
    pub if_match: Option<String>,
    /// `If-None-Match: *`.
    pub if_absent: bool,
    /// `If-Task-Match`.
    pub if_task_match: Option<String>,
}

impl<'a> TickRequest<'a> {
    fn new(method: &'a str, path: impl Into<String>) -> Self {
        Self {
            method,
            path: path.into(),
            ..Self::default()
        }
    }

    fn body(mut self, body: Value) -> Self {
        self.body = Some(body);
        self
    }
}

/// rusty_tick over HTTP, with one user's bearer token.
pub struct TickHttp {
    base: String,
    token: String,
    http: Mutex<Http>,
}

impl TickHttp {
    /// `base` is rusty_tick's root, e.g. `http://127.0.0.1:8788`.
    ///
    /// # Errors
    /// A base that is not http(s), an empty token, or a client that cannot start.
    pub fn new(base: &str, token: &str) -> Result<Self, String> {
        if !(base.starts_with("http://") || base.starts_with("https://")) {
            return Err("--tick-url must start with http:// or https://".into());
        }
        if token.trim().is_empty() {
            return Err("RUSTY_TICK_TOKEN is empty".into());
        }
        Ok(Self {
            base: base.trim_end_matches('/').to_owned(),
            token: token.trim().to_owned(),
            http: Mutex::new(Http::new(Duration::from_secs(10))?),
        })
    }
}

impl TickApi for TickHttp {
    fn send(&self, request: &TickRequest<'_>) -> Result<(u16, Value), BoardError> {
        let url = if request.path == "/health" {
            format!("{}/health", self.base)
        } else {
            format!("{}/api/v1{}", self.base, request.path)
        };
        let bearer = format!("Bearer {}", self.token);
        let mut headers = vec![("Authorization", bearer.as_str())];
        if let Some(tag) = &request.if_match {
            headers.push(("If-Match", tag.as_str()));
        }
        if request.if_absent {
            headers.push(("If-None-Match", "*"));
        }
        if let Some(tag) = &request.if_task_match {
            headers.push(("If-Task-Match", tag.as_str()));
        }
        let body = request.body.as_ref().map(Value::to_string);
        let http = self.http.lock().map_err(poisoned)?;
        let (status, text) = http
            .send(request.method, &url, &headers, body)
            .map_err(|e| BoardError::Storage(format!("rusty_tick: {e}")))?;
        let value = if text.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text)
                .map_err(|e| BoardError::Storage(format!("rusty_tick sent bad JSON: {e}")))?
        };
        Ok((status, value))
    }
}

/// A task as rusty_tick sends it (the fields the board reads).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Task {
    id: Uuid,
    list_id: Uuid,
    title: String,
    notes: String,
    status: String,
    priority: u8,
    tags: Vec<String>,
    sort_order: i64,
    created_ms: i64,
    updated_ms: i64,
    #[serde(default)]
    completed_ms: Option<i64>,
    deleted_ms: Option<i64>,
    etag: String,
}

impl Task {
    fn version(&self) -> Result<u64, BoardError> {
        self.etag
            .parse()
            .map_err(|_| BoardError::Storage("rusty_tick sent a bad etag".into()))
    }

    fn on_board(&self) -> bool {
        self.deleted_ms.is_none() && self.tags.iter().any(|t| t == AGENT_TAG)
    }
}

#[derive(Deserialize)]
struct List {
    id: Uuid,
    name: String,
}

/// A stored document's id and body.
#[derive(Deserialize)]
struct Doc {
    id: Uuid,
    body: Value,
}

/// The board's per-task document.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BoardDoc {
    #[serde(default)]
    deps: Vec<Uuid>,
    #[serde(default)]
    issue: Option<u64>,
    #[serde(default)]
    synced_text: Option<u64>,
    #[serde(default)]
    execution: Option<ExecutionLink>,
}

impl BoardDoc {
    fn of(card: &Card) -> Self {
        Self {
            deps: card.deps.clone(),
            issue: card.issue,
            synced_text: card.synced_text,
            execution: card.execution.clone(),
        }
    }

    fn body(&self) -> Value {
        let mut body = serde_json::to_value(self).unwrap_or(Value::Null);
        if let Value::Object(map) = &mut body {
            map.insert("v".into(), json!(1));
        }
        body
    }
}

/// A `comment` document's body.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommentBody {
    task_id: String,
    text: String,
    created_ms: i64,
    #[serde(default)]
    by: Option<String>,
}

/// Everything read about one task.
struct Parts {
    task: Task,
    project: String,
    doc: Option<BoardDoc>,
    comments: Vec<Comment>,
}

/// Project names by rusty_tick list id, the Inbox left out.
struct Projects(BTreeMap<Uuid, String>);

impl Projects {
    fn of(&self, task: &Task) -> String {
        self.0
            .get(&task.list_id)
            .cloned()
            .unwrap_or_else(|| DEFAULT_PROJECT.to_owned())
    }
}

/// Card fields that live on the task itself.
fn task_tags(card: &Card) -> Vec<String> {
    let column = serde_json::to_value(card.column)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    let mut tags = vec![AGENT_TAG.to_owned(), format!("board:{column}")];
    tags.extend(card.claim.iter().map(|c| format!("claim:{c}")));
    tags.extend(card.number.iter().map(|n| format!("number:{n}")));
    tags.extend(card.tags.iter().cloned());
    tags
}

fn card_of(parts: Parts) -> Result<Card, BoardError> {
    let Parts {
        task,
        project,
        doc,
        comments,
    } = parts;
    let rev = task.version()?.saturating_sub(1);
    let column = if task.status == "open" {
        task.tags
            .iter()
            .find_map(|t| t.strip_prefix("board:"))
            .and_then(|c| serde_json::from_value(json!(c)).ok())
            .unwrap_or(Column::Backlog)
    } else {
        Column::Done
    };
    let claim = task
        .tags
        .iter()
        .find_map(|t| t.strip_prefix("claim:"))
        .map(str::to_owned);
    let number = task
        .tags
        .iter()
        .find_map(|t| t.strip_prefix("number:"))
        .and_then(|n| n.parse().ok());
    let tags = task
        .tags
        .iter()
        .filter(|t| !reserved_tag(t))
        .cloned()
        .collect();
    let doc = doc.unwrap_or_default();
    Ok(Card {
        id: task.id,
        number,
        rev,
        title: task.title,
        body: task.notes,
        column,
        claim,
        tags,
        project,
        comments,
        priority: task.priority,
        deps: doc.deps,
        sort_order: task.sort_order,
        issue: doc.issue,
        synced_text: doc.synced_text,
        execution: doc.execution,
        created_ms: task.created_ms,
        updated_ms: task.updated_ms,
        // Only shipped work has a ship date; won't-do is closed but never shipped.
        done_ms: task.completed_ms.filter(|_| task.status == "done"),
    })
}

/// In-memory event log, and the task versions last seen, to notice edits made in rusty_tick.
#[derive(Default)]
struct Events {
    log: Vec<Event>,
    seen: BTreeMap<Uuid, u64>,
    primed: bool,
}

impl Events {
    fn push(&mut self, mut event: Event) {
        event.id = next_event_id(self.log.last().map(|e| e.id));
        self.log.push(event);
    }
}

fn poisoned<T>(_: T) -> BoardError {
    BoardError::Storage("lock poisoned".into())
}

/// The board's store, kept in rusty_tick.
pub struct TickStore<T: TickApi> {
    tick: T,
    events: Mutex<Events>,
}

impl<T: TickApi> TickStore<T> {
    /// A store over `tick`. Nothing is read until the board asks.
    pub fn new(tick: T) -> Self {
        Self {
            tick,
            events: Mutex::default(),
        }
    }

    /// Send, and refuse any status but `ok`: 412 is a conflict, 404 a missing record.
    fn call(&self, request: TickRequest<'_>, ok: u16) -> Result<Value, BoardError> {
        let (status, body) = self.tick.send(&request)?;
        match status {
            s if s == ok => Ok(body),
            412 => Err(BoardError::Conflict),
            _ => Err(BoardError::Storage(format!(
                "rusty_tick answered {status} to {} {}: {}",
                request.method,
                request.path,
                body.pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("no message")
            ))),
        }
    }

    fn decode<D: for<'de> Deserialize<'de>>(value: Value, key: &str) -> Result<D, BoardError> {
        serde_json::from_value(value[key].clone())
            .map_err(|e| BoardError::Storage(format!("rusty_tick sent an unexpected {key}: {e}")))
    }

    fn lists(&self) -> Result<Vec<List>, BoardError> {
        Self::decode(self.call(TickRequest::new("GET", "/lists"), 200)?, "lists")
    }

    /// Project names by list id. The Inbox, and a list that is gone, are `default`.
    fn projects(&self) -> Result<Projects, BoardError> {
        Ok(Projects(
            self.lists()?
                .into_iter()
                .filter(|l| l.id != INBOX)
                .map(|l| (l.id, l.name))
                .collect(),
        ))
    }

    /// The list a project's tasks live in, created if there is none yet.
    fn list_for(&self, project: &str) -> Result<Uuid, BoardError> {
        if project == DEFAULT_PROJECT {
            return Ok(INBOX);
        }
        if let Some((id, _)) = self.projects()?.0.into_iter().find(|(_, n)| n == project) {
            return Ok(id);
        }
        let created = self.call(
            TickRequest::new("POST", "/lists").body(json!({ "name": project })),
            201,
        )?;
        serde_json::from_value(created["id"].clone())
            .map_err(|_| BoardError::Storage("rusty_tick sent a list without an id".into()))
    }

    fn task(&self, id: Uuid) -> Result<Option<Task>, BoardError> {
        let (status, body) = self
            .tick
            .send(&TickRequest::new("GET", format!("/tasks/{id}")))?;
        match status {
            200 => serde_json::from_value(body)
                .map(Some)
                .map_err(|e| BoardError::Storage(format!("rusty_tick sent a bad task: {e}"))),
            404 => Ok(None),
            _ => Err(BoardError::Storage(format!(
                "rusty_tick answered {status} reading a task"
            ))),
        }
    }

    fn board_docs(&self) -> Result<BTreeMap<Uuid, BoardDoc>, BoardError> {
        let docs: Vec<Doc> = Self::decode(
            self.call(TickRequest::new("GET", "/docs/board"), 200)?,
            "docs",
        )?;
        Ok(docs
            .into_iter()
            .filter_map(|d| {
                let body = serde_json::from_value(d.body).ok()?;
                Some((d.id, body))
            })
            .collect())
    }

    fn comments(&self) -> Result<BTreeMap<Uuid, Vec<Comment>>, BoardError> {
        let docs: Vec<Doc> = Self::decode(
            self.call(TickRequest::new("GET", "/docs/comment"), 200)?,
            "docs",
        )?;
        let mut by_task: BTreeMap<Uuid, Vec<Comment>> = BTreeMap::new();
        for doc in docs {
            let Ok(body) = serde_json::from_value::<CommentBody>(doc.body) else {
                continue;
            };
            let Ok(task) = Uuid::parse_str(&body.task_id) else {
                continue;
            };
            by_task.entry(task).or_default().push(Comment {
                by: body.by.unwrap_or_else(|| OWNER.to_owned()),
                at_ms: body.created_ms,
                text: body.text,
            });
        }
        for comments in by_task.values_mut() {
            comments.sort_by_key(|c| c.at_ms);
        }
        Ok(by_task)
    }

    fn tagged(&self) -> Result<Vec<Task>, BoardError> {
        let tasks: Vec<Task> = Self::decode(
            self.call(
                TickRequest::new("GET", format!("/tags/{AGENT_TAG}/tasks")),
                200,
            )?,
            "tasks",
        )?;
        Ok(tasks.into_iter().filter(Task::on_board).collect())
    }

    /// Everything about one task, or `None` when it is not on the board. The task is
    /// read again after its documents: if a write moved it meanwhile, the documents may
    /// be newer than the version, so the read starts over. [`BoardError::Conflict`] if
    /// the task keeps moving.
    fn parts(&self, id: Uuid) -> Result<Option<Parts>, BoardError> {
        for _ in 0..READ_ATTEMPTS {
            let Some(task) = self.task(id)?.filter(Task::on_board) else {
                return Ok(None);
            };
            let project = self.projects()?.of(&task);
            let doc = self.board_docs()?.remove(&id);
            let comments = self.comments()?.remove(&id).unwrap_or_default();
            let after = self.task(id)?.map(|t| t.version()).transpose()?;
            if after != Some(task.version()?) {
                continue;
            }
            return Ok(Some(Parts {
                task,
                project,
                doc,
                comments,
            }));
        }
        Err(BoardError::Conflict)
    }

    /// The task changes from `current` to `next`, as a rusty_tick `PATCH` body.
    fn task_patch(&self, current: &Card, next: &Card) -> Result<Value, BoardError> {
        let mut patch = serde_json::Map::new();
        if next.title != current.title {
            patch.insert("title".into(), json!(next.title));
        }
        if next.body != current.body {
            patch.insert("notes".into(), json!(next.body));
        }
        if next.priority != current.priority {
            patch.insert("priority".into(), json!(next.priority));
        }
        if task_tags(next) != task_tags(current) {
            patch.insert("tags".into(), json!(task_tags(next)));
        }
        // Done is terminal on the board, so a card only ever closes its task.
        if next.column == Column::Done && current.column != Column::Done {
            patch.insert("status".into(), json!("done"));
        }
        if next.project != current.project {
            patch.insert("listId".into(), json!(self.list_for(&next.project)?));
        }
        Ok(Value::Object(patch))
    }

    /// `PATCH` the task if it is still at `version`; returns its new version.
    fn patch_task(&self, id: Uuid, patch: Value, version: u64) -> Result<u64, BoardError> {
        let mut request = TickRequest::new("PATCH", format!("/tasks/{id}")).body(patch);
        request.if_match = Some(version.to_string());
        let task: Task = serde_json::from_value(self.call(request, 200)?)
            .map_err(|e| BoardError::Storage(format!("rusty_tick sent a bad task: {e}")))?;
        task.version()
    }

    /// Write the card's board document if its task is still at `version`.
    /// rusty_tick moves the task's version in the same locked step; returns it.
    fn put_doc(&self, card: &Card, version: u64) -> Result<u64, BoardError> {
        let mut request = TickRequest::new("PUT", format!("/docs/board/{}", card.id))
            .body(BoardDoc::of(card).body());
        request.if_task_match = Some(version.to_string());
        self.call(request, 200)?;
        Ok(version + 1)
    }

    /// Append `comment` if the task is still at `version`, moving it as `put_doc` does.
    fn put_comment(&self, card: &Card, comment: &Comment, version: u64) -> Result<u64, BoardError> {
        let mut request = TickRequest::new("PUT", format!("/docs/comment/{}", Uuid::now_v7()))
            .body(json!({
                "v": 1,
                "taskId": card.id,
                "text": comment.text,
                "createdMs": comment.at_ms,
                "by": comment.by,
            }));
        request.if_absent = true;
        request.if_task_match = Some(version.to_string());
        self.call(request, 200)?;
        Ok(version + 1)
    }

    /// Note the board's own write, which left the task at `version`; returns the
    /// card revision committed (`version - 1`).
    fn record(&self, mut event: Event, version: u64) -> Result<u64, BoardError> {
        let rev = version.saturating_sub(1);
        event.rev = rev;
        let mut events = self.events.lock().map_err(poisoned)?;
        events.seen.insert(event.card, version);
        events.push(event);
        Ok(rev)
    }

    /// Note tasks changed or newly tagged in rusty_tick since they were last seen.
    fn observe(&self) -> Result<(), BoardError> {
        let tasks = self.tagged()?;
        let mut events = self.events.lock().map_err(poisoned)?;
        let primed = std::mem::replace(&mut events.primed, true);
        for task in tasks {
            let version = task.version()?;
            let before = events.seen.insert(task.id, version);
            if !primed || before == Some(version) {
                continue;
            }
            let kind = if before.is_some() {
                EventKind::Edited
            } else {
                EventKind::Created
            };
            let card = card_of(Parts {
                task,
                project: String::new(),
                doc: None,
                comments: Vec::new(),
            })?;
            events.push(Event::of(&card, kind, TICK_ACTOR));
        }
        Ok(())
    }
}

/// Refuse a claim rusty_tick would change: tags are matched without case there.
fn check_claim(card: &Card) -> Result<(), BoardError> {
    if card.claim.as_ref().is_some_and(|c| *c != c.to_lowercase()) {
        return Err(BoardError::Invalid(
            "with a rusty_tick store, actor names must be lowercase",
        ));
    }
    Ok(())
}

impl<T: TickApi> Store for TickStore<T> {
    fn get(&self, id: Uuid) -> Result<Option<Card>, BoardError> {
        self.parts(id)?.map(card_of).transpose()
    }

    fn list(&self) -> Result<Vec<Card>, BoardError> {
        let tasks = self.tagged()?;
        let projects = self.projects()?;
        let mut docs = self.board_docs()?;
        let mut comments = self.comments()?;
        let mut cards = tasks
            .into_iter()
            .map(|task| {
                let project = projects.of(&task);
                let doc = docs.remove(&task.id);
                let comments = comments.remove(&task.id).unwrap_or_default();
                card_of(Parts {
                    task,
                    project,
                    doc,
                    comments,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        cards.sort_by_key(|c| (c.created_ms, c.id));
        Ok(cards)
    }

    fn insert(&self, card: &Card, event: Event) -> Result<u64, BoardError> {
        check_claim(card)?;
        let mut body = json!({
            "id": card.id,
            "listId": self.list_for(&card.project)?,
            "title": card.title,
            "notes": card.body,
            "priority": card.priority,
            "tags": task_tags(card),
        });
        // A card recorded straight in Done is created closed, in the same request: never an
        // open task left behind if a second write failed.
        if card.column == Column::Done {
            body["status"] = json!("done");
        }
        let (status, _) = self
            .tick
            .send(&TickRequest::new("POST", "/tasks").body(body))?;
        match status {
            201 => {}
            409 => return Err(BoardError::Conflict),
            _ => {
                return Err(BoardError::Storage(format!(
                    "rusty_tick answered {status} creating a task"
                )));
            }
        }
        // A second request: a crash before it leaves the task without its document. Only
        // an import has one at insert, and its id names its issue, so the sync relinks it.
        let version = if BoardDoc::of(card) == BoardDoc::default() {
            1
        } else {
            self.put_doc(card, 1)?
        };
        self.record(event, version)
    }

    fn replace(&self, card: &Card, event: Event) -> Result<u64, BoardError> {
        check_claim(card)?;
        // The task version the board read the card at (`rev = version - 1`, and the
        // next revision is one more).
        let read = card.rev;
        let current = self
            .parts(card.id)?
            .map(card_of)
            .transpose()?
            .ok_or(BoardError::NotFound(card.id))?;
        // Every write moves the version, so an unchanged version means the task and its
        // board document are as the board read them; each request below is fenced on it.
        if current.rev + 1 != read {
            return Err(BoardError::Conflict);
        }
        let version = if event.kind == EventKind::Commented {
            // An explicit append: comments others add meanwhile do not move the version
            // and are kept.
            let comment = card
                .comments
                .last()
                .ok_or_else(|| BoardError::Storage("a comment event carries no comment".into()))?;
            self.put_comment(card, comment, read)?
        } else {
            let patch = self.task_patch(&current, card)?;
            let task_changed = patch.as_object().is_some_and(|p| !p.is_empty());
            let doc_changed = BoardDoc::of(card) != BoardDoc::of(&current);
            let mut version = read;
            // An empty patch still moves the version, so every write is a compare-and-swap.
            if task_changed || !doc_changed {
                version = self.patch_task(card.id, patch, version)?;
            }
            // Both change only for `card_plan` with priority and dependencies: two fenced
            // requests. A failure between them reports an error with the priority applied;
            // the same plan again completes it.
            if doc_changed {
                version = self.put_doc(card, version)?;
            }
            version
        };
        self.record(event, version)
    }

    fn events_since(&self, after: Option<Uuid>, limit: usize) -> Result<Vec<Event>, BoardError> {
        self.observe()?;
        let events = self.events.lock().map_err(poisoned)?;
        let start = after.map_or(0, |a| events.log.partition_point(|e| e.id <= a));
        Ok(events.log[start..].iter().take(limit).cloned().collect())
    }

    fn last_event_id(&self) -> Result<Option<Uuid>, BoardError> {
        self.observe()?;
        Ok(self
            .events
            .lock()
            .map_err(poisoned)?
            .log
            .last()
            .map(|e| e.id))
    }

    /// Reservations are `board_number` documents, created with `If-None-Match: *` and kept
    /// whatever happens to the task, so a number never comes back.
    fn reserve_number(&self, card: Uuid, floor: u64) -> Result<u64, BoardError> {
        let docs: Vec<Doc> = Self::decode(
            self.call(TickRequest::new("GET", format!("/docs/{NUMBER_KIND}")), 200)?,
            "docs",
        )?;
        let reserved = docs
            .iter()
            .filter_map(|d| d.body["number"].as_u64())
            .max()
            .unwrap_or(0);
        let first = reserved.max(floor) + 1;
        for n in first..first + RESERVE_ATTEMPTS {
            let mut request =
                TickRequest::new("PUT", format!("/docs/{NUMBER_KIND}/{}", number_doc(n)))
                    .body(json!({ "v": 1, "number": n, "taskId": card }));
            request.if_absent = true;
            match self.tick.send(&request)?.0 {
                200 => return Ok(n),
                412 => continue,
                status => {
                    return Err(BoardError::Storage(format!(
                        "rusty_tick answered {status} reserving card number {n}"
                    )));
                }
            }
        }
        Err(BoardError::Conflict)
    }

    fn health(&self) -> Result<(), BoardError> {
        let (status, _) = self.tick.send(&TickRequest::new("GET", "/health"))?;
        if status == 200 {
            Ok(())
        } else {
            Err(BoardError::Storage(format!(
                "rusty_tick health is {status}"
            )))
        }
    }
}
