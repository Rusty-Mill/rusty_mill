//! The operation table. MCP tools and the web API both dispatch through it,
//! so the two surfaces cannot drift: one name, one argument type, one schema.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::board::{Board, NewCard};
use crate::domain::{
    Actor, BoardError, Card, CardSummary, CloseReason, Column, DEFAULT_PROJECT, DONE_WINDOW_DAYS,
    Dates, Edit, ExecutionLink, Plan, ProjectStatus, Shelf, Span, View,
};
use crate::external::{Observation, Provider, RefState};
use crate::milestones::MilestoneEdit;
use crate::page::{Cursor, Key, checked_limit, page, query_hash};
use crate::store::Store;

/// The board every surface shares.
pub type Shared = Board<Box<dyn Store>>;

/// Who a credential belongs to. Set by the transport after it checks a bearer
/// token; when present it fixes the caller's actor and role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    /// Stable name, recorded in claims and history.
    pub actor: String,
    /// Role name; see `board_roles`.
    pub role: String,
    /// Projects this credential may work in; `None` is all of them.
    #[serde(default)]
    pub projects: Option<Vec<String>>,
}

/// Why a call failed.
#[derive(Debug)]
pub enum CallError {
    /// The arguments did not parse.
    BadArgs(String),
    /// The board refused or failed.
    Board(BoardError),
}

impl From<BoardError> for CallError {
    fn from(e: BoardError) -> Self {
        Self::Board(e)
    }
}

type RunFn =
    Box<dyn Fn(&Shared, Option<&Identity>, Value) -> Result<Value, CallError> + Send + Sync>;

/// One named operation.
pub struct Op {
    /// Tool name and web path segment.
    pub name: &'static str,
    /// Shown to agents.
    pub description: &'static str,
    /// JSON Schema of the arguments.
    pub schema: Value,
    run: RunFn,
}

impl Op {
    /// Run with raw JSON arguments, as `identity` if the transport
    /// authenticated one.
    ///
    /// # Errors
    /// [`CallError::BadArgs`] if they don't parse, else whatever the board says.
    pub fn call(
        &self,
        board: &Shared,
        identity: Option<&Identity>,
        args: Value,
    ) -> Result<Value, CallError> {
        (self.run)(board, identity, args)
    }
}

fn op<A>(
    name: &'static str,
    description: &'static str,
    f: fn(&Shared, Option<&Identity>, A) -> Result<Value, BoardError>,
) -> Op
where
    A: serde::de::DeserializeOwned + JsonSchema + 'static,
{
    let mut schema = serde_json::to_value(schemars::schema_for!(A)).unwrap_or(Value::Null);
    if let Some(object) = schema.as_object_mut() {
        object.remove("$schema");
        object.remove("title");
    }
    Op {
        name,
        description,
        schema,
        run: Box::new(move |board, identity, args| {
            let args =
                serde_json::from_value(args).map_err(|e| CallError::BadArgs(e.to_string()))?;
            f(board, identity, args).map_err(CallError::Board)
        }),
    }
}

/// Who is acting. With a per-model credential these are fixed by it: omit
/// them (or repeat them exactly). Without one the board trusts what the
/// caller says: a workflow guard between cooperating agents.
#[derive(Deserialize, JsonSchema)]
pub(crate) struct Who {
    /// Stable name of the actor, recorded in claims and history.
    actor: Option<String>,
    /// Role name; see `board_roles`.
    role: Option<String>,
}

/// The acting identity: the credential's if there is one (a disagreeing
/// `actor`/`role` is refused), else the arguments'.
pub(crate) fn acting(
    b: &Shared,
    identity: Option<&Identity>,
    who: &Who,
) -> Result<Actor, BoardError> {
    match identity {
        Some(i) => {
            let differs = who.actor.as_deref().is_some_and(|a| a != i.actor)
                || who.role.as_deref().is_some_and(|r| r != i.role);
            if differs {
                return Err(BoardError::Invalid(
                    "actor and role are fixed by your credential; omit them",
                ));
            }
            let mut actor = b.roles().actor(&i.actor, &i.role)?;
            actor.projects = i.projects.as_ref().map(|p| p.iter().cloned().collect());
            Ok(actor)
        }
        None => {
            let actor = who
                .actor
                .as_deref()
                .ok_or(BoardError::Invalid("actor is required"))?;
            let role = who
                .role
                .as_deref()
                .ok_or(BoardError::Invalid("role is required"))?;
            b.roles().actor(actor, role)
        }
    }
}

/// The actor for an op that only an authenticated admin may run: the inbox ops, which carry
/// BBP's human authority (they lift its gates and budgets), and `board_number_cards`, which
/// writes every project's cards. A caller-named actor and role are not enough: only an
/// identity bound to the transport (a token or OAuth grant) may use them.
fn bound_acting(b: &Shared, identity: Option<&Identity>, who: &Who) -> Result<Actor, BoardError> {
    if identity.is_none() {
        return Err(BoardError::Invalid(
            "this needs an authenticated admin credential; a named actor and role are not enough",
        ));
    }
    acting(b, identity, who)
}

#[derive(Deserialize, JsonSchema)]
struct Nothing {}

/// The arguments of an operation that takes only who is acting.
#[derive(Deserialize, JsonSchema)]
struct WhoArgs {
    #[serde(flatten)]
    who: Who,
}

#[derive(Deserialize, JsonSchema)]
struct InboxActArgs {
    #[serde(flatten)]
    who: Who,
    /// The escalation id from `inbox_list`.
    id: String,
    /// Its `rev` from `inbox_list`; an action against newer work is refused.
    rev: u64,
    /// What you decide.
    #[serde(flatten)]
    action: crate::inbox::Action,
    /// Only show what the first command would write.
    #[serde(default)]
    dry_run: bool,
}

#[derive(Deserialize, JsonSchema)]
struct TakeArgs {
    #[serde(flatten)]
    who: Who,
    /// Take this card (id or number) instead of the next one for your role.
    id: Option<String>,
    /// Only cards of this project (see `board_projects`).
    project: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct ListArgs {
    /// Only this column.
    column: Option<Column>,
    /// Only this project (see `board_projects`).
    project: Option<String>,
    /// Shelved Done cards (archived, or done longer than `done_days` ago): `exclude`
    /// (default), `include`, or `only` (the archive).
    #[serde(default)]
    archived: Shelf,
    /// Days a Done card stays on the board before it is shelved without being archived.
    /// Default 14; 0 shelves only archived cards.
    done_days: Option<u32>,
    /// Only cards created at or after this time (Unix ms).
    created_from: Option<i64>,
    /// Only cards created before this time (Unix ms).
    created_to: Option<i64>,
    /// Only cards last changed at or after this time (Unix ms).
    updated_from: Option<i64>,
    /// Only cards last changed before this time (Unix ms).
    updated_to: Option<i64>,
    /// Only cards that changed column at or after this time (Unix ms); with `moved_to`, in
    /// that range. Entering the board counts as a move.
    moved_from: Option<i64>,
    /// Only cards that changed column before this time (Unix ms).
    moved_to: Option<i64>,
    /// `summary` (default): id, number, rev, title, project, column, priority, claim, tags
    /// and times only, without body, prompt, links or comments; much smaller, and read
    /// without loading any card's prompt or comments. `full`: whole cards, as `card_get`
    /// returns them.
    #[serde(default)]
    view: View,
    /// Only cards of projects with these statuses (`active`, `paused`, `experiment`,
    /// `archived`). Default: active projects only, unless `project` names one.
    project_status: Option<Vec<ProjectStatus>>,
    /// Page size, 1 to 500. With it (or `cursor`) the result also has `next_cursor` (null
    /// on the last page) and `as_of_ms`. A page also stops at 512 KiB of cards; one card always
    /// fits. Without it every matching card comes back at once, as before.
    limit: Option<u32>,
    /// `next_cursor` from the previous page of the same query (same arguments apart from
    /// `limit`). Pages follow creation order; an edit never moves a card between pages.
    cursor: Option<String>,
}

impl ListArgs {
    /// The query's arguments apart from `limit` and `cursor`, in a fixed order: what a cursor
    /// is bound to.
    fn canonical(&self) -> String {
        format!(
            "column={:?};project={:?};archived={:?};done_days={:?};created={:?},{:?};updated={:?},{:?};moved={:?},{:?};view={:?}",
            self.column.as_ref().map(Column::as_str),
            self.project,
            self.archived,
            self.done_days,
            self.created_from,
            self.created_to,
            self.updated_from,
            self.updated_to,
            self.moved_from,
            self.moved_to,
            self.view,
        ) + &self
            .project_status
            .as_ref()
            .map(|s| {
                // A set, not a sequence: the same filter in any order keeps its cursor.
                let mut names: Vec<&str> = s.iter().map(|x| x.as_str()).collect();
                names.sort_unstable();
                names.dedup();
                format!(";project_status={names:?}")
            })
            .unwrap_or_default()
    }

    /// Whether a card of a project with `status` is listed.
    fn keeps(&self, status: ProjectStatus) -> bool {
        match (&self.project_status, &self.project) {
            (Some(wanted), _) => wanted.contains(&status),
            (None, Some(_)) => true,
            (None, None) => status.accepts_cards(),
        }
    }

