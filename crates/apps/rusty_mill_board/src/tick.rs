//! rusty_tick adapter for the [`Store`] port (ADR-0012): the board's cards are
//! rusty_tick tasks tagged `agent`, so there is one task store.
//!
//! **Mapping.** A card is a task: title, notes (body), priority, sort order, and
//! tags carrying the board's state (`board:<column>`, `claim:<actor>`, `number:<n>`,
//! `archived:<ms>`, `restored:<ms>`) beside
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

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::domain::{
    BoardError, Card, CloseReason, Closure, Column, Comment, DEFAULT_PROJECT, Event, EventKind,
    ExecutionLink, Span, next_event_id, reserved_tag,
};
use crate::external::{ExternalRef, Provider, RefState};
use crate::http::Http;
use crate::milestones::{Milestone, MilestoneStore};
use crate::store::Store;

/// The tag that puts a task on the board.
pub const AGENT_TAG: &str = "agent";
/// The tag that makes a rusty_tick task a milestone (see [`crate::milestones`]).
pub const MILESTONE_TAG: &str = "milestone";
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
/// rusty_tick's limit on one document's serialized body.
const MAX_DOC_BYTES: usize = 64 * 1024;

/// Moves asked for per `GET /moves` page: rusty_tick's largest.
const MOVES_PAGE: usize = 1000;
/// How often `observe` reads every card instead of only those changed since the last read,
/// to catch anything a delta read missed (a clock step in rusty_tick, a lost response).
const RECONCILE_EVERY: Duration = Duration::from_secs(5 * 60);
/// How far before the newest comment seen each comment read starts again: comment ids are
/// UUIDv7 from each writer's clock, so a writer a little behind still sorts into the window.
const COMMENT_OVERLAP_MS: u64 = 60_000;

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
    #[serde(default)]
    due_ms: Option<i64>,
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

    /// A milestone: tagged `milestone`, never `agent` (that would make it a card), with a date.
    fn is_milestone(&self) -> bool {
        self.deleted_ms.is_none()
            && self.due_ms.is_some()
            && self.tags.iter().any(|t| t == MILESTONE_TAG)
            && !self.tags.iter().any(|t| t == AGENT_TAG)
    }
}

/// One entry of rusty_tick's move history: which task changed column, and when.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MoveRecord {
    task_id: Uuid,
    at_ms: i64,
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
    // Left out while empty, so documents written before these links existed are unchanged.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    superseded_by: Vec<Uuid>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    related: Vec<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    milestone: Option<Uuid>,
    #[serde(default)]
    issue: Option<u64>,
    #[serde(default)]
    synced_text: Option<u64>,
    #[serde(default)]
    execution: Option<ExecutionLink>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    target_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    external_refs: Vec<StoredRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    evidence: Vec<String>,
    /// Fields this binary does not know, written back unchanged, so a newer board's fields
    /// survive this one's writes (a rollback loses nothing; ADR-0006).
    #[serde(flatten)]
    unknown: serde_json::Map<String, Value>,
}

impl BoardDoc {
    fn of(card: &Card) -> Self {
        Self {
            deps: card.deps.clone(),
            superseded_by: card.superseded_by.clone(),
            related: card.related.clone(),
            parent: card.parent,
            milestone: card.milestone,
            issue: card.issue,
            synced_text: card.synced_text,
            execution: card.execution.clone(),
            prompt: card.prompt.clone(),
            target_ms: card.target_ms,
            external_refs: card
                .external_refs
                .iter()
                .cloned()
                .map(StoredRef::from)
                .collect(),
            evidence: card.evidence.clone(),
            unknown: serde_json::Map::new(),
        }
    }

    /// `card`'s document, keeping the fields of `stored` this binary does not know.
    fn kept(card: &Card, stored: Option<&Self>) -> Self {
        let mut unknown = stored.map(|d| d.unknown.clone()).unwrap_or_default();
        unknown.remove("v"); // the format version, which `body` writes
        Self {
            unknown,
            ..Self::of(card)
        }
    }

    fn body(&self) -> Value {
        let mut body = serde_json::to_value(self).unwrap_or(Value::Null);
        if let Value::Object(map) = &mut body {
            map.insert("v".into(), json!(1));
        }
        body
    }

    /// This document's body, refused before any request when rusty_tick would refuse it, so
    /// an edit never half-lands on a document that cannot be stored.
    fn checked(&self) -> Result<Value, BoardError> {
        let body = self.body();
        let bytes = serde_json::to_vec(&body)
            .map_err(|e| BoardError::Storage(format!("board document: {e}")))?;
        if bytes.len() > MAX_DOC_BYTES {
            return Err(BoardError::Invalid(
                "the card's prompt and links are larger than 64 KiB once stored",
            ));
        }
        Ok(body)
    }
}

/// An [`ExternalRef`] as stored: camelCase, like the rest of the document (ADR-0006).
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredRef {
    provider: Provider,
    #[serde(rename = "ref")]
    reference: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    state: RefState,
    observed_at_ms: i64,
    by: String,
}

impl From<ExternalRef> for StoredRef {
    fn from(r: ExternalRef) -> Self {
        let ExternalRef {
            provider,
            reference,
            url,
            state,
            observed_at_ms,
            by,
        } = r;
        Self {
            provider,
            reference,
            url,
            state,
            observed_at_ms,
            by,
        }
    }
}

impl From<StoredRef> for ExternalRef {
    fn from(r: StoredRef) -> Self {
        let StoredRef {
            provider,
            reference,
            url,
            state,
            observed_at_ms,
            by,
        } = r;
        Self {
            provider,
            reference,
            url,
            state,
            observed_at_ms,
            by,
        }
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
    let mut tags = vec![AGENT_TAG.to_owned(), format!("board:{}", card.column)];
    tags.extend(card.claim.iter().map(|c| format!("claim:{c}")));
    tags.extend(card.number.iter().map(|n| format!("number:{n}")));
    tags.extend(card.archived_ms.iter().map(|ms| format!("archived:{ms}")));
    tags.extend(card.restored_ms.iter().map(|ms| format!("restored:{ms}")));
    tags.extend(
        card.closed
            .iter()
            .map(|c| format!("closed:{}:{}", c.reason.as_str(), c.at_ms)),
    );
    tags.extend(card.tags.iter().cloned());
    tags
}

/// rusty_tick's status for a Done card: `wontdo` when it closed without shipping.
fn closed_status(card: &Card) -> &'static str {
    match card.closed {
        Some(_) => "wontdo",
        None => "done",
    }
}

/// How a closed task's card was dropped: its `closed:<reason>:<ms>` tag, or, for a task
/// marked won't-do in rusty_tick itself (or whose tag is unreadable), `wont_do` at the time
/// tick closed it. `None` for open and done tasks. A task reopened in rusty_tick loses its tag
/// on the board's next write, since an open card has no `closed`.
fn closure_of(task: &Task) -> Option<Closure> {
    if task.status != "wontdo" {
        return None;
    }
    // A task closed before rusty_tick stamped close times falls back to its creation, which
    // never moves, so it never drifts into recent counts.
    let tick_ms = task.completed_ms.unwrap_or(task.created_ms);
    let tagged = task.tags.iter().find_map(|t| {
        let raw = t.strip_prefix("closed:")?;
        let parsed = raw.split_once(':').and_then(|(reason, ms)| {
            Some(Closure {
                reason: CloseReason::parse(reason)?,
                at_ms: ms.parse().ok()?,
            })
        });
        if parsed.is_none() {
            rusty_log::warn!(task = %task.id, tag = %t, "unreadable closed tag; reading the card as won't-do");
        }
        parsed
    });
    tagged.or(Some(Closure {
        reason: CloseReason::WontDo,
        at_ms: tick_ms,
    }))
}

/// The column a task's card is in: its `board:` tag while open, else Done.
fn column_of(task: &Task) -> Column {
    if task.status != "open" {
        return Column::DONE;
    }
    task.tags
        .iter()
        .find_map(|t| t.strip_prefix("board:"))
        .and_then(|c| Column::new(c).ok())
        .unwrap_or(Column::BACKLOG)
}