    fn dates(&self) -> Dates {
        let span = |from, to| Span { from, to };
        Dates {
            created: span(self.created_from, self.created_to),
            updated: span(self.updated_from, self.updated_to),
            moved: span(self.moved_from, self.moved_to),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
struct ArchiveDoneArgs {
    #[serde(flatten)]
    who: Who,
    /// Only this project (see `board_projects`); default every project you may work in.
    project: Option<String>,
    /// Only cards done at least this many days ago. Default 0: every Done card.
    #[serde(default)]
    older_than_days: u32,
}

#[derive(Deserialize, JsonSchema)]
struct RoadmapArgs {
    /// The project (see `board_projects`).
    project: String,
}

#[derive(Deserialize, JsonSchema)]
struct MetricsArgs {
    /// Only this project (see `board_projects`).
    project: Option<String>,
    /// Only this repository's projects.
    repo: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct MilestoneAddArgs {
    #[serde(flatten)]
    who: Who,
    /// The project (see `board_projects`).
    project: String,
    /// What is reached, 1 to 200 characters.
    title: String,
    /// When it should be reached, Unix ms.
    target_ms: i64,
}

#[derive(Deserialize, JsonSchema)]
struct MilestoneUpdateArgs {
    #[serde(flatten)]
    who: Who,
    /// The milestone's id.
    id: String,
    /// A new title.
    title: Option<String>,
    /// A new target, Unix ms.
    target_ms: Option<i64>,
    /// `true` closes it (done), `false` reopens it.
    done: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct OwnsArgs {
    /// The project (see `board_projects`).
    project: String,
    /// Repository paths to classify.
    files: Vec<String>,
}

#[derive(Deserialize, JsonSchema)]
struct AssignArgs {
    #[serde(flatten)]
    who: Who,
    /// Card id, or its number (`#42` or `42`).
    id: String,
    /// The project to move it to (see `board_projects`).
    project: String,
}

#[derive(Deserialize, JsonSchema)]
struct ProjectAddArgs {
    #[serde(flatten)]
    who: Who,
    /// Repository the project belongs to (see `board_projects`); a new name adds a repository.
    repo: String,
    /// Project name: 1-40 of a-z 0-9 _ -, unique across repositories.
    name: String,
    /// Repository paths the project's cards may change, e.g. `crates/libs/ui/rusty_term`. None
    /// may overlap another project's path in that repository.
    paths: Vec<String>,
}

#[derive(Deserialize, JsonSchema)]
struct PlanArgs {
    #[serde(flatten)]
    who: Who,
    /// Card id, or its number (`#42` or `42`).
    id: String,
    /// 0 (none), 1 (low), 3 (medium) or 5 (high); higher is taken first.
    priority: Option<u8>,
    /// Ids or numbers of the cards that must be Done before this one is taken; replaces the list.
    deps: Option<Vec<String>>,
    /// Ids or numbers of newer cards that replace this one; replaces the list, `[]` clears it.
    /// `card_take` never hands out a superseded card.
    superseded_by: Option<Vec<String>>,
    /// Ids or numbers of cards worth reading alongside this one; replaces the list.
    related: Option<Vec<String>>,
    /// Id or number of the card this one was split from; `null` clears it. One parent per card,
    /// never a loop. Left out, it is unchanged.
    #[serde(default, deserialize_with = "present")]
    parent: Option<Option<String>>,
    /// Id of the milestone (from `board_milestones`, same project) this card counts towards;
    /// `null` clears it. Left out, it is unchanged.
    #[serde(default, deserialize_with = "present")]
    milestone: Option<Option<String>>,
    /// When the card should be done, Unix ms; `null` clears it. Left out, it is unchanged.
    #[serde(default, deserialize_with = "present")]
    target_ms: Option<Option<i64>>,
}

/// A field that is present, even as `null`: with `#[serde(default)]`, absent stays `None`.
fn present<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

#[derive(Deserialize, JsonSchema)]
struct GetArgs {
    /// Card id, or its number (`#42` or `42`).
    id: String,
}

#[derive(Deserialize, JsonSchema)]
struct AddArgs {
    #[serde(flatten)]
    who: Who,
    /// Short title.
    title: String,
    /// Description (markdown).
    body: Option<String>,
    /// Labels.
    tags: Option<Vec<String>>,
    /// The project the card belongs to (default: `default`; see `board_projects`).
    project: Option<String>,
    /// Create it straight in Done, to record work that already merged (needs `close`).
    done: Option<bool>,
    /// Id or number of the card this one is split from. Set it whenever you split work into
    /// pieces, so the decomposition is recorded rather than implied by titles.
    parent: Option<String>,
    /// Evidence that the work shipped (a merged PR or other http(s) link), at most 8. A Done
    /// card counts as shipped only with evidence; without, metrics report it as a prototype.
    evidence: Option<Vec<String>>,
}

#[derive(Deserialize, JsonSchema)]
struct EvidenceArgs {
    #[serde(flatten)]
    who: Who,
    /// Card id, or its number (`#42` or `42`).
    id: String,
    /// Links to add: a merged PR, or any http(s) page that shows the work shipped.
    links: Vec<String>,
}

#[derive(Deserialize, JsonSchema)]
struct MoveArgs {
    #[serde(flatten)]
    who: Who,
    /// Card id, or its number (`#42` or `42`).
    id: String,
    /// Destination column.
    to: Column,
}

#[derive(Deserialize, JsonSchema)]
struct CloseArgs {
    #[serde(flatten)]
    who: Who,
    /// Card id, or its number (`#42` or `42`).
    id: String,
    /// Why it is closed without shipping: `wont_do`, `duplicate` or `superseded`.
    reason: CloseReason,
    /// Optional context, added as your comment: the surviving card, the decision, and so on.
    note: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct IdArgs {
    #[serde(flatten)]
    who: Who,
    /// Card id, or its number (`#42` or `42`).
    id: String,
}

#[derive(Deserialize, JsonSchema)]
struct UpdateArgs {
    #[serde(flatten)]
    who: Who,
    /// Card id, or its number (`#42` or `42`).
    id: String,
    /// New title.
    title: Option<String>,
    /// New description.
    body: Option<String>,
    /// New labels (replaces all).
    tags: Option<Vec<String>>,
    /// New prompt: instructions for whoever takes the card, kept apart from the body
    /// (`""` clears it; at most 32 KiB).
    prompt: Option<String>,
    /// The card revision you last saw. If the card has moved on, the update is refused (409)
    /// so you can look at what changed instead of overwriting it.
    rev: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
struct CommentArgs {
    #[serde(flatten)]
    who: Who,
    /// Card id, or its number (`#42` or `42`).
    id: String,
    /// Comment text.
    text: String,
}

#[derive(Deserialize, JsonSchema)]
struct BindExecutionArgs {
    #[serde(flatten)]
    who: Who,
    /// Card id, or its number (`#42` or `42`).
    id: String,
    /// Stable logical execution id; repeat it to adopt after a restart.
    execution: String,
    /// Orch goal id.
    goal: String,
    /// BBP task id.
    bbp_task: String,
    /// Task-host attempt id.
    attempt: String,
    /// Configured provider-neutral orchestrator assignment.
    assignment: String,
    /// Assignment generation used to fence stale host commands.
    generation: u64,
}

#[derive(Deserialize, JsonSchema)]
struct RecoverExecutionArgs {
    #[serde(flatten)]
    link: BindExecutionArgs,
    /// The generation the stored link must still carry (compare-and-set).
    expected_generation: u64,
}

impl BindExecutionArgs {
    fn link(self) -> (Who, String, ExecutionLink) {
        let link = ExecutionLink {
            execution: self.execution,
            goal: self.goal,
            bbp_task: self.bbp_task,
            attempt: self.attempt,
            assignment: self.assignment,
            generation: self.generation,
        };
        (self.who, self.id, link)
    }
}

#[derive(Deserialize, JsonSchema)]
struct ObserveArgs {
    #[serde(flatten)]
    who: Who,
    /// Card id, or its number (`#42` or `42`).
    id: String,
    /// Where it ran: `codex-cloud`, `github-pr` or `github-actions`. Orch execution is
    /// `card_execution_link`, not this.
    provider: Provider,
    /// The provider's own id: a Codex Cloud thread id, `owner/repo#123`, an Actions run id.
    /// 1-128 characters of `A-Z a-z 0-9 . _ : / # -`.
    #[serde(rename = "ref")]
    reference: String,
    /// The state you saw.
    state: RefState,
    /// An `https://` link to it (at most 512 bytes). Omit to keep the one already recorded.
    url: Option<String>,
    /// When you saw that state, Unix ms. At most 5 minutes ahead of the board's clock.
    observed_at_ms: i64,
}

impl ObserveArgs {
    fn observation(self) -> (Who, String, Observation) {
        let obs = Observation {
            provider: self.provider,
            reference: self.reference,
            state: self.state,
            url: self.url,
            observed_at_ms: self.observed_at_ms,
        };
        (self.who, self.id, obs)
    }
}

#[derive(Deserialize, JsonSchema)]
struct EventsArgs {
    /// Return events after this event id (omit for the beginning).
    after: Option<String>,
    /// At most this many (default 100).
    limit: Option<usize>,
    /// Return no events, only the `cursor` at the end of the log right now. Start
    /// a watcher with this, then read the board: anything that changes in between
    /// is delivered by the next poll.
    tail: Option<bool>,
}

fn id(raw: &str) -> Result<Uuid, BoardError> {
    Uuid::parse_str(raw).map_err(|_| BoardError::Invalid("id is not a UUID"))
}

/// A card named by its id, or by its number (`#42` or `42`).
fn card_ref(b: &Shared, raw: &str) -> Result<Uuid, BoardError> {
    let raw = raw.trim();
    let Ok(number) = raw.strip_prefix('#').unwrap_or(raw).parse::<u64>() else {
        return Uuid::parse_str(raw)
            .map_err(|_| BoardError::Invalid("id is neither a UUID nor a card number"));
    };
    Ok(b.get_number(number)?.id)
}

/// The projects a credential is limited to; `None` when unrestricted or unauthenticated.
fn scope(identity: Option<&Identity>) -> Option<&[String]> {
    identity.and_then(|i| i.projects.as_deref())
}

fn in_scope(scope: Option<&[String]>, project: &str) -> bool {
    scope.is_none_or(|s| s.iter().any(|p| p == project))
}

/// `project` as a known project the caller may see, or an error.
fn checked_project(
    b: &Shared,
    scope: Option<&[String]>,
    project: Option<&str>,
) -> Result<(), BoardError> {
    let Some(p) = project else { return Ok(()) };
    if !b.projects().contains(p) {
        return Err(BoardError::Invalid("unknown project"));
    }
    if !in_scope(scope, p) {
        return Err(BoardError::OutOfScope(p.to_owned()));
    }
    Ok(())
}

fn card(c: Card) -> Result<Value, BoardError> {
    serde_json::to_value(json!({ "card": c })).map_err(|e| BoardError::Storage(e.to_string()))
}

/// `board_list`'s result: every card, or a page of them, as `view` asks.
fn list_result(b: &Shared, a: &ListArgs, cards: Vec<Card>) -> Result<Value, BoardError> {
    let as_of_ms = b.now_ms();
    let unpaged = a.limit.is_none() && a.cursor.is_none();
    if unpaged {
        return match a.view {
            View::Full => to_json(json!({ "cards": cards })),
            View::Summary => {
                let cards: Vec<CardSummary> = cards.iter().map(CardSummary::from).collect();
                to_json(json!({ "cards": cards }))
            }
        };
    }
    let limit = checked_limit(a.limit.unwrap_or(100))?;
    let query = query_hash(&a.canonical());
    let after = a
        .cursor
        .as_deref()
        .map(|c| Cursor::decode(c, query))
        .transpose()?
        .map(|c| c.after);
    match a.view {
        View::Full => paged(
            cards,
            |c| (c.created_ms, c.id),
            after,
            limit,
            query,
            as_of_ms,
        ),
        View::Summary => {
            let cards: Vec<CardSummary> = cards.iter().map(CardSummary::from).collect();
            paged(
                cards,
                |c| (c.created_ms, c.id),
                after,
                limit,
                query,
                as_of_ms,
            )
        }
    }
}

/// The page of `items` after `after`, as `board_list` returns it. Only the page's items are
/// serialized, so a full-view page costs its own size, not the board's.
fn paged<T: serde::Serialize>(
    items: Vec<T>,
    key: impl Fn(&T) -> Key,
    after: Option<Key>,
    limit: usize,
    query: u64,
    as_of_ms: i64,
) -> Result<Value, BoardError> {
    let p = page(items, key, after, limit, query)?;
    to_json(json!({
        "cards": p.items,
        "next_cursor": p.next.map(|c| c.encode()),
        "as_of_ms": as_of_ms,
    }))
}

fn to_json(v: impl serde::Serialize) -> Result<Value, BoardError> {
    serde_json::to_value(v).map_err(|e| BoardError::Storage(e.to_string()))
}

/// Every operation, in display order.
pub fn ops() -> Vec<Op> {
    vec![
        op(
            "board_list",
            "List cards, optionally one column (an id from `board_columns`) and/or one project. A credential limited to some projects only sees those. Done cards that are archived, or done more than `done_days` (default 14) ago, are left out unless `archived` is `include` or `only`. `created_*`, `updated_*` and `moved_*` (Unix ms, `from` inclusive, `to` exclusive) keep cards created, last changed or moved column in a range. By default it returns small summaries without body, prompt, links or comments; pass `view: full` for whole cards, or `card_get` one card. `limit` pages the result in creation order: pass the page's `next_cursor` as `cursor` with the same arguments until it is null.",
            |b, i, a: ListArgs| {
                checked_project(b, scope(i), a.project.as_deref())?;
                let days = a.done_days.unwrap_or(DONE_WINDOW_DAYS);
                let mut cards = b.list_view(a.column.as_ref(), a.archived, days, a.view)?;
                // Scope and filters first: pages, and cursors, only ever span visible cards.
                let statuses = b.statuses();
                cards.retain(|c| {
                    in_scope(scope(i), &c.project)
                        && a.project.as_deref().is_none_or(|p| c.project == p)
                        && a.keeps(statuses.of(&c.project))
                });
                b.retain_dates(&mut cards, &a.dates())?;
                list_result(b, &a, cards)
            },
        ),
        op(
            "board_projects",
            "List the repositories this board serves, their projects (applications) and the repository paths each project may change: {repos: {repo: {project: [paths]}}}. Cards belong to one project; a repository's own project (named like it) owns every path no other project of that repository does.",
            |b, i, _: Nothing| {
                // A limited credential is told only about the projects it may work in.
                let mut repos = serde_json::to_value(b.projects())
                    .map_err(|e| BoardError::Storage(e.to_string()))?;
                if let (Some(scope), Some(map)) = (scope(i), repos.as_object_mut()) {
                    map.values_mut().for_each(|projects| {
                        if let Some(p) = projects.as_object_mut() {
                            p.retain(|name, _| scope.iter().any(|s| s == name));
                        }
                    });
                    map.retain(|_, projects| projects.as_object().is_some_and(|p| !p.is_empty()));
                }
                let statuses = b.statuses();
                let mut status = serde_json::Map::new();
                for name in b.projects().names().filter(|n| in_scope(scope(i), n)) {
                    let s = statuses.of(name);
                    if s != ProjectStatus::Active {
                        status.insert(name.to_owned(), json!(s));
                    }
                }
                to_json(json!({
                    "repos": repos,
                    "status": status,
                    "default": DEFAULT_PROJECT,
                    "count": b.projects().names().count(),
                }))
            },
        ),
        op(
            "board_roadmap",
            "A project's roadmap, derived from its cards: lanes now (work columns), next (queues), later (backlog), blocked, and shipped (done in the last 30 days, newest first). Each lane is ordered as card_take orders work; `waits_on` lists dependencies not yet done. {project, lanes: [{lane, cards}]}.",
            |b, i, a: RoadmapArgs| {
                checked_project(b, scope(i), Some(&a.project))?;
                to_json(b.roadmap(&a.project)?)
            },
        ),
        op(
            "board_metrics",
            "Board metrics over the cards the caller may see, optionally one project or one repository's projects: open cards by column, project (with shipped in the last 30 days), priority and claimant (work columns); cards added and shipped per week for 8 weeks, oldest first; created-to-shipped lead time (median, p85, ms) over the last 30 days; and the 5 open cards past backlog idle longest. {open, by_column, by_project, by_priority, by_claim, weeks, lead_time, idle}.",
            |b, i, a: MetricsArgs| {
                checked_project(b, scope(i), a.project.as_deref())?;
                let projects = b.projects();
                if let Some(r) = a.repo.as_deref()
                    && !projects.repos().any(|(name, _)| name == r)
                {
                    return Err(BoardError::Invalid("unknown repository"));
                }
                to_json(b.metrics(|c| {
                    in_scope(scope(i), &c.project)
                        && a.project.as_deref().is_none_or(|p| c.project == p)
                        && a.repo
                            .as_deref()
                            .is_none_or(|r| projects.repo_of(&c.project) == Some(r))
                })?)
            },
        ),
        op(
            "board_milestones",
            "Milestones the caller may see, optionally one project or one repository's projects, by target date: {milestones: [{id, project, title, target_ms, done_ms, progress: {total, done, blocked, ready_to_close}}]}. A milestone is a project's dated goal; the roadmap draws it ahead of today. `done_ms` is set once it is closed. `progress` counts the cards linked to it (card_plan `milestone`); `ready_to_close` means all are Done and it is still open: close it with milestone_update.",
            |b, i, a: MetricsArgs| {
                checked_project(b, scope(i), a.project.as_deref())?;
                let projects = b.projects();
                if let Some(r) = a.repo.as_deref()
                    && !projects.repos().any(|(name, _)| name == r)
                {
                    return Err(BoardError::Invalid("unknown repository"));
                }
                let milestones = b.milestones(|m| {
                    in_scope(scope(i), &m.project)
                        && a.project.as_deref().is_none_or(|p| m.project == p)
                        && a.repo
                            .as_deref()
                            .is_none_or(|r| projects.repo_of(&m.project) == Some(r))
                })?;
                to_json(json!({ "milestones": b.milestone_progress(milestones)? }))
            },
        ),
        op(
            "milestone_add",
            "Add a milestone to a project (needs Triage): a title and the date it should be reached (`target_ms`, Unix ms). Milestones are never deleted; close one with milestone_update `done: true`. {milestone}.",
            |b, i, a: MilestoneAddArgs| {
                let who = acting(b, i, &a.who)?;
                checked_project(b, scope(i), Some(&a.project))?;
                to_json(
                    json!({ "milestone": b.add_milestone(&who, &a.project, &a.title, a.target_ms)? }),
                )
            },
        ),
        op(
            "milestone_update",
            "Change a milestone (needs Triage): its `title`, its `target_ms`, or `done` (true closes it, false reopens it). Fields left out are unchanged. {milestone}.",
            |b, i, a: MilestoneUpdateArgs| {
                let who = acting(b, i, &a.who)?;
                let edit = MilestoneEdit {
                    title: a.title,
                    target_ms: a.target_ms,
                    done: a.done,
                };
                let milestone =
                    b.edit_milestone(&who, id(&a.id)?, &edit, |p| in_scope(scope(i), p))?;
                to_json(json!({ "milestone": milestone }))
            },
        ),
        op(
            "board_owns",
            "Which of `files` (repository paths, `/`-separated) the project does NOT own: those belonging to another project of its repository. Answers from the whole registry, so a runner limited to one project can still tell when work strays into a sibling's paths: {foreign: [paths]}.",
            |b, i, a: OwnsArgs| {
                checked_project(b, scope(i), Some(&a.project))?;
                to_json(json!({ "foreign": b.projects().not_owned_by(&a.project, &a.files) }))
            },
        ),
        op(
            "board_roles",
            "List roles and the capabilities each holds.",
            |b, _, _: Nothing| to_json(json!({ "roles": b.roles().0 })),
        ),
        op(
            "board_columns",
            "The board's columns in display order ({id, label, kind}: kind backlog, queue (waits to be taken), work (claimed; leaving needs the claim), done or blocked) and the moves between them ({from, to, cap, gate}: the capability a move needs; a gate also needs admin). Any open card may move to blocked.",
            |b, _, _: Nothing| to_json(json!(b.workflow())),
        ),
        op(
            "board_events",
            "Poll the change log, oldest first. Send the `cursor` of your last reply as `after` to get only newer events (the reply's `cursor` is the last event returned, or your `after` if none). `tail: true` returns just the current end of the log, to start a watcher. `resync: true` means your `after` is not in the log (the board restarted since you read it): changes may have been missed, so re-read the board (`board_list`; its default summaries are cheapest) and go on from the reply's `cursor`.",
            |b, _, a: EventsArgs| {
                if a.tail == Some(true) {
                    return to_json(
                        json!({ "events": [], "cursor": b.cursor()?, "resync": false }),
                    );
                }
                let after = a.after.as_deref().map(id).transpose()?;
                let resync = match after {
                    Some(a) => !b.knows_cursor(a)?,
                    None => false,
                };
                // An unknown cursor says nothing about where to resume: read the log from its start.
                let after = after.filter(|_| !resync);
                let events = b.events(after, a.limit.unwrap_or(100))?;
                let cursor = events.last().map(|e| e.id).or(after);
                to_json(json!({ "events": events, "cursor": cursor, "resync": resync }))
            },
        ),
        op(
            "board_whoami",
            "Who you are on this board: your actor name, role and what that role may do.",
            |b, i, _: Nothing| {
                let Some(i) = i else {
                    return to_json(json!({
                        "authenticated": false,
                        "note": "no per-model credential: pass actor and role on each call"
                    }));
                };
                let who = b.roles().actor(&i.actor, &i.role)?;
                to_json(json!({
                    "authenticated": true,
                    "actor": who.name,
                    "role": i.role,
                    "capabilities": who.caps
                }))
            },
        ),
        op(
            "card_take",
            "Pull your next piece of work: the card nearest Done that your role can act on. A card in Ready is moved to Building, and the card is claimed for you. Returns {card: null} when nothing is waiting. Then do the work, comment with what you did, and card_move it on.",
            |b, i, a: TakeArgs| {
                let who = acting(b, i, &a.who)?;
                let id = a.id.as_deref().map(|r| card_ref(b, r)).transpose()?;
                to_json(json!({ "card": b.take_in(&who, id, a.project.as_deref())? }))
            },
        ),
        op(
            "card_get",
            "Fetch one card with its comments.",
            |b, i, a: GetArgs| {
                let c = b.get(card_ref(b, &a.id)?)?;
                if !in_scope(scope(i), &c.project) {
                    return Err(BoardError::OutOfScope(c.project));
                }
                card(c)
            },
        ),
        op(
            "card_trace",
            "A card's place in its decomposition (parent links): `ancestors`, its chain of parents, root first, and `tree`, the card and every card split from it, each {id, number, title, column, project, rollup: {total, done, blocked, ready_to_close}, children}. A rollup counts direct children; `ready_to_close` means all are Done and the card is still open: close it, it never closes itself. Cards outside your scope are left out but still counted.",
            |b, i, a: GetArgs| {
                let c = b.get(card_ref(b, &a.id)?)?;
                if !in_scope(scope(i), &c.project) {
                    return Err(BoardError::OutOfScope(c.project));
                }
                to_json(b.trace(c.id, |k| in_scope(scope(i), &k.project))?)
            },
        ),
        op(
            "card_add",
            "Create a card in Backlog, or with `done: true` straight in Done to record work that already merged (needs `close`; pass the merged PR as `evidence`, or it counts as a prototype). When splitting work into pieces, pass `parent` (the card it is split from) on each piece.",
            |b, i, a: AddArgs| {
                let who = acting(b, i, &a.who)?;
                let new = NewCard {
                    done: a.done == Some(true),
                    parent: a.parent.as_deref().map(|r| card_ref(b, r)).transpose()?,
                    evidence: a.evidence.unwrap_or_default(),
                    ..NewCard::new(
                        &a.title,
                        a.body.as_deref().unwrap_or(""),
                        a.tags.unwrap_or_default(),
                    )
                };
                card(b.add_card(&who, a.project.as_deref().unwrap_or(DEFAULT_PROJECT), new)?)
            },
        ),
        op(
            "card_execution_link",
            "Initiate or adopt this card's one recoverable Orch execution. A different logical execution is refused; this persists the Orch goal, BBP task, host attempt, assignment and generation with the card.",
            |b, i, a: BindExecutionArgs| {
                let (who, card_id, link) = a.link();
                let who = acting(b, i, &who)?;
                card(b.bind_execution(card_ref(b, &card_id)?, link, &who)?)
            },
        ),
        op(
            "card_execution_recover",
            "Hand this card's execution to a replacement assignment generation. Compare-and-set: refused unless the stored link still carries expected_generation and names the same execution, goal, BBP task, attempt and assignment; the new generation must be higher.",
            |b, i, a: RecoverExecutionArgs| {
                let expected = a.expected_generation;
                let (who, card_id, link) = a.link.link();
                let who = acting(b, i, &who)?;
                card(b.recover_execution(card_ref(b, &card_id)?, expected, link, &who)?)
            },
        ),
        op(
            "card_execution_observe",
            "Record what you saw of execution outside Orch that works on this card: a Codex Cloud thread, a GitHub PR or an Actions run, and its state. Observation only: nothing on the board acts on it. Upserts by provider and ref; a replay or an older observation changes nothing, and the same time with a different state is refused. At most 8 refs per card: a new one replaces the finished ref seen longest ago, and is refused if none has finished. Needs `build`.",
            |b, i, a: ObserveArgs| {
                let (who, card_id, obs) = a.observation();
                let who = acting(b, i, &who)?;
                card(b.observe_execution(card_ref(b, &card_id)?, obs, &who)?)
            },
        ),
        op(
            "project_add",
            "Add a project to a repository (needs `project`). Additive only: existing projects are never changed or removed, and a new project's paths may not overlap another's. Returns the projects as `board_projects` does.",
            |b, i, a: ProjectAddArgs| {
                let who = acting(b, i, &a.who)?;
                let projects = b.add_project(&who, &a.repo, &a.name, &a.paths)?;
                Ok(json!({ "repos": projects }))
            },
        ),
        op(
            "card_assign",
            "Move a card to another project (needs `triage`, or `project` for a card nobody else has claimed; a Done card needs `triage` or `close` and stays Done). Drops any claim.",
            |b, i, a: AssignArgs| {
                let who = acting(b, i, &a.who)?;
                card(b.assign(card_ref(b, &a.id)?, &a.project, &who)?)
            },
        ),
        op(
            "card_move",
            "Move a card to a column (an id from `board_columns`). Needs the capability of that transition (`board_columns` lists them; a gate also needs admin); leaving a work column needs your claim. Any open card may be moved to blocked. While an admin has turned movement rules off, any move but out of Done and through a gate is allowed.",
            |b, i, a: MoveArgs| {
                let who = acting(b, i, &a.who)?;
                card(b.move_card(card_ref(b, &a.id)?, &a.to, &who)?)
            },
        ),
        op(
            "card_claim",
            "Claim a card so only you move it. Fails if someone else holds it.",
            |b, i, a: IdArgs| {
                let who = acting(b, i, &a.who)?;
                card(b.claim(card_ref(b, &a.id)?, &who)?)
            },
        ),
        op(
            "card_release",
            "Release your claim on a card.",
            |b, i, a: IdArgs| {
                let who = acting(b, i, &a.who)?;
                card(b.release(card_ref(b, &a.id)?, &who)?)
            },
        ),
        op(
            "card_evidence",
            "Add evidence that a card's work shipped: a merged PR, or any http(s) page that shows it (at most 8 per card; links already there are kept). A Done card counts as shipped in `board_metrics` only with evidence; without, it is reported as a prototype. Needs `close`, `merge` or `triage`.",
            |b, i, a: EvidenceArgs| {
                let who = acting(b, i, &a.who)?;
                card(b.add_evidence(card_ref(b, &a.id)?, &a.links, &who)?)
            },
        ),
        op(
            "card_close",
            "Close an unstarted card (in backlog or a queue) without shipping it. `reason` is required: `wont_do`, `duplicate` or `superseded`; an optional `note` is added as your comment. The card goes to done with no ship time, counts as dropped in `board_metrics`, never as shipped. Needs `triage`. Refused for started, blocked or done cards.",
            |b, i, a: CloseArgs| {
                let who = acting(b, i, &a.who)?;
                card(b.close(card_ref(b, &a.id)?, a.reason, a.note.as_deref(), &who)?)
            },
        ),
        op(
            "card_archive",
            "Archive a Done card: it leaves the board's default views but stays Done, counts in roadmap and metrics, and `card_get` still finds it. Needs `close`. Refused for an open card.",
            |b, i, a: IdArgs| {
                let who = acting(b, i, &a.who)?;
                card(b.archive(card_ref(b, &a.id)?, &who)?)
            },
        ),
        op(
            "card_restore",
            "Put an archived card back on the board's default views. Needs `close`.",
            |b, i, a: IdArgs| {
                let who = acting(b, i, &a.who)?;
                card(b.restore(card_ref(b, &a.id)?, &who)?)
            },
        ),
        op(
            "card_archive_done",
            "Archive every Done card not yet archived, optionally only one project's and only those done at least `older_than_days` ago. Needs `triage`. {archived: count}.",
            |b, i, a: ArchiveDoneArgs| {
                checked_project(b, scope(i), a.project.as_deref())?;
                let who = acting(b, i, &a.who)?;
                let archived = b.archive_done(&who, a.project.as_deref(), a.older_than_days)?;
                to_json(json!({ "archived": archived.len() }))
            },
        ),
        op(
            "card_update",
            "Edit a card's title, body, tags or prompt (instructions for whoever takes it, shown apart from the body). Pass `rev` (the revision you last read) to refuse the edit if the card has changed since.",
            |b, i, a: UpdateArgs| {
                let who = acting(b, i, &a.who)?;
                let edit = Edit {
                    title: a.title.as_deref(),
                    body: a.body.as_deref(),
                    tags: a.tags,
                    prompt: a.prompt.as_deref(),
                };
                card(b.edit_at(card_ref(b, &a.id)?, &who, a.rev, &edit)?)
            },
        ),
        op(
            "card_plan",
            "Set a card's priority, links and target date (needs Triage): `deps` (waits on), `superseded_by` (newer cards that replace it), `related`, `parent` (the card it was split from; `null` clears it), `milestone` (a milestone of its project; `null` clears it), and `target_ms` (when it should be done; `null` clears it). Each list replaces the current one; `[]` clears it. `card_take` takes higher priority first, skips a card until its dependencies are Done, and never hands out a superseded card. After marking a card superseded, close it (move it to done) or re-scope it and clear `superseded_by`.",
            |b, i, a: PlanArgs| {
                let who = acting(b, i, &a.who)?;
                let refs = |l: Option<Vec<String>>| {
                    l.map(|l| {
                        l.iter()
                            .map(|r| card_ref(b, r))
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .transpose()
                };
                let plan = Plan {
                    priority: a.priority,
                    deps: refs(a.deps)?,
                    superseded_by: refs(a.superseded_by)?,
                    related: refs(a.related)?,
                    parent: a
                        .parent
                        .map(|p| p.map(|r| card_ref(b, &r)).transpose())
                        .transpose()?,
                    milestone: a
                        .milestone
                        .map(|m| m.map(|m| id(&m)).transpose())
                        .transpose()?,
                    target_ms: a.target_ms,
                };
                card(b.plan(card_ref(b, &a.id)?, &who, &plan)?)
            },
        ),
        op(
            "inbox_list",
            "The decisions waiting for the owner (needs an authenticated admin credential): open BBP escalations of the tasks linked from cards, irreversible first, then oldest. Each has an `id`, a `rev`, the `reason` (and `label` in plain words), the claim with its confidence and budget, the strongest evidence, and its card. Empty means nothing needs you.",
            |b, i, a: WhoArgs| {
                let who = bound_acting(b, i, &a.who)?;
                Ok(json!(b.inbox(&who)?))
            },
        ),
        op(
            "inbox_act",
            "Decide an escalation from `inbox_list` (needs an authenticated admin credential). `action`: `approve` (do it, keep it, or spend exactly one more allowance), `reject` (drop it; a task total needs `reason`), `choose` with `chosen` (a stuck question's candidate id), or `refine` with `note` (clarify the intent, then the lenses judge again). Pass the `rev` you read; `dry_run: true` shows what BBP would write and writes nothing.",
            |b, i, a: InboxActArgs| {
                let who = bound_acting(b, i, &a.who)?;
                Ok(json!(
                    b.inbox_act(&who, &a.id, a.rev, &a.action, a.dry_run)?
                ))
            },
        ),
        op(
            "board_number_cards",
            "Number the cards that have no number yet: tasks tagged `agent` directly in rusty_tick since the board started (the board also does this when it starts). Needs an authenticated admin credential, unlimited to projects. {numbered: count}.",
            |b, i, a: WhoArgs| {
                let who = bound_acting(b, i, &a.who)?;
                to_json(json!({ "numbered": b.number_cards_as(&who)? }))
            },
        ),
        op(
            "card_comment",
            "Add a comment to a card.",
            |b, i, a: CommentArgs| {
                let who = acting(b, i, &a.who)?;
                card(b.comment(card_ref(b, &a.id)?, &who, &a.text)?)
            },
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Roles;
    use crate::domain::Capability;
    use crate::store::MemStore;

    fn call(name: &str, args: Value) -> Result<Value, CallError> {
        let board: Shared = Board::new(Box::new(MemStore::default()), Roles::default());
        call_as(&board, None, name, args)
    }

    fn call_as(
        board: &Shared,
        who: Option<&Identity>,
        name: &str,
        args: Value,
    ) -> Result<Value, CallError> {
        ops()
            .iter()
            .find(|o| o.name == name)
            .expect("op")
            .call(board, who, args)
    }

    #[test]
    fn schemas_are_objects_with_properties() {
        for o in ops() {
            assert_eq!(o.schema["type"], "object", "{}", o.name);
        }
        let add = ops()
            .into_iter()
            .find(|o| o.name == "card_add")
            .expect("add");
        assert!(add.schema["properties"]["actor"].is_object());
        assert!(add.schema["properties"]["title"].is_object());
    }

    #[test]
    fn board_list_leaves_archived_cards_out_unless_asked() {
        let board: Shared = Board::new(Box::new(MemStore::default()), Roles::default());
        let human = json!({"actor": "nano", "role": "human"});
        let with = |extra: Value| {
            let mut args = human.clone();
            args.as_object_mut()
                .expect("object")
                .extend(extra.as_object().expect("object").clone());
            args
        };
        call_as(&board, None, "card_add", with(json!({"title": "open"}))).expect("add");
        call_as(
            &board,
            None,
            "card_add",
            with(json!({"title": "shipped", "done": true})),
        )
        .expect("add done");
        let titles = |args: Value| -> Vec<String> {
            call_as(&board, None, "board_list", args).expect("list")["cards"]
                .as_array()
                .expect("cards")
                .iter()
                .map(|c| c["title"].as_str().expect("title").to_owned())
                .collect()
        };
        assert_eq!(titles(json!({})).len(), 2);

        let archived = call_as(&board, None, "card_archive_done", with(json!({}))).expect("bulk");
        assert_eq!(archived["archived"], 1);
        assert_eq!(titles(json!({})), ["open"]);
        assert_eq!(titles(json!({"archived": "only"})), ["shipped"]);
        assert_eq!(titles(json!({"archived": "include"})).len(), 2);

        call_as(&board, None, "card_restore", with(json!({"id": "#2"}))).expect("restore");
        assert_eq!(titles(json!({})).len(), 2);
        call_as(&board, None, "card_archive", with(json!({"id": "#2"}))).expect("archive");
        assert_eq!(titles(json!({"archived": "only"})), ["shipped"]);
        assert!(matches!(
            call_as(&board, None, "card_archive", with(json!({"id": "#1"}))),
            Err(CallError::Board(BoardError::Invalid(_)))
        ));
    }

    /// A clock that moves on 1 ms per reading, so cards list in the order they were made.
    fn ticking() -> i64 {
        static NOW: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(1);
        NOW.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    /// An in-memory board on [`ticking`] time, for tests that depend on list order.
    fn paged_board() -> Shared {
        Board::with_clock(Box::new(MemStore::default()), Roles::default(), ticking)
    }

    /// `board_list` pages of `args`, followed to the end with `limit`: every card's JSON.
    fn scan(board: &Shared, who: Option<&Identity>, args: Value, limit: u32) -> Vec<Value> {
        let mut cards = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut a = args.clone();
            a["limit"] = json!(limit);
            if let Some(c) = &cursor {
                a["cursor"] = json!(c);
            }
            let out = call_as(board, who, "board_list", a).expect("page");
            let page = out["cards"].as_array().expect("cards");
            assert!(page.len() <= limit as usize);
            assert!(out["as_of_ms"].is_i64());
            cards.extend(page.iter().cloned());
            match out["next_cursor"].as_str() {
                Some(next) => cursor = Some(next.to_owned()),
                None => return cards,
            }
        }
    }

    fn add_cards(board: &Shared, n: usize) {
        let human = json!({"actor": "nano", "role": "human"});
        for i in 0..n {
            let mut args = human.clone();
            args["title"] = json!(format!("card {i}"));
            args["body"] = json!("body text");
            call_as(board, None, "card_add", args).expect("add");
            let mut c = human.clone();
            c["id"] = json!(format!("#{}", i + 1));
            c["text"] = json!("a comment");
            call_as(board, None, "card_comment", c).expect("comment");
        }
    }

    #[test]
    fn board_list_defaults_to_summaries_and_gives_whole_cards_when_asked() {
        let board = paged_board();
        add_cards(&board, 2);
        let out = call_as(&board, None, "board_list", json!({})).expect("list");
        let keys: Vec<&String> = out.as_object().expect("object").keys().collect();
        assert_eq!(keys, ["cards"]);
        assert!(out["cards"][0].get("body").is_none(), "summary by default");
        let out = call_as(&board, None, "board_list", json!({"view": "full"})).expect("full");
        assert_eq!(out["cards"][0]["body"], "body text");
        assert_eq!(out["cards"][0]["comments"][0]["text"], "a comment");
    }

    #[test]
    fn summaries_leave_out_body_prompt_links_and_comments() {
        let board = paged_board();
        add_cards(&board, 1);
        let out = call_as(&board, None, "board_list", json!({"view": "summary"})).expect("summary");
        let card = out["cards"][0].as_object().expect("card");
        for absent in ["body", "prompt", "comments", "deps", "parent", "execution"] {
            assert!(!card.contains_key(absent), "{absent}");
        }
        for present in [
            "id",
            "number",
            "rev",
            "title",
            "project",
            "column",
            "created_ms",
        ] {
            assert!(card.contains_key(present), "{present}");
        }
    }

    #[test]
    fn a_paged_scan_returns_exactly_the_unpaged_list() {
        let board = paged_board();
        add_cards(&board, 11);
        for view in ["full", "summary"] {
            let args = json!({"view": view, "archived": "include"});
            let all = call_as(&board, None, "board_list", args.clone()).expect("all")["cards"]
                .as_array()
                .expect("cards")
                .clone();
            for limit in [1, 3, 11, 500] {
                assert_eq!(
                    scan(&board, None, args.clone(), limit),
                    all,
                    "{view} {limit}"
                );
            }
        }
    }

    #[test]
    fn edits_and_new_cards_during_a_scan_never_repeat_or_drop_a_card() {
        let board = paged_board();
        add_cards(&board, 6);
        let human = json!({"actor": "nano", "role": "human"});
        let first = call_as(&board, None, "board_list", json!({"limit": 3})).expect("page");
        let cursor = first["next_cursor"].as_str().expect("more").to_owned();
        // An edit to a card already returned and to one still ahead, and a new card.
        for (id, title) in [("#1", "edited back"), ("#5", "edited ahead")] {
            let mut a = human.clone();
            a["id"] = json!(id);
            a["title"] = json!(title);
            call_as(&board, None, "card_update", a).expect("edit");
        }
        let mut a = human.clone();
        a["title"] = json!("added");
        call_as(&board, None, "card_add", a).expect("add");
        let rest = call_as(
            &board,
            None,
            "board_list",
            json!({"limit": 100, "cursor": cursor}),
        )
        .expect("rest");
        let titles = |v: &Value| -> Vec<String> {
            v["cards"]
                .as_array()
                .expect("cards")
                .iter()
                .map(|c| c["title"].as_str().expect("t").to_owned())
                .collect()
        };
        assert_eq!(titles(&first), ["card 0", "card 1", "card 2"]);
        assert_eq!(titles(&rest), ["card 3", "edited ahead", "card 5", "added"]);
        assert!(rest["next_cursor"].is_null());
    }

    #[test]
    fn bad_limits_and_cursors_are_refused_not_answered_empty() {
        let board = paged_board();
        add_cards(&board, 3);
        let first = call_as(&board, None, "board_list", json!({"limit": 1})).expect("page");
        let cursor = first["next_cursor"].as_str().expect("more").to_owned();
        let refused = |args: Value| {
            matches!(
                call_as(&board, None, "board_list", args),
                Err(CallError::Board(BoardError::Invalid(_)))
            )
        };
        assert!(refused(json!({"limit": 0})));
        assert!(refused(json!({"limit": 501})));
        assert!(refused(json!({"limit": 1, "cursor": "garbage"})));
        assert!(refused(json!({"limit": 1, "cursor": format!("{cursor}x")})));
        // The same cursor with other arguments is another query.
        assert!(refused(
            json!({"limit": 1, "cursor": cursor, "view": "full"})
        ));
        assert!(refused(
            json!({"limit": 1, "cursor": cursor, "archived": "include"})
        ));
        // With the query unchanged and only the limit different, it continues.
        let next = call_as(
            &board,
            None,
            "board_list",
            json!({"limit": 5, "cursor": cursor}),
        )
        .expect("continue");
        assert_eq!(next["cards"].as_array().expect("cards").len(), 2);
    }

    #[test]
    fn a_page_stops_at_its_byte_budget_and_keeps_every_card_whole() {
        let board = paged_board();
        let human = json!({"actor": "nano", "role": "human"});
        let body = "b".repeat(300 * 1024);
        for i in 0..3 {
            let mut a = human.clone();
            a["title"] = json!(format!("big {i}"));
            a["body"] = json!(body);
            call_as(&board, None, "card_add", a).expect("add");
        }
        let full = json!({"limit": 500, "view": "full"});
        let first = call_as(&board, None, "board_list", full).expect("page");
        assert_eq!(first["cards"].as_array().expect("cards").len(), 1);
        let cards = scan(&board, None, json!({"view": "full"}), 500);
        assert_eq!(cards.len(), 3);
        assert!(cards.iter().all(|c| c["body"] == body.as_str()));
    }

    #[test]
    fn board_list_keeps_cards_by_created_updated_and_moved_dates() {
        use std::sync::atomic::{AtomicI64, Ordering};
        static NOW: AtomicI64 = AtomicI64::new(10);
        let board: Shared =
            Board::with_clock(Box::new(MemStore::default()), Roles::default(), || {
                NOW.load(Ordering::SeqCst)
            });
        let human = |extra: Value| {
            let mut args = json!({"actor": "nano", "role": "human"});
            args.as_object_mut()
                .expect("object")
                .extend(extra.as_object().expect("object").clone());
            args
        };
        let add = |title: &str| {
            call_as(&board, None, "card_add", human(json!({ "title": title }))).expect("add")
        };
        add("first");
        NOW.store(20, Ordering::SeqCst);
        add("second");
        NOW.store(30, Ordering::SeqCst);
        call_as(
            &board,
            None,
            "card_comment",
            human(json!({"id": "#1", "text": "hi"})),
        )
        .expect("comment");
        NOW.store(40, Ordering::SeqCst);
        call_as(
            &board,
            None,
            "card_move",
            human(json!({"id": "#2", "to": "blocked"})),
        )
        .expect("move");
        let titles = |args: Value| -> Vec<String> {
            call_as(&board, None, "board_list", args).expect("list")["cards"]
                .as_array()
                .expect("cards")
                .iter()
                .map(|c| c["title"].as_str().expect("title").to_owned())
                .collect()
        };
        assert_eq!(titles(json!({"created_from": 15})), ["second"]);
        assert_eq!(
            titles(json!({"created_to": 20})),
            ["first"],
            "`to` is exclusive"
        );
        assert_eq!(
            titles(json!({"updated_from": 25, "updated_to": 35})),
            ["first"]
        );
        assert_eq!(
            titles(json!({"moved_from": 35})),
            ["second"],
            "a comment is not a move"
        );
        assert_eq!(
            titles(json!({"moved_from": 0, "moved_to": 15})),
            ["first"],
            "entering the board counts"
        );
        assert_eq!(titles(json!({})).len(), 2);
    }

    #[test]
    fn card_restore_puts_an_aged_out_card_back_in_board_list() {
        use std::sync::atomic::{AtomicI64, Ordering};
        static NOW: AtomicI64 = AtomicI64::new(0);
        let board: Shared =
            Board::with_clock(Box::new(MemStore::default()), Roles::default(), || {
                NOW.load(Ordering::SeqCst)
            });
        let human = |extra: Value| {
            let mut args = json!({"actor": "nano", "role": "human"});
            args.as_object_mut()
                .expect("object")
                .extend(extra.as_object().expect("object").clone());
            args
        };
        call_as(
            &board,
            None,
            "card_add",
            human(json!({"title": "old", "done": true})),
        )
        .expect("add done");
        let count = |args: Value| {
            call_as(&board, None, "board_list", args).expect("list")["cards"]
                .as_array()
                .expect("cards")
                .len()
        };
        NOW.store(20 * 86_400_000, Ordering::SeqCst);
        assert_eq!(
            (count(json!({})), count(json!({"archived": "only"}))),
            (0, 1)
        );
        assert_eq!(count(json!({"done_days": 30})), 1);

        call_as(&board, None, "card_restore", human(json!({"id": "#1"}))).expect("restore");
        assert_eq!(
            (count(json!({})), count(json!({"archived": "only"}))),
            (1, 0)
        );
        let bulk = call_as(
            &board,
            None,
            "card_archive_done",
            human(json!({"older_than_days": 5})),
        )
        .expect("bulk");
        assert_eq!(bulk["archived"], 0, "the window counts from the restore");
    }

    /// A board with projects `on`, `test`, `held` and `gone`, a Ready card in each, and
    /// `test` an experiment, `held` paused and `gone` archived (set after the cards went in).
    fn statused_board() -> Shared {
        use crate::domain::ProjectStatus::{Archived, Experiment, Paused};
        let projects = crate::projects::Projects::parse(
            r#"{"on": ["a"], "test": ["b"], "held": ["c"], "gone": ["d"]}"#,
        )
        .expect("projects");
        let board: Shared = Board::new(
            Box::new(MemStore::default()) as Box<dyn Store>,
            Roles::default(),
        )
        .with_projects(projects);
        let human = json!({"actor": "nano", "role": "human"});
        for p in ["on", "test", "held", "gone"] {
            let mut args = human.clone();
            args["title"] = json!(p);
            args["project"] = json!(p);
            let c = call_as(&board, None, "card_add", args).expect("add");
            let mut mv = human.clone();
            mv["id"] = c["card"]["id"].clone();
            mv["to"] = json!("ready");
            call_as(&board, None, "card_move", mv).expect("ready");
        }
        for (p, s) in [("test", Experiment), ("held", Paused), ("gone", Archived)] {
            board.set_project_status(p, s, |_| Ok(())).expect("status");
        }
        board
    }

    #[test]
    fn board_list_shows_projects_that_take_work_unless_asked_or_one_is_named() {
        let board = statused_board();
        let titles = |args: Value| -> Vec<String> {
            let mut t: Vec<String> =
                call_as(&board, None, "board_list", args).expect("list")["cards"]
                    .as_array()
                    .expect("cards")
                    .iter()
                    .map(|c| c["title"].as_str().expect("title").to_owned())
                    .collect();
            t.sort();
            t
        };
        assert_eq!(titles(json!({})), ["on", "test"], "projects that take work");
        assert_eq!(titles(json!({"project": "held"})), ["held"]);
        assert_eq!(
            titles(json!({"project_status": ["experiment", "paused"]})),
            ["held", "test"]
        );
        assert_eq!(
            titles(json!({"project_status": ["active", "paused", "experiment", "archived"]})).len(),
            4
        );
        let listed = call_as(&board, None, "board_projects", json!({})).expect("projects");
        assert_eq!(
            listed["status"],
            json!({"test": "experiment", "held": "paused", "gone": "archived"})
        );
    }

    #[test]
    fn paused_and_archived_projects_refuse_cards_and_hand_out_no_work() {
        let board = statused_board();
        let add = |p: &str| {
            call_as(
                &board,
                None,
                "card_add",
                json!({"actor": "nano", "role": "human", "title": "new", "project": p}),
            )
        };
        assert!(
            add("on").is_ok() && add("test").is_ok(),
            "active and experiment take cards"
        );
        for p in ["held", "gone"] {
            assert!(
                matches!(add(p), Err(CallError::Board(BoardError::Refused(_)))),
                "{p}"
            );
        }
        let take = |p: &str| {
            call_as(
                &board,
                None,
                "card_take",
                json!({"actor": "dev", "role": "builder", "project": p}),
            )
            .expect("take")["card"]
                .clone()
        };
        assert_eq!(
            take("held"),
            Value::Null,
            "a paused project's work is not handed out"
        );
        assert_eq!(take("test")["title"], "test", "an experiment's is");

        // Open work moves only into a project that takes it; a Done card is re-filed anywhere.
        let on = |args: Value| {
            let mut a = json!({"actor": "nano", "role": "human"});
            a.as_object_mut()
                .expect("object")
                .extend(args.as_object().expect("object").clone());
            a
        };
        let moved = call_as(
            &board,
            None,
            "card_assign",
            on(json!({"id": "#1", "project": "held"})),
        );
        assert!(matches!(
            moved,
            Err(CallError::Board(BoardError::Refused(_)))
        ));
        call_as(
            &board,
            None,
            "card_close",
            on(json!({"id": "#1", "reason": "wont_do"})),
        )
        .expect("close");
        call_as(
            &board,
            None,
            "card_assign",
            on(json!({"id": "#1", "project": "gone"})),
        )
        .expect("re-file");
    }

    #[test]
    fn project_status_edges_hold() {
        let board = statused_board();
        // Taking a paused project's card by id says why, rather than that it is missing.
        let took = call_as(
            &board,
            None,
            "card_take",
            json!({"actor": "dev", "role": "builder", "id": "#3"}),
        );
        assert!(
            matches!(took, Err(CallError::Board(BoardError::Refused(_)))),
            "{took:?}"
        );
        // Nothing starts in a paused project: no claim, no move into a work column.
        let nano = |extra: Value| {
            let mut a = json!({"actor": "nano", "role": "human"});
            a.as_object_mut()
                .expect("object")
                .extend(extra.as_object().expect("object").clone());
            a
        };
        for (op, args) in [
            ("card_claim", json!({"id": "#3"})),
            ("card_move", json!({"id": "#3", "to": "building"})),
        ] {
            let out = call_as(&board, None, op, nano(args));
            assert!(
                matches!(out, Err(CallError::Board(BoardError::Refused(_)))),
                "{op}: {out:?}"
            );
        }
        // Work under way when the project was paused may move on.
        call_as(
            &board,
            None,
            "card_move",
            nano(json!({"id": "#1", "to": "building"})),
        )
        .expect("start on");
        board
            .set_project_status("on", ProjectStatus::Paused, |_| Ok(()))
            .expect("pause on");
        call_as(
            &board,
            None,
            "card_move",
            nano(json!({"id": "#1", "to": "qa"})),
        )
        .expect("finishes");
        board
            .set_project_status("on", ProjectStatus::Active, |_| Ok(()))
            .expect("resume on");
        // A failed write leaves the status as it was.
        let failed =
            board.set_project_status("on", ProjectStatus::Paused, |_| Err("disk full".to_owned()));
        assert!(matches!(failed, Err(BoardError::Refused(_))));
        assert_eq!(board.statuses().of("on"), ProjectStatus::Active);
        // Forge issues are not imported into a paused project.
        board
            .set_project_status(DEFAULT_PROJECT, ProjectStatus::Paused, |_| Ok(()))
            .expect("pause default");
        let bot = board.roles().actor("sync", "human").expect("actor");
        assert!(matches!(
            board.import(&bot, "issue", "", Vec::new(), 7),
            Err(BoardError::Refused(_))
        ));
        // The cursor is bound to the set of statuses, not their order.
        let page = |order: Value, cursor: Value| {
            call_as(
                &board,
                None,
                "board_list",
                json!({"project_status": order, "limit": 1, "cursor": cursor}),
            )
        };
        let first = page(json!(["paused", "experiment"]), Value::Null).expect("first");
        let next = first["next_cursor"].clone();
        assert!(next.is_string(), "{first}");
        assert!(page(json!(["experiment", "paused"]), next).is_ok());
    }

    #[test]
    fn a_status_does_not_outlive_its_project() {
        let projects =
            crate::projects::Projects::parse(r#"{"keep": ["a"], "spare": ["b"]}"#).expect("parse");
        let without = crate::projects::Projects::parse(r#"{"keep": ["a"]}"#).expect("parse");
        let board: Shared = Board::new(
            Box::new(MemStore::default()) as Box<dyn Store>,
            Roles::default(),
        )
        .with_projects(projects);
        board
            .set_project_status("spare", ProjectStatus::Paused, |_| Ok(()))
            .expect("pause");
        let removed = board.set_projects(without.clone(), |_| Ok(()));
        assert!(
            matches!(&removed, Err(BoardError::Refused(why)) if why.contains("make it active")),
            "{removed:?}"
        );
        board
            .set_project_status("spare", ProjectStatus::Active, |_| Ok(()))
            .expect("activate");
        board
            .set_projects(without, |_| Ok(()))
            .expect("removed once active");
    }

    #[test]
    fn card_close_drops_a_backlog_card_with_its_reason_and_note() {
        let board: Shared = Board::new(Box::new(MemStore::default()), Roles::default());
        let as_ = |role: &str, extra: Value| {
            let mut args = json!({"actor": "nano", "role": role});
            args.as_object_mut()
                .expect("object")
                .extend(extra.as_object().expect("object").clone());
            args
        };
        for title in ["keep", "drop"] {
            call_as(
                &board,
                None,
                "card_add",
                as_("human", json!({"title": title})),
            )
            .expect("add");
        }
        let missing = call_as(
            &board,
            None,
            "card_close",
            as_("human", json!({"id": "#2"})),
        );
        assert!(
            matches!(missing, Err(CallError::BadArgs(_))),
            "reason is required"
        );
        let builder = call_as(
            &board,
            None,
            "card_close",
            as_("builder", json!({"id": "#2", "reason": "wont_do"})),
        );
        assert!(matches!(
            builder,
            Err(CallError::Board(BoardError::Forbidden(Capability::Triage)))
        ));

        let closed = call_as(
            &board,
            None,
            "card_close",
            as_(
                "human",
                json!({"id": "#2", "reason": "duplicate", "note": "same as #1"}),
            ),
        )
        .expect("close")["card"]
            .clone();
        assert_eq!(closed["column"], "done");
        assert_eq!(closed["done_ms"], Value::Null);
        assert_eq!(closed["closed"]["reason"], "duplicate");
        assert_eq!(closed["comments"][0]["text"], "same as #1");

        let m = call_as(&board, None, "board_metrics", json!({})).expect("metrics");
        assert_eq!(m["open"], 1);
        assert_eq!(m["lead_time"]["shipped"], 0);
        assert_eq!(m["dropped"][1], json!({"key": "duplicate", "count": 1}));
    }

    #[test]
    fn a_refused_close_adds_no_note() {
        let board: Shared = Board::new(Box::new(MemStore::default()), Roles::default());
        let human = |extra: Value| {
            let mut a = json!({"actor": "nano", "role": "human"});
            a.as_object_mut()
                .expect("object")
                .extend(extra.as_object().expect("object").clone());
            a
        };
        call_as(&board, None, "card_add", human(json!({"title": "t"}))).expect("add");
        call_as(
            &board,
            None,
            "card_move",
            human(json!({"id": "#1", "to": "ready"})),
        )
        .expect("ready");
        call_as(
            &board,
            None,
            "card_move",
            human(json!({"id": "#1", "to": "building"})),
        )
        .expect("start");
        let refused = call_as(
            &board,
            None,
            "card_close",
            human(json!({"id": "#1", "reason": "wont_do", "note": "nope"})),
        );
        assert!(matches!(
            refused,
            Err(CallError::Board(BoardError::Refused(_)))
        ));
        let got = call_as(&board, None, "card_get", json!({"id": "#1"})).expect("get");
        assert_eq!(got["card"]["comments"], json!([]));
    }

    #[test]
    fn evidence_is_added_by_whoever_records_shipped_work() {
        let board: Shared = Board::new(Box::new(MemStore::default()), Roles::default());
        let as_ = |role: &str, extra: Value| {
            let mut a = json!({"actor": "nano", "role": role});
            a.as_object_mut()
                .expect("object")
                .extend(extra.as_object().expect("object").clone());
            a
        };
        let pr = "https://github.com/o/r/pull/9";
        let added = call_as(
            &board,
            None,
            "card_add",
            as_(
                "builder",
                json!({"title": "merged", "done": true, "evidence": [pr]}),
            ),
        )
        .expect("add")["card"]
            .clone();
        assert_eq!(added["evidence"], json!([pr]));
        call_as(
            &board,
            None,
            "card_add",
            as_("human", json!({"title": "t"})),
        )
        .expect("add");
        let refused = call_as(
            &board,
            None,
            "card_evidence",
            as_("reviewer", json!({"id": "#2", "links": [pr]})),
        );
        assert!(
            matches!(&refused, Err(CallError::Board(BoardError::ForbiddenAny(cs))) if cs.len() == 3),
            "{refused:?}"
        );
        // Nor can evidence come in with a new card, done or not, without those capabilities.
        for done in [false, true] {
            let sneaked = call_as(
                &board,
                None,
                "card_add",
                as_(
                    "reviewer",
                    json!({"title": "sneak", "done": done, "evidence": [pr]}),
                ),
            );
            // The Done gate (`Close`) answers first for a done card.
            let gated = if done {
                matches!(
                    sneaked,
                    Err(CallError::Board(BoardError::Forbidden(Capability::Close)))
                )
            } else {
                matches!(sneaked, Err(CallError::Board(BoardError::ForbiddenAny(_))))
            };
            assert!(gated, "done {done}: {sneaked:?}");
        }
        let bad = call_as(
            &board,
            None,
            "card_evidence",
            as_("human", json!({"id": "#2", "links": ["not a link"]})),
        );
        assert!(matches!(bad, Err(CallError::Board(BoardError::Invalid(_)))));
        let ok = call_as(
            &board,
            None,
            "card_evidence",
            as_("human", json!({"id": "#2", "links": [pr]})),
        )
        .expect("evidence");
        assert_eq!(ok["card"]["evidence"], json!([pr]));
        let m = call_as(&board, None, "board_metrics", json!({})).expect("metrics");
        assert_eq!(
            m["lead_time"]["shipped"], 1,
            "the merged card counts as shipped"
        );
    }

    #[test]
    fn a_card_is_named_by_id_or_by_number() {
        let board: Shared = Board::new(Box::new(MemStore::default()), Roles::default());
        let add = json!({"actor": "nano", "role": "human", "title": "t"});
        let first = call_as(&board, None, "card_add", add.clone()).expect("add");
        call_as(&board, None, "card_add", add).expect("add");
        assert_eq!(first["card"]["number"], 1);
        for name in [first["card"]["id"].clone(), json!("#1"), json!(" 1 ")] {
            let got = call_as(&board, None, "card_get", json!({"id": name})).expect("get");
            assert_eq!(got["card"]["id"], first["card"]["id"]);
        }
        assert!(matches!(
            call_as(&board, None, "card_get", json!({"id": "#9"})),
            Err(CallError::Board(BoardError::Invalid(
                "no card has that number"
            )))
        ));
        assert!(matches!(
            call_as(&board, None, "card_get", json!({"id": "#x"})),
            Err(CallError::Board(BoardError::Invalid(_)))
        ));
    }

    #[test]
    fn bad_args_and_board_errors_are_distinguished() {
        assert!(matches!(
            call("card_add", json!({"title": 3})),
            Err(CallError::BadArgs(_))
        ));
        assert!(matches!(
            call(
                "card_add",
                json!({"actor": "a", "role": "nope", "title": "t"})
            ),
            Err(CallError::Board(BoardError::Invalid("unknown role")))
        ));
        assert!(matches!(
            call("card_get", json!({"id": "x"})),
            Err(CallError::Board(BoardError::Invalid(_)))
        ));
    }

    #[test]
    fn add_returns_card() {
        let out = call(
            "card_add",
            json!({"actor": "a", "role": "human", "title": "t"}),
        )
        .expect("add");
        assert_eq!(out["card"]["column"], "backlog");
    }

    #[test]
    fn board_number_cards_numbers_strays_for_an_authenticated_unlimited_admin_only() {
        let store = MemStore::default();
        let stray = Card::new("stray", "", vec![], 1).expect("card");
        store
            .insert(
                &stray,
                crate::domain::Event::of(&stray, crate::domain::EventKind::Created, "x"),
            )
            .expect("insert");
        let board: Shared = Board::new(Box::new(store), Roles::default());
        let refused = |who: &Identity| {
            matches!(
                call_as(&board, Some(who), "board_number_cards", json!({})),
                Err(CallError::Board(BoardError::Forbidden(_)))
            )
        };
        assert!(refused(&ident("claude", "builder")));
        let mut limited = ident("nano", "human");
        limited.projects = Some(vec!["default".into()]);
        assert!(refused(&limited), "it writes every project's cards");
        let stated = json!({"actor": "nano", "role": "human"});
        assert!(
            matches!(
                call_as(&board, None, "board_number_cards", stated),
                Err(CallError::Board(BoardError::Invalid(m))) if m.contains("authenticated admin credential")
            ),
            "a stated admin role without a credential is refused"
        );
        assert!(
            call_as(&board, None, "board_number_cards", json!({})).is_err(),
            "no credential and no actor is refused"
        );
        let human = ident("nano", "human");
        let out = call_as(&board, Some(&human), "board_number_cards", json!({})).expect("number");
        assert_eq!(out["numbered"], 1);
        assert_eq!(board.get(stray.id).expect("get").number, Some(1));
        let again = call_as(&board, Some(&human), "board_number_cards", json!({})).expect("again");
        assert_eq!(again["numbered"], 0);
    }

    fn ident(actor: &str, role: &str) -> Identity {
        Identity {
            projects: None,
            actor: actor.into(),
            role: role.into(),
        }
    }

    #[test]
    fn card_add_done_records_merged_work_and_needs_close() {
        let board: Shared = Board::new(Box::new(MemStore::default()), Roles::default());
        let added = call_as(
            &board,
            Some(&ident("claude", "builder")),
            "card_add",
            json!({"title": "shipped in #1", "done": true}),
        )
        .expect("add done");
        assert_eq!(added["card"]["column"], "done");
        assert!(matches!(
            call_as(
                &board,
                Some(&ident("q", "qa")),
                "card_add",
                json!({"title": "t", "done": true})
            ),
            Err(CallError::Board(BoardError::Forbidden(Capability::Close)))
        ));
        let plain = call_as(
            &board,
            Some(&ident("q", "qa")),
            "card_add",
            json!({"title": "t", "done": false}),
        )
        .expect("add");
        assert_eq!(plain["card"]["column"], "backlog");
    }

    #[test]
    fn a_credential_fixes_actor_and_role() {
        let board: Shared = Board::new(Box::new(MemStore::default()), Roles::default());
        let dev = ident("claude", "builder");
        // No actor/role needed; the card records the credential's actor.
        let added = call_as(&board, Some(&dev), "card_add", json!({"title": "t"})).expect("add");
        assert_eq!(added["card"]["column"], "backlog");
        // Repeating them exactly is fine; claiming to be someone else is refused.
        assert!(
            call_as(
                &board,
                Some(&dev),
                "card_add",
                json!({"title": "t", "actor": "claude", "role": "builder"})
            )
            .is_ok()
        );
        for lie in [
            json!({"title": "t", "role": "human"}),
            json!({"title": "t", "actor": "nano"}),
        ] {
            assert!(
                matches!(call_as(&board, Some(&dev), "card_add", lie), Err(CallError::Board(BoardError::Invalid(m))) if m.contains("fixed by your credential"))
            );
        }
        // Without a credential, they are required.
        assert!(matches!(
            call_as(&board, None, "card_add", json!({"title": "t"})),
            Err(CallError::Board(BoardError::Invalid("actor is required")))
        ));
    }

    #[test]
    fn whoami_reports_identity_and_capabilities() {
        let board: Shared = Board::new(Box::new(MemStore::default()), Roles::default());
        let me = call_as(
            &board,
            Some(&ident("claude", "builder")),
            "board_whoami",
            json!({}),
        )
        .expect("whoami");
        assert_eq!(
            (
                me["authenticated"].clone(),
                me["actor"].clone(),
                me["capabilities"].clone()
            ),
            (
                json!(true),
                json!("claude"),
                json!(["build", "close", "project"])
            )
        );
        let anon = call_as(&board, None, "board_whoami", json!({})).expect("whoami");
        assert_eq!(anon["authenticated"], json!(false));
    }

    #[test]
    fn take_pulls_a_ready_card_for_the_credentialed_model() {
        let board: Shared = Board::new(Box::new(MemStore::default()), Roles::default());
        let (human, dev) = (ident("nano", "human"), ident("claude", "builder"));
        let id = call_as(&board, Some(&human), "card_add", json!({"title": "work"})).expect("add")
            ["card"]["id"]
            .as_str()
            .expect("id")
            .to_owned();
        call_as(
            &board,
            Some(&human),
            "card_move",
            json!({"id": id, "to": "ready"}),
        )
        .expect("ready");

        let taken = call_as(&board, Some(&dev), "card_take", json!({})).expect("take");
        assert_eq!(
            (
                taken["card"]["column"].clone(),
                taken["card"]["claim"].clone()
            ),
            (json!("building"), json!("claude"))
        );
        // Nothing else waiting for a builder.
        let again = call_as(
            &board,
            Some(&ident("codex", "builder")),
            "card_take",
            json!({}),
        )
        .expect("take");
        assert_eq!(again["card"], Value::Null);
    }

    #[test]
    fn events_carry_a_cursor_and_tail_starts_a_watcher_at_the_end() {
        let board: Shared = Board::new(Box::new(MemStore::default()), Roles::default());
        let human = ident("nano", "human");
        for t in ["a", "b", "c"] {
            call_as(&board, Some(&human), "card_add", json!({"title": t})).expect("add");
        }
        let page =
            call_as(&board, Some(&human), "board_events", json!({"limit": 2})).expect("events");
        let first = page["events"].as_array().expect("events");
        assert_eq!(first.len(), 2);
        assert_eq!(
            page["cursor"], first[1]["id"],
            "the cursor is the last event returned"
        );

        let rest = call_as(
            &board,
            Some(&human),
            "board_events",
            json!({"after": page["cursor"]}),
        )
        .expect("events");
        assert_eq!(rest["events"].as_array().expect("events").len(), 1);

        // Nothing newer: no events, and the cursor stays where the caller was.
        let none = call_as(
            &board,
            Some(&human),
            "board_events",
            json!({"after": rest["cursor"]}),
        )
        .expect("events");
        assert_eq!(
            (
                none["events"].as_array().map(Vec::len),
                none["cursor"].clone()
            ),
            (Some(0), rest["cursor"].clone())
        );

        // A watcher starts at the very end, however long the log is.
        let tail =
            call_as(&board, Some(&human), "board_events", json!({"tail": true})).expect("tail");
        assert_eq!(
            (
                tail["events"].as_array().map(Vec::len),
                tail["cursor"].clone()
            ),
            (Some(0), rest["cursor"].clone())
        );
        let empty: Shared = Board::new(Box::new(MemStore::default()), Roles::default());
        assert_eq!(
            call_as(&empty, Some(&human), "board_events", json!({"tail": true})).expect("tail")["cursor"],
            Value::Null
        );
    }

    #[test]
    fn an_update_with_a_stale_revision_is_refused() {
        let board: Shared = Board::new(Box::new(MemStore::default()), Roles::default());
        let human = ident("nano", "human");
        let card =
            call_as(&board, Some(&human), "card_add", json!({"title": "t"})).expect("add")["card"]
                .clone();
        let id = card["id"].clone();
        let rev = card["rev"].clone();

        let ok = call_as(
            &board,
            Some(&human),
            "card_update",
            json!({"id": id, "rev": rev, "title": "one"}),
        )
        .expect("current rev");
        assert_eq!(ok["card"]["title"], "one");
        // The same (now stale) revision again: refused, and nothing changed.
        let stale = call_as(
            &board,
            Some(&human),
            "card_update",
            json!({"id": id, "rev": rev, "title": "two"}),
        );
        assert!(
            matches!(stale, Err(CallError::Board(BoardError::Conflict))),
            "{stale:?}"
        );
        assert_eq!(
            call_as(&board, Some(&human), "card_get", json!({"id": id})).expect("get")["card"]["title"],
            "one"
        );
        // Without a revision the edit applies, as before.
        assert!(
            call_as(
                &board,
                Some(&human),
                "card_update",
                json!({"id": id, "title": "three"})
            )
            .is_ok()
        );
    }

    #[test]
    fn pages_and_cursors_only_ever_span_the_callers_projects() {
        use crate::projects::Projects;
        let projects =
            Projects::parse(r#"{"rl": ["crates/rl"], "orch": ["crates/orch"]}"#).expect("projects");
        let board = paged_board().with_projects(projects);
        let human = ident("nano", "human");
        let mut dev = ident("claude", "builder");
        dev.projects = Some(vec!["rl".into()]);
        // Interleaved, so every page of the whole board mixes the two projects.
        for i in 0..8 {
            let project = if i % 2 == 0 { "rl" } else { "orch" };
            call_as(
                &board,
                Some(&human),
                "card_add",
                json!({"title": format!("{project} {i}"), "project": project}),
            )
            .expect("add");
        }
        for view in ["full", "summary"] {
            let seen = scan(&board, Some(&dev), json!({"view": view}), 2);
            assert_eq!(seen.len(), 4, "{view}");
            assert!(seen.iter().all(|c| c["project"] == "rl"), "{view}");
        }
        // An admin's cursor replayed by the limited credential continues over its cards only.
        let first = call_as(
            &board,
            Some(&human),
            "board_list",
            json!({"limit": 1, "view": "summary"}),
        )
        .expect("admin page");
        let cursor = first["next_cursor"].as_str().expect("more");
        let replay = call_as(
            &board,
            Some(&dev),
            "board_list",
            json!({"limit": 500, "view": "summary", "cursor": cursor}),
        )
        .expect("replay");
        let replayed = replay["cards"].as_array().expect("cards");
        assert_eq!(replayed.len(), 3, "rl cards after the first");
        assert!(replayed.iter().all(|c| c["project"] == "rl"));
        assert!(matches!(
            call_as(
                &board,
                Some(&dev),
                "board_list",
                json!({"limit": 5, "project": "orch"})
            ),
            Err(CallError::Board(BoardError::OutOfScope(_)))
        ));
    }

    #[test]
    fn a_credential_limited_to_projects_sees_and_takes_only_those() {
        use crate::projects::Projects;
        let projects = Projects::parse(
            r#"{"rl": ["crates/apps/rocket_league"], "orch": ["crates/apps/rusty_orch"]}"#,
        )
        .expect("projects");
        let board: Shared = Board::new(
            Box::new(MemStore::default()) as Box<dyn Store>,
            Roles::default(),
        )
        .with_projects(projects);
        let human = ident("nano", "human");
        let mut dev = ident("claude", "builder");
        dev.projects = Some(vec!["rl".into()]);
        let mut ids = Vec::new();
        for (project, title) in [("rl", "rl card"), ("orch", "orch card")] {
            let out = call_as(
                &board,
                Some(&human),
                "card_add",
                json!({"title": title, "project": project}),
            )
            .expect("add");
            let id = out["card"]["id"].as_str().expect("id").to_owned();
            call_as(
                &board,
                Some(&human),
                "card_move",
                json!({"id": id, "to": "ready"}),
            )
            .expect("ready");
            ids.push(id);
        }

        let titles = |who: &Identity, args: Value| -> Vec<String> {
            let out = call_as(&board, Some(who), "board_list", args).expect("list");
            out["cards"]
                .as_array()
                .expect("cards")
                .iter()
                .map(|c| c["title"].as_str().expect("t").to_owned())
                .collect()
        };
        assert_eq!(
            titles(&dev, json!({})),
            ["rl card"],
            "the other project is invisible"
        );
        assert_eq!(titles(&human, json!({})).len(), 2);
        assert_eq!(titles(&human, json!({"project": "orch"})), ["orch card"]);
        assert!(matches!(
            call_as(&board, Some(&dev), "board_list", json!({"project": "orch"})),
            Err(CallError::Board(BoardError::OutOfScope(_)))
        ));
        assert!(matches!(
            call_as(&board, Some(&dev), "card_get", json!({"id": ids[1]})),
            Err(CallError::Board(BoardError::OutOfScope(_)))
        ));
        assert!(matches!(
            call_as(&board, Some(&dev), "card_take", json!({"id": ids[1]})),
            Err(CallError::Board(BoardError::OutOfScope(_)))
        ));
        let taken = call_as(&board, Some(&dev), "card_take", json!({})).expect("take");
        assert_eq!(taken["card"]["title"], "rl card");
        let listed = call_as(&board, Some(&dev), "board_projects", json!({})).expect("projects");
        assert_eq!(listed["default"], "default");
        let repos = listed["repos"].as_object().expect("repos");
        assert_eq!(
            repos.len(),
            1,
            "a limited credential hears of no other project: {repos:?}"
        );
        assert_eq!(repos["default"].as_object().expect("projects").len(), 1);
        let everything =
            call_as(&board, Some(&human), "board_projects", json!({})).expect("projects");
        assert_eq!(
            everything["repos"]["default"]
                .as_object()
                .expect("projects")
                .len(),
            3
        );
        assert_eq!(
            listed["repos"]["default"]["rl"],
            json!(["crates/apps/rocket_league"])
        );
    }

    #[test]
    fn roadmap_lanes_a_project_and_respects_scope() {
        use crate::projects::Projects;
        let projects = Projects::parse(r#"{"rl": ["a"], "orch": ["b"]}"#).expect("projects");
        let board: Shared = Board::new(
            Box::new(MemStore::default()) as Box<dyn Store>,
            Roles::default(),
        )
        .with_projects(projects);
        let human = ident("nano", "human");
        let out = call_as(
            &board,
            Some(&human),
            "card_add",
            json!({"title": "next up", "project": "rl"}),
        )
        .expect("add");
        let id = out["card"]["id"].as_str().expect("id").to_owned();
        call_as(
            &board,
            Some(&human),
            "card_move",
            json!({"id": id, "to": "ready"}),
        )
        .expect("ready");

        let map = call_as(
            &board,
            Some(&human),
            "board_roadmap",
            json!({"project": "rl"}),
        )
        .expect("roadmap");
        let lanes: Vec<&str> = map["lanes"]
            .as_array()
            .expect("lanes")
            .iter()
            .map(|l| l["lane"].as_str().expect("lane"))
            .collect();
        assert_eq!(lanes, ["now", "next", "later", "blocked", "shipped"]);
        assert_eq!(map["lanes"][1]["cards"][0]["title"], "next up");
        assert!(
            map["lanes"][1]["cards"][0].get("body").is_none(),
            "trimmed card"
        );

        assert!(matches!(
            call_as(
                &board,
                Some(&human),
                "board_roadmap",
                json!({"project": "nope"})
            ),
            Err(CallError::Board(BoardError::Invalid("unknown project")))
        ));
        let mut dev = ident("claude", "builder");
        dev.projects = Some(vec!["orch".into()]);
        assert!(matches!(
            call_as(
                &board,
                Some(&dev),
                "board_roadmap",
                json!({"project": "rl"})
            ),
            Err(CallError::Board(BoardError::OutOfScope(_)))
        ));
    }
    #[test]
    fn metrics_narrow_to_scope_project_and_repo() {
        use crate::projects::Projects;
        let projects =
            Projects::parse(r#"{"repos": {"games": {"rl": ["a"]}, "tools": {"orch": ["b"]}}}"#)
                .expect("projects");
        let board: Shared = Board::new(
            Box::new(MemStore::default()) as Box<dyn Store>,
            Roles::default(),
        )
        .with_projects(projects);
        let human = ident("nano", "human");
        for project in ["rl", "rl", "orch"] {
            call_as(
                &board,
                Some(&human),
                "card_add",
                json!({"title": "c", "project": project}),
            )
            .expect("add");
        }
        let open = |who: &Identity, args: Value| {
            call_as(&board, Some(who), "board_metrics", args).expect("metrics")["open"].clone()
        };
        assert_eq!(open(&human, json!({})), 3);
        assert_eq!(open(&human, json!({"project": "rl"})), 2);
        assert_eq!(open(&human, json!({"repo": "games"})), 2);
        let mut dev = ident("claude", "builder");
        dev.projects = Some(vec!["orch".into()]);
        assert_eq!(
            open(&dev, json!({})),
            1,
            "out-of-scope cards are not counted"
        );
        assert!(matches!(
            call_as(
                &board,
                Some(&dev),
                "board_metrics",
                json!({"project": "rl"})
            ),
            Err(CallError::Board(BoardError::OutOfScope(_)))
        ));
        assert!(matches!(
            call_as(
                &board,
                Some(&human),
                "board_metrics",
                json!({"repo": "nope"})
            ),
            Err(CallError::Board(BoardError::Invalid("unknown repository")))
        ));
    }

    #[test]
    fn a_target_date_is_set_kept_when_left_out_and_cleared_by_null() {
        let board: Shared = Board::new(Box::new(MemStore::default()), Roles::default());
        let human = ident("nano", "human");
        let plan = |args: Value| {
            call_as(&board, Some(&human), "card_plan", args)
                .map(|out| out["card"]["target_ms"].clone())
        };
        let added =
            call_as(&board, Some(&human), "card_add", json!({"title": "dated"})).expect("add");
        let id = added["card"]["id"].as_str().expect("id").to_owned();
        assert_eq!(
            plan(json!({"id": id, "target_ms": 1_800_000_000_000_i64})).expect("set"),
            json!(1_800_000_000_000_i64)
        );
        assert_eq!(
            plan(json!({"id": id, "priority": 3})).expect("left out"),
            json!(1_800_000_000_000_i64)
        );
        assert_eq!(
            plan(json!({"id": id, "target_ms": null})).expect("clear"),
            Value::Null
        );
        assert!(matches!(
            plan(json!({"id": id, "target_ms": -1})),
            Err(CallError::Board(BoardError::Invalid(_)))
        ));
    }

    #[test]
    fn a_split_card_names_its_parent_and_the_trace_rolls_it_up() {
        let board: Shared = Board::new(Box::new(MemStore::default()), Roles::default());
        let human = ident("nano", "human");
        let builder = ident("claude", "builder");
        let call = |who: &Identity, op: &str, args: Value| call_as(&board, Some(who), op, args);
        let id = |v: &Value| v["card"]["id"].as_str().expect("id").to_owned();
        let arc = call(&human, "card_add", json!({"title": "arc 2"})).expect("add");
        // A builder has no Triage, but records the parent when it splits work.
        let s2 =
            call(&builder, "card_add", json!({"title": "S2", "parent": "#1"})).expect("add child");
        assert_eq!(s2["card"]["parent"], arc["card"]["id"]);
        let s2a = call(&human, "card_add", json!({"title": "S2a"})).expect("add");
        call(&human, "card_plan", json!({"id": "#3", "parent": "#2"})).expect("set parent");
        assert!(matches!(
            call(&builder, "card_plan", json!({"id": "#3", "parent": null})),
            Err(CallError::Board(BoardError::Forbidden(_)))
        ));
        assert!(
            matches!(
                call(&human, "card_plan", json!({"id": "#1", "parent": "#3"})),
                Err(CallError::Board(BoardError::Invalid(m))) if m.contains("own parent")
            ),
            "a loop is refused"
        );
        assert!(matches!(
            call(
                &human,
                "card_add",
                json!({"title": "x", "parent": Uuid::new_v4()})
            ),
            Err(CallError::Board(BoardError::NotFound(_)))
        ));

        let t = call(&human, "card_trace", json!({"id": id(&s2a)})).expect("trace");
        let titles: Vec<_> = t["ancestors"]
            .as_array()
            .expect("ancestors")
            .iter()
            .map(|c| c["title"].clone())
            .collect();
        assert_eq!(titles, [json!("arc 2"), json!("S2")]);
        let t = call(&human, "card_trace", json!({"id": "#1"})).expect("trace");
        assert_eq!(t["tree"]["rollup"]["total"], 1);
        assert_eq!(t["tree"]["children"][0]["children"][0]["title"], "S2a");

        call(
            &human,
            "card_add",
            json!({"title": "S2b", "parent": "#2", "done": true}),
        )
        .expect("merged piece");
        let t = call(&human, "card_trace", json!({"id": "#2"})).expect("trace");
        assert_eq!(t["tree"]["rollup"]["done"], 1);
        assert_eq!(t["tree"]["rollup"]["ready_to_close"], false);
        call(&human, "card_plan", json!({"id": "#3", "parent": null})).expect("move S2a out");
        let t = call(&human, "card_trace", json!({"id": "#2"})).expect("trace");
        assert_eq!(t["tree"]["rollup"]["ready_to_close"], true);
        let s2 = call(&human, "card_get", json!({"id": "#2"})).expect("get");
        assert_ne!(s2["card"]["column"], "done", "a parent never closes itself");

        let cleared = call(&human, "card_get", json!({"id": "#3"})).expect("get");
        assert_eq!(cleared["card"]["parent"], Value::Null);
    }

    #[test]
    fn a_card_links_to_a_milestone_of_its_project_and_the_milestone_rolls_it_up() {
        use crate::projects::Projects;
        let projects = Projects::parse(r#"{"rl": ["a"], "orch": ["b"]}"#).expect("projects");
        let board: Shared = Board::new(
            Box::new(MemStore::default()) as Box<dyn Store>,
            Roles::default(),
        )
        .with_projects(projects);
        let human = ident("nano", "human");
        let call = |op: &str, args: Value| call_as(&board, Some(&human), op, args);
        let stone = |project: &str| {
            call(
                "milestone_add",
                json!({"project": project, "title": "Beta", "target_ms": 9}),
            )
            .expect("milestone")["milestone"]["id"]
                .clone()
        };
        let (beta, other) = (stone("rl"), stone("orch"));
        call(
            "card_add",
            json!({"title": "a", "project": "rl", "done": true}),
        )
        .expect("add");
        call("card_add", json!({"title": "b", "project": "rl"})).expect("add");
        let linked = call("card_plan", json!({"id": "#1", "milestone": beta})).expect("link");
        assert_eq!(linked["card"]["milestone"], beta);
        call("card_plan", json!({"id": "#2", "milestone": beta})).expect("link");
        assert!(matches!(
            call("card_plan", json!({"id": "#1", "milestone": other})),
            Err(CallError::Board(BoardError::Invalid(_)))
        ));
        assert!(matches!(
            call(
                "card_plan",
                json!({"id": "#1", "milestone": Uuid::new_v4()})
            ),
            Err(CallError::Board(BoardError::NotFound(_)))
        ));
        let progress = |n: usize| {
            call("board_milestones", json!({"project": "rl"})).expect("list")["milestones"][n]
                ["progress"]
                .clone()
        };
        assert_eq!(
            progress(0),
            json!({"total": 2, "done": 1, "dropped": 0, "blocked": 0, "ready_to_close": false})
        );
        call("card_move", json!({"id": "#2", "to": "blocked"})).expect("blocked");
        assert_eq!(
            progress(0),
            json!({"total": 2, "done": 1, "dropped": 0, "blocked": 1, "ready_to_close": false})
        );
        call("card_plan", json!({"id": "#2", "milestone": null})).expect("unlink");
        assert_eq!(progress(0)["ready_to_close"], true);
    }

    #[test]
    fn milestones_are_added_listed_by_scope_and_closed() {
        use crate::projects::Projects;
        let projects = Projects::parse(r#"{"rl": ["a"], "orch": ["b"]}"#).expect("projects");
        let board: Shared = Board::new(
            Box::new(MemStore::default()) as Box<dyn Store>,
            Roles::default(),
        )
        .with_projects(projects);
        let human = ident("nano", "human");
        let add = |project: &str, title: &str, at: i64| {
            call_as(
                &board,
                Some(&human),
                "milestone_add",
                json!({"project": project, "title": title, "target_ms": at}),
            )
        };
        let late = add("rl", "Launch", 3_000).expect("add")["milestone"].clone();
        add("rl", "Beta", 2_000).expect("add");
        add("orch", "Cutover", 1_000).expect("add");
        assert!(matches!(
            add("nope", "x", 1),
            Err(CallError::Board(BoardError::Invalid(_)))
        ));

        let titles = |args: Value| -> Vec<String> {
            call_as(&board, Some(&human), "board_milestones", args).expect("list")["milestones"]
                .as_array()
                .expect("array")
                .iter()
                .map(|m| m["title"].as_str().expect("title").to_owned())
                .collect()
        };
        assert_eq!(
            titles(json!({})),
            ["Cutover", "Beta", "Launch"],
            "by target date"
        );
        assert_eq!(titles(json!({"project": "rl"})), ["Beta", "Launch"]);

        let scoped = Identity {
            projects: Some(vec!["orch".into()]),
            ..ident("dev", "human")
        };
        let seen = call_as(&board, Some(&scoped), "board_milestones", json!({})).expect("list");
        assert_eq!(
            seen["milestones"].as_array().map(Vec::len),
            Some(1),
            "only its projects"
        );
        assert!(matches!(
            call_as(
                &board,
                Some(&scoped),
                "milestone_update",
                json!({"id": late["id"], "done": true})
            ),
            Err(CallError::Board(BoardError::OutOfScope(_)))
        ));

        let closed = call_as(
            &board,
            Some(&human),
            "milestone_update",
            json!({"id": late["id"], "done": true}),
        )
        .expect("close");
        assert!(closed["milestone"]["done_ms"].is_i64());
        let builder = ident("claude", "builder");
        assert!(matches!(
            call_as(
                &board,
                Some(&builder),
                "milestone_add",
                json!({"project": "rl", "title": "x", "target_ms": 1})
            ),
            Err(CallError::Board(BoardError::Forbidden(_)))
        ));
    }
}