fn card_of(parts: Parts) -> Result<Card, BoardError> {
    let Parts {
        task,
        project,
        doc,
        comments,
    } = parts;
    let rev = task.version()?.saturating_sub(1);
    let column = column_of(&task);
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
    // Only a Done card is archived or restored; such a tag added by hand to an open task
    // means nothing.
    let shelf_tag = |prefix: &str| -> Option<i64> {
        task.tags
            .iter()
            .find_map(|t| t.strip_prefix(prefix))
            .and_then(|ms| ms.parse().ok())
            .filter(|_| column == Column::DONE)
    };
    let archived_ms = shelf_tag("archived:");
    let restored_ms = shelf_tag("restored:");
    let tags = task
        .tags
        .iter()
        .filter(|t| !reserved_tag(t))
        .cloned()
        .collect();
    let doc = doc.unwrap_or_default();
    let closed = closure_of(&task);
    Ok(Card {
        id: task.id,
        number,
        rev,
        title: task.title,
        body: task.notes,
        prompt: doc.prompt,
        column,
        claim,
        tags,
        project,
        comments,
        priority: task.priority,
        deps: doc.deps,
        superseded_by: doc.superseded_by,
        related: doc.related,
        parent: doc.parent,
        milestone: doc.milestone,
        sort_order: task.sort_order,
        issue: doc.issue,
        synced_text: doc.synced_text,
        execution: doc.execution,
        external_refs: doc
            .external_refs
            .into_iter()
            .map(ExternalRef::from)
            .collect(),
        created_ms: task.created_ms,
        updated_ms: task.updated_ms,
        // Only shipped work has a ship date; won't-do is closed but never shipped.
        done_ms: task.completed_ms.filter(|_| task.status == "done"),
        closed,
        evidence: doc.evidence,
        archived_ms,
        restored_ms,
        target_ms: doc.target_ms,
    })
}

/// In-memory event log, and the task versions last seen, to notice edits made in rusty_tick.
#[derive(Default)]
struct Events {
    log: Vec<Event>,
    seen: BTreeMap<Uuid, u64>,
    /// The newest `updatedMs` observed: the next delta read asks for tasks changed at or
    /// after it, so writes in the same millisecond are read again rather than missed.
    high_water: i64,
    /// When `observe` last read every card; `None` before the first read.
    reconciled: Option<Instant>,
    /// When (Unix ms, this board's clock) the last comment read began; the next starts
    /// [`COMMENT_OVERLAP_MS`] before it. Not the newest comment id: a writer whose clock
    /// runs fast would push that ahead and hide comments from correct clocks.
    comments_read_ms: Option<u64>,
    /// Comments already accounted for: written by this board, or observed.
    comments_seen: BTreeSet<Uuid>,
    /// Cards whose rusty_tick write has started but is not yet in `log`.
    /// Observation shares this lock, so it cannot mistake that intermediate
    /// state for an outside write.
    writing: BTreeMap<Uuid, usize>,
    write_epoch: u64,
    primed: bool,
}

impl Events {
    fn push(&mut self, mut event: Event) {
        event.id = next_event_id(self.log.last().map(|e| e.id));
        self.log.push(event);
    }
}

/// Where a comment read starts: [`COMMENT_OVERLAP_MS`] before `since`, the start of the
/// last comment read (or now, for the first).
fn comment_window(since: u64) -> Uuid {
    let from = since.saturating_sub(COMMENT_OVERLAP_MS);
    // A UUIDv7 holds its millisecond in the top 48 bits; all else zero sorts first.
    Uuid::from_u128(u128::from(from) << 80)
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn poisoned<T>(_: T) -> BoardError {
    BoardError::Storage("lock poisoned".into())
}

/// The board's store, kept in rusty_tick.
pub struct TickStore<T: TickApi> {
    tick: T,
    events: Mutex<Events>,
    reconcile_every: Duration,
}

impl<T: TickApi> TickStore<T> {
    /// A store over `tick`. Nothing is read until the board asks.
    pub fn new(tick: T) -> Self {
        Self {
            tick,
            events: Mutex::default(),
            reconcile_every: RECONCILE_EVERY,
        }
    }

    /// Read every card at least this often when observing, instead of every five minutes.
    #[must_use]
    pub fn with_reconcile_every(mut self, every: Duration) -> Self {
        self.reconcile_every = every;
        self
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

    fn decode<D: for<'de> Deserialize<'de>>(mut value: Value, key: &str) -> Result<D, BoardError> {
        serde_json::from_value(value[key].take())
            .map_err(|e| BoardError::Storage(format!("rusty_tick sent an unexpected {key}: {e}")))
    }

    /// Read every bounded storage page, retaining complete records for the Store contract.
    fn collection<D: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        key: &str,
    ) -> Result<Vec<D>, BoardError> {
        self.collection_after(path, key, None)
    }

    /// As [`Self::collection`], but only records whose id sorts after `start`. The start
    /// rides the page cursor, because rusty_tick reads only the first `after` in a query.
    fn collection_after<D: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        key: &str,
        start: Option<Uuid>,
    ) -> Result<Vec<D>, BoardError> {
        let separator = if path.contains('?') { '&' } else { '?' };
        let mut after = start;
        let mut records = Vec::new();
        loop {
            let mut target = format!("{path}{separator}limit=1000");
            if let Some(cursor) = after {
                target.push_str(&format!("&after={cursor}"));
            }
            let mut page = self.call(TickRequest::new("GET", target), 200)?;
            let next = match page.get("next") {
                None | Some(Value::Null) => None,
                Some(value) => {
                    Some(serde_json::from_value::<Uuid>(value.clone()).map_err(|_| {
                        BoardError::Storage("rusty_tick sent a bad page cursor".into())
                    })?)
                }
            };
            if next.is_some_and(|next| after.is_some_and(|after| next <= after)) {
                return Err(BoardError::Storage(
                    "rusty_tick page cursor did not advance".into(),
                ));
            }
            let batch: Vec<D> = serde_json::from_value(page[key].take()).map_err(|e| {
                BoardError::Storage(format!("rusty_tick sent an unexpected {key}: {e}"))
            })?;
            if next.is_some() && batch.is_empty() {
                return Err(BoardError::Storage(
                    "rusty_tick sent an empty continuing page".into(),
                ));
            }
            records.extend(batch);
            match next {
                Some(next) => after = Some(next),
                None => return Ok(records),
            }
        }
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
        let docs: Vec<Doc> = self.collection("/docs/board", "docs")?;
        Ok(docs
            .into_iter()
            .filter_map(|d| {
                let body = serde_json::from_value(d.body).ok()?;
                Some((d.id, body))
            })
            .collect())
    }

    fn comments(&self, task: Option<Uuid>) -> Result<BTreeMap<Uuid, Vec<Comment>>, BoardError> {
        let path = task.map_or_else(
            || "/docs/comment".to_owned(),
            |id| format!("/docs/comment?taskId={id}"),
        );
        let docs: Vec<Doc> = self.collection(&path, "docs")?;
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

    /// Reserve the first free number of `RESERVE_ATTEMPTS` from `first`; `None` if another
    /// board holds them all.
    fn reserve_from(&self, card: Uuid, first: u64) -> Result<Option<u64>, BoardError> {
        for n in first..first + RESERVE_ATTEMPTS {
            let mut request =
                TickRequest::new("PUT", format!("/docs/{NUMBER_KIND}/{}", number_doc(n)))
                    .body(json!({ "v": 1, "number": n, "taskId": card }));
            request.if_absent = true;
            match self.tick.send(&request)?.0 {
                200 => return Ok(Some(n)),
                412 => continue,
                status => {
                    return Err(BoardError::Storage(format!(
                        "rusty_tick answered {status} reserving card number {n}"
                    )));
                }
            }
        }
        Ok(None)
    }

    /// Every card on the board in list order; with `whole`, also their board documents and
    /// comments, otherwise the tasks alone.
    fn board(&self, whole: bool) -> Result<Vec<Card>, BoardError> {
        let tasks = self.tagged()?;
        let projects = self.projects()?;
        let (mut docs, mut comments) = if whole {
            (self.board_docs()?, self.comments(None)?)
        } else {
            (BTreeMap::new(), BTreeMap::new())
        };
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

    fn tagged(&self) -> Result<Vec<Task>, BoardError> {
        let tasks: Vec<Task> = self.collection(&format!("/tags/{AGENT_TAG}/tasks"), "tasks")?;
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
            let (status, body) = self
                .tick
                .send(&TickRequest::new("GET", format!("/docs/board/{id}")))?;
            let doc = match status {
                200 => {
                    let doc: Doc = serde_json::from_value(body).map_err(|e| {
                        BoardError::Storage(format!("rusty_tick sent a bad document: {e}"))
                    })?;
                    Some(serde_json::from_value(doc.body).map_err(|e| {
                        BoardError::Storage(format!("rusty_tick sent a bad board document: {e}"))
                    })?)
                }
                404 => None,
                _ => {
                    return Err(BoardError::Storage(format!(
                        "rusty_tick answered {status} reading a board document"
                    )));
                }
            };
            let comments = self.comments(Some(id))?.remove(&id).unwrap_or_default();
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
        if next.column == Column::DONE && current.column != Column::DONE {
            patch.insert("status".into(), json!(closed_status(next)));
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
    fn put_doc(&self, card: &Card, body: Value, version: u64) -> Result<u64, BoardError> {
        let mut request = TickRequest::new("PUT", format!("/docs/board/{}", card.id)).body(body);
        request.if_task_match = Some(version.to_string());
        self.call(request, 200)?;
        Ok(version + 1)
    }

    /// Append `comment` if the task is still at `version`, moving it as `put_doc` does.
    fn put_comment(&self, card: &Card, comment: &Comment, version: u64) -> Result<u64, BoardError> {
        let id = Uuid::now_v7();
        // This write is logged as the card's own event, so observing must not log it again.
        self.events
            .lock()
            .map_err(poisoned)?
            .comments_seen
            .insert(id);
        let mut request = TickRequest::new("PUT", format!("/docs/comment/{id}")).body(json!({
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
    fn finish_write(
        &self,
        mut event: Event,
        result: Result<u64, BoardError>,
    ) -> Result<u64, BoardError> {
        let mut events = self.events.lock().map_err(poisoned)?;
        let count = events
            .writing
            .get_mut(&event.card)
            .expect("a finished write was marked");
        *count -= 1;
        if *count == 0 {
            events.writing.remove(&event.card);
        }
        events.write_epoch = events.write_epoch.wrapping_add(1);
        let version = result?;
        let rev = version.saturating_sub(1);
        event.rev = rev;
        // Same-card mutations are serialized by rusty_tick, but their callers can
        // return here in reverse order. Never forget the newer mutation in that case.
        events
            .seen
            .entry(event.card)
            .and_modify(|seen| *seen = (*seen).max(version))
            .or_insert(version);
        events.push(event);
        Ok(rev)
    }

    /// Mark a write before its first task mutation. The mark is removed by
    /// `finish_write`, including when rusty_tick refuses or fails the write.
    fn start_write(&self, card: Uuid) -> Result<(), BoardError> {
        let mut events = self.events.lock().map_err(poisoned)?;
        *events.writing.entry(card).or_default() += 1;
        events.write_epoch = events.write_epoch.wrapping_add(1);
        Ok(())
    }

    /// Note tasks changed or newly tagged in rusty_tick since they were last seen, and
    /// comments added there. Reads only what changed since the last observation (tasks by
    /// `updatedMs`, comments by id), and every card each `reconcile_every` to catch anything
    /// a delta read missed.
    fn observe(&self) -> Result<(), BoardError> {
        // If a write starts or finishes while the task snapshot is fetched,
        // fetch it again. Holding this lock across I/O would let a busy
        // observer starve writers that need it to commit their event.
        let (tasks, comments, full, mut events) = loop {
            let now = now_unix_ms();
            let (epoch, full, high_water, comments_from) = {
                let e = self.events.lock().map_err(poisoned)?;
                let full = e
                    .reconciled
                    .is_none_or(|at| at.elapsed() >= self.reconcile_every);
                (
                    e.write_epoch,
                    full,
                    e.high_water,
                    comment_window(e.comments_read_ms.unwrap_or(now)),
                )
            };
            let tasks = if full {
                self.tagged()?
            } else {
                self.changed_since(high_water)?
            };
            let comments: Vec<Doc> =
                self.collection_after("/docs/comment", "docs", Some(comments_from))?;
            let mut events = self.events.lock().map_err(poisoned)?;
            if events.write_epoch == epoch {
                events.comments_read_ms = Some(now);
                break (tasks, comments, full, events);
            }
        };
        if full {
            events.reconciled = Some(Instant::now());
        }
        let primed = std::mem::replace(&mut events.primed, true);
        let mut high_water = events.high_water;
        // A task skipped because a board write to it is in flight is read again next poll:
        // an outside edit racing that write must not wait for the next full read.
        let mut in_flight = i64::MAX;
        for task in tasks {
            high_water = high_water.max(task.updated_ms);
            let version = task.version()?;
            if events.writing.contains_key(&task.id) {
                in_flight = in_flight.min(task.updated_ms);
                continue;
            }
            if !task.on_board() {
                continue;
            }
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
        events.high_water = high_water.min(in_flight);
        let mut commented = Vec::new();
        for doc in comments {
            if !events.comments_seen.insert(doc.id) || !primed {
                continue;
            }
            let task = doc.body["taskId"]
                .as_str()
                .and_then(|t| Uuid::parse_str(t).ok());
            commented.extend(task.map(|t| (doc.id, t)));
        }
        drop(events);
        self.log_outside_comments(&commented)
    }

    /// Log a `Commented` event for each `(comment, task)` added in rusty_tick, reading the
    /// task as it is now. A comment on a task that is not on the board logs nothing. If a
    /// read fails, the comments not yet logged are unmarked, so the next poll logs them.
    fn log_outside_comments(&self, comments: &[(Uuid, Uuid)]) -> Result<(), BoardError> {
        for (i, (_, id)) in comments.iter().enumerate() {
            let task = match self.task(*id) {
                Ok(task) => task,
                Err(e) => {
                    let mut events = self.events.lock().map_err(poisoned)?;
                    for (comment, _) in &comments[i..] {
                        events.comments_seen.remove(comment);
                    }
                    return Err(e);
                }
            };
            let Some(task) = task.filter(Task::on_board) else {
                continue;
            };
            let card = card_of(Parts {
                task,
                project: String::new(),
                doc: None,
                comments: Vec::new(),
            })?;
            self.events.lock().map_err(poisoned)?.push(Event::of(
                &card,
                EventKind::Commented,
                TICK_ACTOR,
            ));
        }
        Ok(())
    }

    /// Every task changed at or after `from` (Unix ms), on the board or not: one search
    /// whose answer is the size of the change, not of the board.
    fn changed_since(&self, from: i64) -> Result<Vec<Task>, BoardError> {
        let body = self.call(
            TickRequest::new("GET", format!("/search?updatedFrom={from}")),
            200,
        )?;
        Self::decode(body, "tasks")
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
    fn durable(&self) -> bool {
        true
    }

    fn get(&self, id: Uuid) -> Result<Option<Card>, BoardError> {
        self.parts(id)?.map(card_of).transpose()
    }

    fn list(&self) -> Result<Vec<Card>, BoardError> {
        self.board(true)
    }

    /// The tasks alone: one read of the project lists and the tag's pages, never the board
    /// documents or comments, so prompts, links and comments are left empty.
    fn summaries(&self) -> Result<Vec<Card>, BoardError> {
        self.board(false)
    }

    /// Through the card's `number:<n>` tag: one tag read and one read per match, whatever
    /// the size of the board.
    fn numbered(&self, number: u64) -> Result<Vec<Card>, BoardError> {
        let tasks: Vec<Task> = self.collection(&format!("/tags/number:{number}/tasks"), "tasks")?;
        let mut cards = Vec::new();
        for task in tasks.iter().filter(|t| t.on_board()) {
            // The task may have left the board, or changed number, since the tag read.
            if let Some(card) = self.get(task.id)?.filter(|c| c.number == Some(number)) {
                cards.push(card);
            }
        }
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
        if card.column == Column::DONE {
            body["status"] = json!(closed_status(card));
        }
        let doc = match BoardDoc::of(card) == BoardDoc::default() {
            true => None,
            false => Some(BoardDoc::of(card).checked()?),
        };
        self.start_write(card.id)?;
        let result = (|| {
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
            match doc {
                None => Ok(1),
                Some(doc) => self.put_doc(card, doc, 1),
            }
        })();
        self.finish_write(event, result)
    }

    fn replace(&self, card: &Card, event: Event) -> Result<u64, BoardError> {
        check_claim(card)?;
        // The task version the board read the card at (`rev = version - 1`, and the
        // next revision is one more).
        let read = card.rev;
        let parts = self.parts(card.id)?.ok_or(BoardError::NotFound(card.id))?;
        let stored = parts.doc.clone();
        let current = card_of(parts)?;
        // Every write moves the version, so an unchanged version means the task and its
        // board document are as the board read them; each request below is fenced on it.
        if current.rev + 1 != read {
            return Err(BoardError::Conflict);
        }
        let doc = match BoardDoc::of(card) != BoardDoc::of(&current) {
            true => Some(BoardDoc::kept(card, stored.as_ref()).checked()?),
            false => None,
        };
        self.start_write(card.id)?;
        let result = (|| {
            if event.kind == EventKind::Commented {
                // An explicit append: comments others add meanwhile do not move the version
                // and are kept.
                let comment = card.comments.last().ok_or_else(|| {
                    BoardError::Storage("a comment event carries no comment".into())
                })?;
                self.put_comment(card, comment, read)
            } else {
                let patch = self.task_patch(&current, card)?;
                let task_changed = patch.as_object().is_some_and(|p| !p.is_empty());
                let mut version = read;
                // An empty patch still moves the version, so every write is a compare-and-swap.
                if task_changed || doc.is_none() {
                    version = self.patch_task(card.id, patch, version)?;
                }
                // Both change for `card_plan` with priority and links, and for an edit of the
                // prompt with title, body or tags: two fenced requests. The document is checked
                // above, so only a lost connection or a writer between them can split the
                // write. That is reported as `PartlySaved` with the revision the task write
                // left, never as a refused change; the same change on that revision finishes it.
                if let Some(doc) = doc {
                    let task_written = version != read;
                    version = self
                        .put_doc(card, doc, version)
                        .map_err(|e| match task_written {
                            true => BoardError::PartlySaved {
                                rev: version - 1,
                                why: e.to_string(),
                            },
                            false => e,
                        })?;
                }
                Ok(version)
            }
        })();
        self.finish_write(event, result)
    }

    fn events_since(&self, after: Option<Uuid>, limit: usize) -> Result<Vec<Event>, BoardError> {
        self.observe()?;
        let events = self.events.lock().map_err(poisoned)?;
        let start = after.map_or(0, |a| events.log.partition_point(|e| e.id <= a));
        Ok(events.log[start..].iter().take(limit).cloned().collect())
    }

    fn holds_event(&self, id: Uuid) -> Result<bool, BoardError> {
        let events = self.events.lock().map_err(poisoned)?;
        Ok(events.log.binary_search_by_key(&id, |e| e.id).is_ok())
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

    /// From rusty_tick's move history (`GET /moves`), which outlives restarts and purges,
    /// paged from `span.from`.
    fn moved_in(&self, span: Span) -> Result<BTreeSet<Uuid>, BoardError> {
        let since = span.from.unwrap_or(0);
        let mut after = 0u64;
        let mut moved = BTreeSet::new();
        loop {
            let page = self.call(
                TickRequest::new(
                    "GET",
                    format!("/moves?since={since}&after={after}&limit={MOVES_PAGE}"),
                ),
                200,
            )?;
            let moves: Vec<MoveRecord> = Self::decode(page.clone(), "moves")?;
            moved.extend(
                moves
                    .iter()
                    .filter(|m| span.contains(m.at_ms))
                    .map(|m| m.task_id),
            );
            match page["next"].as_u64() {
                Some(next) if next > after => after = next,
                _ => return Ok(moved),
            }
        }
    }

    /// Reservations are `board_number` documents, created with `If-None-Match: *` and kept
    /// whatever happens to the task, so a number never comes back. Tries the numbers above
    /// `floor` in turn, one write each; if another board has taken all of a run of them, it
    /// reads every reservation once and goes on above the highest.
    fn reserve_number(&self, card: Uuid, floor: u64) -> Result<u64, BoardError> {
        if let Some(n) = self.reserve_from(card, floor + 1)? {
            return Ok(n);
        }
        let first = self.highest_reserved()?.max(floor + RESERVE_ATTEMPTS) + 1;
        self.reserve_from(card, first)?.ok_or(BoardError::Conflict)
    }

    fn highest_reserved(&self) -> Result<u64, BoardError> {
        let docs: Vec<Doc> = self.collection(&format!("/docs/{NUMBER_KIND}"), "docs")?;
        Ok(docs
            .iter()
            .filter_map(|d| d.body["number"].as_u64())
            .max()
            .unwrap_or(0))
    }

    /// The tasks tagged with each column, never a board document or comment: one paged tag
    /// read per column and one of the project lists.
    fn in_columns(&self, columns: &[Column]) -> Result<Vec<Card>, BoardError> {
        let projects = self.projects()?;
        let mut cards: BTreeMap<Uuid, Card> = BTreeMap::new();
        for column in columns {
            let tasks: Vec<Task> =
                self.collection(&format!("/tags/board:{column}/tasks"), "tasks")?;
            for task in tasks.into_iter().filter(Task::on_board) {
                let project = projects.of(&task);
                let card = card_of(Parts {
                    task,
                    project,
                    doc: None,
                    comments: Vec::new(),
                })?;
                // A task tagged with two columns is in the first one only, as everywhere.
                if columns.contains(&card.column) {
                    cards.insert(card.id, card);
                }
            }
        }
        let mut cards: Vec<Card> = cards.into_values().collect();
        cards.sort_by_key(|c| (c.created_ms, c.id));
        Ok(cards)
    }

    /// One task read per id.
    fn open_among(&self, ids: &[Uuid]) -> Result<BTreeSet<Uuid>, BoardError> {
        let mut open = BTreeSet::new();
        for id in ids {
            if self
                .task(*id)?
                .is_some_and(|t| t.on_board() && column_of(&t) != Column::DONE)
            {
                open.insert(*id);
            }
        }
        Ok(open)
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

/// Milestones in rusty_tick: tasks tagged `milestone` in the project's list, `dueMs` the target,
/// closed when done.
impl<T: TickApi> TickStore<T> {
    fn milestone_of(projects: &Projects, task: Task) -> Option<Milestone> {
        let target_ms = task.due_ms?;
        let rev = task.version().ok()?;
        Some(Milestone {
            rev,
            id: task.id,
            project: projects.of(&task),
            done_ms: task.completed_ms.filter(|_| task.status == "done"),
            title: task.title,
            target_ms,
        })
    }
}

impl<T: TickApi> MilestoneStore for TickStore<T> {
    fn milestones(&self) -> Result<Vec<Milestone>, BoardError> {
        let tasks: Vec<Task> = self.collection(&format!("/tags/{MILESTONE_TAG}/tasks"), "tasks")?;
        let projects = self.projects()?;
        Ok(tasks
            .into_iter()
            .filter(Task::is_milestone)
            .filter_map(|t| Self::milestone_of(&projects, t))
            .collect())
    }

    fn milestone(&self, id: Uuid) -> Result<Option<Milestone>, BoardError> {
        let Some(task) = self.task(id)?.filter(Task::is_milestone) else {
            return Ok(None);
        };
        Ok(Self::milestone_of(&self.projects()?, task))
    }

    fn add_milestone(&self, milestone: &Milestone) -> Result<u64, BoardError> {
        let body = json!({
            "id": milestone.id,
            "listId": self.list_for(&milestone.project)?,
            "title": milestone.title,
            "tags": [MILESTONE_TAG],
            "dueMs": milestone.target_ms,
        });
        let (status, created) = self
            .tick
            .send(&TickRequest::new("POST", "/tasks").body(body))?;
        match status {
            201 => serde_json::from_value::<Task>(created)
                .map_err(|e| BoardError::Storage(format!("rusty_tick sent a bad task: {e}")))?
                .version(),
            409 => Err(BoardError::Conflict),
            _ => Err(BoardError::Storage(format!(
                "rusty_tick answered {status} creating a milestone"
            ))),
        }
    }

    fn save_milestone(&self, current: &Milestone, next: &Milestone) -> Result<u64, BoardError> {
        let mut patch = serde_json::Map::new();
        if next.title != current.title {
            patch.insert("title".into(), json!(next.title));
        }
        if next.target_ms != current.target_ms {
            patch.insert("dueMs".into(), json!(next.target_ms));
        }
        if next.done_ms.is_some() != current.done_ms.is_some() {
            let status = if next.done_ms.is_some() {
                "done"
            } else {
                "open"
            };
            patch.insert("status".into(), json!(status));
        }
        if patch.is_empty() {
            return Ok(current.rev);
        }
        // Fenced on the version the caller's scope check read: if the task moved meanwhile (to
        // another project's list, say), rusty_tick refuses with 412 and the edit must start over.
        self.patch_task(current.id, Value::Object(patch), current.rev)
    }
}
