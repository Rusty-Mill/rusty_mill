//! Pure board domain: cards, columns, capabilities and the transition table.
//!
//! No I/O and no clock: callers pass `now_ms`. Every operation takes a card
//! by reference and returns the next card, so an illegal move is an `Err`,
//! never a half-applied mutation.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::external::ExternalRef;
use crate::workflow::{Kind, Transition, Workflow};

/// A board column's id: 1-32 of `a-z 0-9 _`, starting with a letter. It is what cards
/// store (rusty_tick's `board:<id>` tag), so it never changes; people see a column's
/// label ([`ColumnDef`](crate::workflow::ColumnDef)), which an admin may rename.
///
/// Which columns exist, their order and the moves between them are the board's
/// [`Workflow`]. Three ids are fixed because storage relies
/// on them without knowing the workflow: [`Column::BACKLOG`] (where new cards land, and
/// what a card with no or an unknown column reads as), [`Column::DONE`] (a closed task)
/// and [`Column::BLOCKED`].
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(try_from = "String", into = "String")]
#[schemars(with = "String")]
pub struct Column(Cow<'static, str>);

impl Column {
    /// Where new cards land.
    pub const BACKLOG: Self = Self(Cow::Borrowed("backlog"));
    /// Shipped or closed. Terminal.
    pub const DONE: Self = Self(Cow::Borrowed("done"));
    /// Stuck; needs a human. Any open card may be moved here.
    pub const BLOCKED: Self = Self(Cow::Borrowed("blocked"));
    /// The default workflow's queue: approved, waiting for a builder.
    pub const READY: Self = Self(Cow::Borrowed("ready"));
    /// The default workflow's building column.
    pub const BUILDING: Self = Self(Cow::Borrowed("building"));
    /// The default workflow's verification column.
    pub const QA: Self = Self(Cow::Borrowed("qa"));
    /// The default workflow's review column.
    pub const REVIEW: Self = Self(Cow::Borrowed("review"));

    /// Longest id, in bytes.
    pub const MAX_LEN: usize = 32;

    /// The column `id`.
    ///
    /// # Errors
    /// If `id` is not 1-32 of `a-z 0-9 _` starting with a letter.
    pub fn new(id: &str) -> Result<Self, BoardError> {
        let ok = id.len() <= Self::MAX_LEN
            && id.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
            && id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if !ok {
            return Err(BoardError::Invalid(
                "a column id is 1-32 of a-z 0-9 _, starting with a letter",
            ));
        }
        Ok(Self(Cow::Owned(id.to_owned())))
    }

    /// The id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Column {
    type Error = BoardError;
    fn try_from(id: String) -> Result<Self, BoardError> {
        Self::new(&id)
    }
}

impl From<Column> for String {
    fn from(c: Column) -> Self {
        c.0.into_owned()
    }
}

impl fmt::Display for Column {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What an actor is allowed to do. A role is a set of these; the board
/// enforces them, role definitions live elsewhere (orch).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// Approve work and unblock it.
    Triage,
    /// Implement.
    Build,
    /// Write requirements: pick up approved cards and plan them.
    Plan,
    /// Design: turn a card's requirements into an architecture.
    Architect,
    /// Plan tests, write them first, and verify built work. Was `qa`, which stored roles
    /// still say, so it still reads as this.
    #[serde(alias = "qa")]
    Test,
    /// Review code.
    Review,
    /// Merge a reviewed card to Done.
    Merge,
    /// Record as Done a card from Building or QA whose work already merged
    /// outside the board's flow (the PR was reviewed and merged on the forge).
    /// Also archives and restores Done cards.
    Close,
    /// Add a project (never change or remove one).
    Project,
    /// Change the board's settings: agents and their credentials, repositories, projects, roles.
    Admin,
}

impl Capability {
    /// Every capability, in display order.
    pub const ALL: [Capability; 10] = [
        Capability::Triage,
        Capability::Plan,
        Capability::Architect,
        Capability::Build,
        Capability::Test,
        Capability::Review,
        Capability::Merge,
        Capability::Close,
        Capability::Project,
        Capability::Admin,
    ];

    /// What holding this lets a role do, for the settings view.
    pub fn about(self) -> &'static str {
        match self {
            Capability::Triage => {
                "Approve cards into Ready, set priority and project, and take over others' claims."
            }
            Capability::Build => "Claim Ready cards and build them, then pass them to QA.",
            Capability::Plan => "Take approved cards and write their requirements.",
            Capability::Architect => "Design cards whose requirements have passed review.",
            Capability::Test => "Plan and write tests first, then verify built work.",
            Capability::Review => "Send reviewed work back to Building.",
            Capability::Merge => "Move reviewed cards to Done.",
            Capability::Close => {
                "Move Building or QA cards whose PR already merged straight to Done, and archive or restore Done cards."
            }
            Capability::Project => "Add a project (never change or remove one).",
            Capability::Admin => "Change these settings: agents, projects, roles, GitHub, apps.",
        }
    }
}

/// Who is acting, and what they may do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    /// Stable name, recorded in claims and history.
    pub name: String,
    /// Capabilities held.
    pub caps: BTreeSet<Capability>,
    /// Projects this actor may work in; `None` means all of them.
    #[serde(default)]
    pub projects: Option<BTreeSet<String>>,
}

impl Actor {
    /// Whether this actor may read or change cards of `project`.
    pub fn may_work_in(&self, project: &str) -> bool {
        self.projects.as_ref().is_none_or(|p| p.contains(project))
    }
}

/// The project a card belongs to unless one is named. It always exists.
pub const DEFAULT_PROJECT: &str = "default";

fn default_project() -> String {
    DEFAULT_PROJECT.to_owned()
}

/// Whether moves follow the workflow's transitions (the default), or are open: an admin's
/// temporary switch for when the table is in the way.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MoveRules {
    /// The workflow's transitions, their capabilities and the claim rule apply.
    #[default]
    Enforced,
    /// Any column to any other, by any actor holding a capability. Done stays terminal and
    /// a gate still needs its capability and `Admin`.
    Open,
}

/// How long a Done card stays on the board's default views before it is shelved without
/// being archived (see [`Card::shelved`]), in days.
pub const DONE_WINDOW_DAYS: u32 = 14;

/// A half-open range of Unix milliseconds, `[from, to)`; either end may be open.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Span {
    /// At or after, if set.
    pub from: Option<i64>,
    /// Before, if set.
    pub to: Option<i64>,
}

impl Span {
    /// Whether `ms` falls in the range.
    pub fn contains(&self, ms: i64) -> bool {
        self.from.is_none_or(|f| ms >= f) && self.to.is_none_or(|t| ms < t)
    }

    /// Whether either end is set.
    pub fn is_bounded(&self) -> bool {
        self.from.is_some() || self.to.is_some()
    }
}

/// Which cards a listing keeps by date: created, last changed, or moved column in a span.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Dates {
    /// When the card was created.
    pub created: Span,
    /// When the card last changed.
    pub updated: Span,
    /// When the card changed column (entering the board counts).
    pub moved: Span,
}

/// Which cards a listing shows, by whether they are shelved (see [`Card::shelved`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Shelf {
    /// Only cards on the default views: open cards, and Done cards neither archived nor
    /// past the window.
    #[default]
    Exclude,
    /// Every card.
    Include,
    /// Only shelved cards: the archive.
    Only,
}

impl Shelf {
    /// Whether a card shelved (`shelved`) or not belongs in this view.
    pub fn keeps(self, shelved: bool) -> bool {
        match self {
            Shelf::Exclude => !shelved,
            Shelf::Include => true,
            Shelf::Only => shelved,
        }
    }
}

/// The id of the card imported from forge issue `issue`: derived from the number,
/// so an import interrupted between two writes (a store that cannot commit a card
/// and its link together) is finished, not repeated, by the next run.
///
/// The id depends on the number only, not on the repository. A board pointed at a
/// second repository treats that repository's issue `n` as already imported when
/// it has imported `n` from the first, and skips it; give such a repository its
/// own board.
pub fn import_id(issue: u64) -> Uuid {
    const NAMESPACE: Uuid = Uuid::from_u128(0x6d69_6c6c_2d62_6f61_7264_2d69_7373_7565);
    Uuid::new_v5(&NAMESPACE, &issue.to_be_bytes())
}

/// The priorities a card may have, on rusty_tick's scale: none, low, medium, high.
pub const PRIORITIES: [u8; 4] = [0, 1, 3, 5];

/// Most cards one card may wait on; also the most in each of its other link lists.
pub const MAX_DEPS: usize = 32;

/// What an edit changes; each field left as it is when `None`.
#[derive(Debug, Clone, Default)]
pub struct Edit<'a> {
    /// New title.
    pub title: Option<&'a str>,
    /// New description.
    pub body: Option<&'a str>,
    /// New labels (replaces all).
    pub tags: Option<Vec<String>>,
    /// New prompt; `""` clears it.
    pub prompt: Option<&'a str>,
}

/// The longest prompt, in bytes. It is stored in the card's rusty_tick board document,
/// which rusty_tick caps at 64 KiB, beside the links.
pub const MAX_PROMPT_BYTES: usize = 32 * 1024;

/// A planning change (`card_plan`): each `None` field is left as it is, and a list replaces
/// the card's current one (an empty list clears it).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// One of [`PRIORITIES`].
    pub priority: Option<u8>,
    /// Cards that must be Done before this one is taken.
    pub deps: Option<Vec<Uuid>>,
    /// Newer cards that replace this one.
    pub superseded_by: Option<Vec<Uuid>>,
    /// Cards worth reading alongside this one.
    pub related: Option<Vec<Uuid>>,
    /// The card this one is a derived piece of: `Some(None)` clears it.
    pub parent: Option<Option<Uuid>>,
    /// The milestone this card counts towards: `Some(None)` clears it.
    pub milestone: Option<Option<Uuid>>,
    /// When the card should be done, Unix ms: `Some(None)` clears it.
    pub target_ms: Option<Option<i64>>,
}

impl Plan {
    /// Whether the plan changes nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Every card the plan links to, its new parent included.
    pub fn linked(&self) -> impl Iterator<Item = &Uuid> {
        [&self.deps, &self.superseded_by, &self.related]
            .into_iter()
            .flatten()
            .flatten()
            .chain(self.parent.iter().flatten())
    }
}

/// Whether `tag` is one the board keeps for itself in rusty_tick (ADR-0012):
/// `agent` puts a task on the board, `board:<column>`, `claim:<actor>` and
/// `number:<n>` carry its state. Tags match case-insensitively there, so here too.
pub fn reserved_tag(tag: &str) -> bool {
    let tag = tag.trim().to_lowercase();
    tag == "agent"
        || tag.starts_with("board:")
        || tag.starts_with("claim:")
        || tag.starts_with("number:")
        || tag.starts_with("archived:")
        || tag.starts_with("restored:")
}

fn check_tags(tags: &[String]) -> Result<(), BoardError> {
    if tags.iter().any(|t| reserved_tag(t)) {
        return Err(BoardError::Invalid(
            "tags `agent`, `board:…`, `claim:…`, `number:…`, `archived:…` and `restored:…` are the board's own",
        ));
    }
    Ok(())
}

/// A stable 64-bit hash (FNV-1a) of a title and body, for comparing text across
/// sync runs. Stable by construction: it is stored on cards. Insensitive to what
/// forges change in passing: surrounding whitespace and `\r\n` line endings.
pub fn text_hash(title: &str, body: &str) -> u64 {
    let body = body.replace("\r\n", "\n");
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in title
        .trim()
        .bytes()
        .chain([0xff])
        .chain(body.trim_end().bytes())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// A note on a card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    /// Author.
    pub by: String,
    /// Unix milliseconds.
    pub at_ms: i64,
    /// Text.
    pub text: String,
}

/// One card. `rev` increases on every change and is the store's CAS token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Card {
    /// Identity.
    pub id: Uuid,
    /// Short board-wide number people use, shown as `#n`. The board assigns it in creation
    /// order; `None` until then (a card made directly in rusty_tick, before the board numbers it).
    #[serde(default)]
    pub number: Option<u64>,
    /// Optimistic-concurrency revision; `0` for a new card.
    pub rev: u64,
    /// Short title.
    pub title: String,
    /// Longer description (markdown).
    pub body: String,
    /// Instructions for whoever takes the card, kept apart from the description so they can be
    /// read or copied whole. Empty when there are none.
    #[serde(default)]
    pub prompt: String,
    /// Current column.
    pub column: Column,
    /// Who holds the work, if anyone.
    pub claim: Option<String>,
    /// Free-form labels.
    pub tags: Vec<String>,
    /// The project (application) this card belongs to; see `projects`.
    #[serde(default = "default_project")]
    pub project: String,
    /// Discussion, oldest first.
    pub comments: Vec<Comment>,
    /// Higher is taken first within a queue; one of [`PRIORITIES`].
    #[serde(default)]
    pub priority: u8,
    /// Cards that must be Done before this one is taken.
    #[serde(default)]
    pub deps: Vec<Uuid>,
    /// Newer cards that replace this one. While any is set, `take` never hands the card out:
    /// close it, or re-scope it and clear the link.
    #[serde(default)]
    pub superseded_by: Vec<Uuid>,
    /// Cards worth reading alongside this one. Informational: nothing waits on them.
    #[serde(default)]
    pub related: Vec<Uuid>,
    /// The card this one was split from (a decomposition link, not a dependency): at most one,
    /// and never a cycle. Its progress rolls up there; see [`crate::trace`].
    #[serde(default)]
    pub parent: Option<Uuid>,
    /// The milestone this card counts towards (see [`crate::milestones`]), in the card's project
    /// when linked. Separate from `parent`: a card may have both.
    #[serde(default)]
    pub milestone: Option<Uuid>,
    /// Manual order within a queue, lower first, after priority. Set by the store
    /// (rusty_tick's drag order); `0` elsewhere.
    #[serde(default)]
    pub sort_order: i64,
    /// Linked forge issue number, if synced.
    #[serde(default)]
    pub issue: Option<u64>,
    /// [`text_hash`] of the title and body as of the last sync with the linked
    /// issue: the common ancestor that says which side changed the text since.
    #[serde(default)]
    pub synced_text: Option<u64>,
    /// The single recoverable Orch execution this card owns, when factory hosting is enabled.
    /// It is data rather than a provider handle: providers, sessions and processes can change.
    #[serde(default)]
    pub execution: Option<ExecutionLink>,
    /// Execution outside Orch that worked on this card, and its last observed state
    /// (ADR-0006). Observation only: nothing on the board decides anything from it.
    #[serde(default)]
    pub external_refs: Vec<ExternalRef>,
    /// Creation time, Unix ms.
    pub created_ms: i64,
    /// Last change, Unix ms.
    pub updated_ms: i64,
    /// When the card was completed (reached Done as shipped work), Unix ms. `None` while open,
    /// and for work closed without shipping (rusty_tick's won't-do).
    #[serde(default)]
    pub done_ms: Option<i64>,
    /// When the card was archived, Unix ms: a Done card set aside from the board's default
    /// views. It is still Done, so roadmap, metrics and dependencies count it as before.
    #[serde(default)]
    pub archived_ms: Option<i64>,
    /// When the card was last put back from the archive, Unix ms. The Done window counts from
    /// it rather than from `done_ms`, so a restored card stays on the board while its ship time,
    /// and with it the roadmap and metrics, is left as it was.
    #[serde(default)]
    pub restored_ms: Option<i64>,
    /// When the card should be done, Unix ms, if planned: what the roadmap draws ahead of today.
    #[serde(default)]
    pub target_ms: Option<i64>,
}

/// How much of each card a listing returns.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum View {
    /// The whole card: body, prompt, links and comments.
    #[default]
    Full,
    /// [`CardSummary`]: what a board or queue view needs, without body, prompt, links or
    /// comments.
    Summary,
}

/// A card without its body, prompt, links or comments: `board_list` with `view: summary`.
/// Every field comes from the card's rusty_tick task, so a listing of summaries never reads
/// board documents or comments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CardSummary {
    /// The card's id.
    pub id: Uuid,
    /// Its board-wide number, once given one.
    pub number: Option<u64>,
    /// Its revision.
    pub rev: u64,
    /// Short title.
    pub title: String,
    /// Its project.
    pub project: String,
    /// Its column.
    pub column: Column,
    /// Priority: 0 none, 1 low, 3 medium, 5 high.
    pub priority: u8,
    /// Who holds it, if anyone.
    pub claim: Option<String>,
    /// Labels.
    pub tags: Vec<String>,
    /// Creation time, Unix ms.
    pub created_ms: i64,
    /// Last change, Unix ms.
    pub updated_ms: i64,
    /// When it shipped, Unix ms.
    pub done_ms: Option<i64>,
    /// When it was archived, Unix ms.
    pub archived_ms: Option<i64>,
}

impl From<&Card> for CardSummary {
    fn from(c: &Card) -> Self {
        Self {
            id: c.id,
            number: c.number,
            rev: c.rev,
            title: c.title.clone(),
            project: c.project.clone(),
            column: c.column.clone(),
            priority: c.priority,
            claim: c.claim.clone(),
            tags: c.tags.clone(),
            created_ms: c.created_ms,
            updated_ms: c.updated_ms,
            done_ms: c.done_ms,
            archived_ms: c.archived_ms,
        }
    }
}

/// Stable cross-component ids for the one logical execution a card may own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionLink {
    /// Logical execution id, stable through task-host recovery.
    pub execution: String,
    /// Orch goal id.
    pub goal: String,
    /// BBP task id.
    pub bbp_task: String,
    /// Task-host attempt id.
    pub attempt: String,
    /// Provider-neutral configured assignment id.
    pub assignment: String,
    /// Assignment generation that must accompany host commands.
    pub generation: u64,
}

impl ExecutionLink {
    /// Every id is required and generations start at 1.
    fn validate(&self) -> Result<(), BoardError> {
        let ids = [
            &self.execution,
            &self.goal,
            &self.bbp_task,
            &self.attempt,
            &self.assignment,
        ];
        if ids.iter().any(|value| value.is_empty()) || self.generation == 0 {
            return Err(BoardError::Invalid("execution link is incomplete"));
        }
        Ok(())
    }

    /// Whether `other` names the same logical execution, whatever its generation.
    fn same_execution(&self, other: &Self) -> bool {
        self.execution == other.execution
            && self.goal == other.goal
            && self.bbp_task == other.bbp_task
            && self.attempt == other.attempt
            && self.assignment == other.assignment
    }
}

/// Why an operation was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardError {
    /// No card with this id.
    NotFound(Uuid),
    /// Done cards do not move.
    Terminal,
    /// The table has no such transition.
    NoSuchMove {
        /// Source.
        from: Column,
        /// Destination.
        to: Column,
    },
    /// The actor lacks the capability for this move.
    Forbidden(Capability),
    /// Someone else holds the claim.
    Claimed(String),
    /// Claim needed to move out of a working column.
    NotClaimant,
    /// Another writer changed the card first; reload and retry.
    Conflict,
    /// A change stored in two steps kept only its first: the task's fields (title, body,
    /// tags, priority) are saved at revision `rev`, the card's prompt and links are not.
    /// Read the card and apply the rest on `rev`.
    PartlySaved {
        /// The card's revision after the step that was saved.
        rev: u64,
        /// Why the second step failed.
        why: String,
    },
    /// Input rejected at the boundary.
    Invalid(&'static str),
    /// The card belongs to a project this actor may not work in.
    OutOfScope(String),
    /// A settings change was refused, with the reason (it would lock everyone out, drop a
    /// project that still has cards, and so on).
    Refused(String),
    /// The backing store failed.
    Storage(String),
}

impl fmt::Display for BoardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(id) => write!(f, "no card {id}"),
            Self::Terminal => write!(f, "done cards cannot move"),
            Self::NoSuchMove { from, to } => write!(f, "no move {from} -> {to}"),
            Self::Forbidden(c) => write!(f, "requires capability {c:?}"),
            Self::Claimed(by) => write!(f, "claimed by {by}"),
            Self::NotClaimant => write!(f, "only the claim holder may move this card"),
            Self::Conflict => write!(f, "card changed concurrently; retry"),
            Self::PartlySaved { rev, why } => write!(
                f,
                "only part of this change was saved: title, body, tags and priority are at rev \
                 {rev}, but the prompt and links are not ({why}); read the card and save again"
            ),
            Self::Invalid(why) => write!(f, "invalid input: {why}"),
            Self::OutOfScope(p) => write!(f, "project {p:?} is outside your scope"),
            Self::Refused(why) => f.write_str(why),
            Self::Storage(why) => write!(f, "storage error: {why}"),
        }
    }
}

impl std::error::Error for BoardError {}

/// What happened to a card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// Card created.
    Created,
    /// Card moved column.
    Moved,
    /// Claim taken.
    Claimed,
    /// Claim dropped.
    Released,
    /// Title/body/tags edited.
    Edited,
    /// Comment added.
    Commented,
    /// Linked to a forge issue.
    Linked,
    /// A Done card set aside from the default views.
    Archived,
    /// An archived card put back.
    Restored,
    /// Title/body brought in line with the linked issue.
    Synced,
    /// A factory execution was initiated or adopted.
    ExecutionLinked,
    /// A factory execution moved to a replacement assignment generation.
    ExecutionRecovered,
    /// Execution outside Orch was observed (ADR-0006).
    ExecutionObserved,
    /// Moved to another project.
    Assigned,
    /// Given its board-wide number.
    Numbered,
}

/// One entry in the append-only log. `id` is a UUIDv7 assigned by the store
/// *inside* the commit, so ids sort in commit order and double as a polling
/// cursor: a reader that has seen id N has seen every event committed before it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    /// Commit-ordered identity and cursor; nil until the store commits it.
    pub id: Uuid,
    /// Card concerned.
    pub card: Uuid,
    /// Who did it.
    pub actor: String,
    /// What happened.
    pub kind: EventKind,
    /// Card revision after the change.
    pub rev: u64,
    /// Column after the change.
    pub column: Column,
    /// Unix milliseconds.
    pub at_ms: i64,
}

/// The id for the next event: a fresh UUIDv7, forced above `last` so ids are
/// strictly increasing even if the clock stalls or steps back.
pub fn next_event_id(last: Option<Uuid>) -> Uuid {
    let now = Uuid::now_v7();
    match last {
        Some(prev) if now <= prev => Uuid::from_u128(prev.as_u128().wrapping_add(1)),
        _ => now,
    }
}

impl Event {
    /// The event describing `card` just after `kind` happened. Its id is
    /// assigned by the store at commit.
    pub fn of(card: &Card, kind: EventKind, actor: &str) -> Self {
        Self {
            id: Uuid::nil(),
            card: card.id,
            actor: actor.to_owned(),
            kind,
            rev: card.rev,
            column: card.column.clone(),
            at_ms: card.updated_ms,
        }
    }
}

/// Whether `by` may make one of the transitions `ts` (alternatives for the same move).
/// If none, the first one's reason.
fn check_any(ts: &[&Transition], by: &Actor) -> Result<(), BoardError> {
    let mut first = None;
    for t in ts {
        match check_transition(t, by) {
            Ok(()) => return Ok(()),
            Err(e) => {
                first.get_or_insert(e);
            }
        }
    }
    first.map_or(Ok(()), Err)
}

/// Whether `by` may make transition `t`: its capability, and `Admin` too for a gate.
fn check_transition(t: &Transition, by: &Actor) -> Result<(), BoardError> {
    if !by.caps.contains(&t.cap) {
        return Err(BoardError::Forbidden(t.cap));
    }
    if t.gate && !by.caps.contains(&Capability::Admin) {
        return Err(BoardError::Forbidden(Capability::Admin));
    }
    Ok(())
}

impl Card {
    /// A new Backlog card.
    ///
    /// # Errors
    /// If `title` is blank or a tag is reserved ([`reserved_tag`]).
    pub fn new(
        title: &str,
        body: &str,
        tags: Vec<String>,
        now_ms: i64,
    ) -> Result<Self, BoardError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(BoardError::Invalid("title is empty"));
        }
        check_tags(&tags)?;
        Ok(Self {
            id: Uuid::new_v4(),
            number: None,
            rev: 0,
            title: title.to_owned(),
            body: body.to_owned(),
            prompt: String::new(),
            column: Column::BACKLOG,
            claim: None,
            tags,
            project: default_project(),
            comments: Vec::new(),
            priority: 0,
            deps: Vec::new(),
            superseded_by: Vec::new(),
            related: Vec::new(),
            parent: None,
            milestone: None,
            sort_order: 0,
            issue: None,
            synced_text: None,
            execution: None,
            external_refs: Vec::new(),
            created_ms: now_ms,
            updated_ms: now_ms,
            done_ms: None,
            archived_ms: None,
            restored_ms: None,
            target_ms: None,
        })
    }

    /// The card given its board-wide `number`, or unchanged if it has one already: a number
    /// never changes. Not a change people make, so `updated_ms` stays.
    #[must_use]
    pub fn numbered(&self, number: u64) -> Self {
        if self.number.is_some() {
            return self.clone();
        }
        Self {
            number: Some(number),
            rev: self.rev + 1,
            ..self.clone()
        }
    }

    pub(crate) fn next(&self, now_ms: i64) -> Self {
        Self {
            rev: self.rev + 1,
            updated_ms: now_ms,
            ..self.clone()
        }
    }

    /// Re-file the card under column `to` when the workflow's columns change (a retag, not a
    /// move): no transition, rule or claim check, and the claim is kept, since the work is the
    /// same work in a renamed stage. Callers keep Done and Blocked out of it, as
    /// [`crate::retag::parse_map`] does.
    pub fn retag(&self, to: &Column, now_ms: i64) -> Self {
        Self {
            column: to.clone(),
            ..self.next(now_ms)
        }
    }

    /// Move the card to `project`, dropping any claim (the claimant may not be in scope there).
    /// Only the project changes: the column stays, so a Done card is re-filed, never reopened.
    pub fn assign(&self, project: &str, now_ms: i64) -> Self {
        Self {
            project: project.to_owned(),
            claim: None,
            ..self.next(now_ms)
        }
    }

    /// Take the claim.
    ///
    /// # Errors
    /// If another actor already holds it.
    pub fn claim(&self, by: &Actor, now_ms: i64) -> Result<Self, BoardError> {
        match &self.claim {
            Some(holder) if *holder != by.name => Err(BoardError::Claimed(holder.clone())),
            _ => Ok(Self {
                claim: Some(by.name.clone()),
                ..self.next(now_ms)
            }),
        }
    }

    /// Drop the claim; only its holder may, or anyone holding `Triage`
    /// (a person taking a card back from a stuck or runaway agent).
    ///
    /// # Errors
    /// If someone else holds it.
    pub fn release(&self, by: &Actor, now_ms: i64) -> Result<Self, BoardError> {
        match &self.claim {
            Some(holder) if *holder != by.name && !by.caps.contains(&Capability::Triage) => {
                Err(BoardError::Claimed(holder.clone()))
            }
            _ => Ok(Self {
                claim: None,
                ..self.next(now_ms)
            }),
        }
    }

    /// Move to `to` along `flow`. The claim is cleared on arrival, so the next stage's
    /// worker claims it afresh. Leaving a worked column ([`Kind::Work`]) requires holding
    /// the claim, unless the actor holds `Triage` (the human override).
    ///
    /// # Errors
    /// If the move does not exist, the actor lacks its capability (or `Admin`, for a gate),
    /// or the actor is not the claim holder.
    pub fn move_to(
        &self,
        flow: &Workflow,
        to: &Column,
        by: &Actor,
        now_ms: i64,
    ) -> Result<Self, BoardError> {
        if let Some(ts) = flow.transition(&self.column, to)? {
            check_any(&ts, by)?;
        }
        if flow.kind(&self.column) == Some(Kind::Work)
            && self.claim.as_deref() != Some(&by.name)
            && !by.caps.contains(&Capability::Triage)
        {
            return Err(BoardError::NotClaimant);
        }
        Ok(self.arrive(to, now_ms))
    }

    /// Move to `to` with the movement rules off: no transition table, capability or claim
    /// check, only an actor holding some capability, except that gates hold: a listed move
    /// with a gated alternative needs one of its alternatives, and a move that jumps over a
    /// gate ([`Workflow::gates_crossed`]) needs `Admin`, or, when the workflow lists it, the
    /// capability of one of its ways (Building -> Done by `close`). The claim is cleared on
    /// arrival.
    ///
    /// # Errors
    /// If the card is Done, already in `to`, `to` is not a column, the actor holds no
    /// capability, or the move is a gate the actor may not pass.
    pub fn move_freely(
        &self,
        flow: &Workflow,
        to: &Column,
        by: &Actor,
        now_ms: i64,
    ) -> Result<Self, BoardError> {
        if self.column == Column::DONE {
            return Err(BoardError::Terminal);
        }
        if self.column == *to {
            return Err(BoardError::NoSuchMove {
                from: to.clone(),
                to: to.clone(),
            });
        }
        if flow.column(to).is_none() {
            return Err(BoardError::Invalid("no such column"));
        }
        if by.caps.is_empty() {
            return Err(BoardError::Invalid("moving a card needs some capability"));
        }
        // Gates hold with the rules off. A listed move with a gated way of making it needs one
        // of its ways (a gated merge, or the `close` beside it); a move that jumps over a gate
        // needs `Admin`, or one of its listed ways.
        let listed = flow.transition(&self.column, to).ok().flatten();
        match listed {
            Some(ts) if ts.iter().any(|t| t.gate) => check_any(&ts, by)?,
            _ if by.caps.contains(&Capability::Admin)
                || flow.gates_crossed(&self.column, to).is_empty() => {}
            // A listed shortcut past a gate (Building -> Done by `close`) keeps its capability.
            Some(ts) => check_any(&ts, by)?,
            None => return Err(BoardError::Forbidden(Capability::Admin)),
        }
        Ok(self.arrive(to, now_ms))
    }

    fn arrive(&self, to: &Column, now_ms: i64) -> Self {
        Self {
            column: to.clone(),
            claim: None,
            done_ms: (*to == Column::DONE).then_some(now_ms),
            ..self.next(now_ms)
        }
    }

    /// Archive the card: set it aside from the default views. Unchanged if already archived.
    ///
    /// # Errors
    /// [`BoardError::Invalid`] if the card is not Done: archiving tidies finished work and
    /// never closes open work.
    pub fn archive(&self, now_ms: i64) -> Result<Self, BoardError> {
        if self.column != Column::DONE {
            return Err(BoardError::Invalid("only done cards can be archived"));
        }
        if self.archived_ms.is_some() {
            return Ok(self.clone());
        }
        Ok(Self {
            archived_ms: Some(now_ms),
            restored_ms: None,
            ..self.next(now_ms)
        })
    }

    /// Put a shelved card (see [`Card::shelved`]) back on the default views: no longer archived,
    /// and its Done window starts again at `now_ms`. Unchanged if it is not shelved.
    #[must_use]
    pub fn restore(&self, now_ms: i64, window_ms: Option<i64>) -> Self {
        if !self.shelved(now_ms, window_ms) {
            return self.clone();
        }
        Self {
            archived_ms: None,
            restored_ms: Some(now_ms),
            ..self.next(now_ms)
        }
    }

    /// Where a Done card's window starts: its ship time (or, closed without shipping, its last
    /// change), or its last restore if later.
    pub fn shelf_since_ms(&self) -> i64 {
        let closed_ms = self.done_ms.unwrap_or(self.updated_ms);
        self.restored_ms.map_or(closed_ms, |r| r.max(closed_ms))
    }

    /// Whether the card is off the board's default views at `now_ms`: a Done card that is
    /// archived, or whose window ([`Card::shelf_since_ms`]) is older than `window_ms`, when
    /// there is a window.
    pub fn shelved(&self, now_ms: i64, window_ms: Option<i64>) -> bool {
        if self.column != Column::DONE {
            return false;
        }
        if self.archived_ms.is_some() {
            return true;
        }
        window_ms.is_some_and(|w| now_ms.saturating_sub(self.shelf_since_ms()) > w)
    }

    /// Replace title/body/tags (each optional).
    ///
    /// # Errors
    /// If the new title is blank or a tag is reserved.
    pub fn edit(
        &self,
        title: Option<&str>,
        body: Option<&str>,
        tags: Option<Vec<String>>,
        now_ms: i64,
    ) -> Result<Self, BoardError> {
        let mut next = self.next(now_ms);
        if let Some(t) = title {
            if t.trim().is_empty() {
                return Err(BoardError::Invalid("title is empty"));
            }
            next.title = t.trim().to_owned();
        }
        if let Some(b) = body {
            next.body = b.to_owned();
        }
        if let Some(t) = tags {
            check_tags(&t)?;
            next.tags = t;
        }
        Ok(next)
    }

    /// Replace the prompt; an empty one clears it.
    ///
    /// # Errors
    /// A prompt over [`MAX_PROMPT_BYTES`].
    pub fn with_prompt(mut self, prompt: &str) -> Result<Self, BoardError> {
        if prompt.len() > MAX_PROMPT_BYTES {
            return Err(BoardError::Invalid("prompt is longer than 32 KiB"));
        }
        self.prompt = prompt.to_owned();
        Ok(self)
    }

    /// Apply a [`Plan`]: priority, links and target date, each left as it is when `None`. Repeated ids
    /// in a list are dropped.
    ///
    /// # Errors
    /// A priority outside [`PRIORITIES`], a card linking to itself, more than
    /// [`MAX_DEPS`] ids in one list, or a target that is not a positive time.
    pub fn plan(&self, plan: &Plan, now_ms: i64) -> Result<Self, BoardError> {
        let mut next = self.next(now_ms);
        if let Some(p) = plan.priority {
            if !PRIORITIES.contains(&p) {
                return Err(BoardError::Invalid("priority must be 0, 1, 3 or 5"));
            }
            next.priority = p;
        }
        if let Some(d) = &plan.deps {
            next.deps = self.links(d)?;
        }
        if let Some(s) = &plan.superseded_by {
            next.superseded_by = self.links(s)?;
        }
        if let Some(r) = &plan.related {
            next.related = self.links(r)?;
        }
        if let Some(p) = plan.parent {
            if p == Some(self.id) {
                return Err(BoardError::Invalid("a card cannot be its own parent"));
            }
            next.parent = p;
        }
        if let Some(m) = plan.milestone {
            next.milestone = m;
        }
        if let Some(t) = plan.target_ms {
            if t.is_some_and(|ms| ms <= 0) {
                return Err(BoardError::Invalid(
                    "target_ms must be a positive Unix time in ms",
                ));
            }
            next.target_ms = t;
        }
        Ok(next)
    }

    /// `ids` sorted and deduplicated, as one of this card's link lists.
    fn links(&self, ids: &[Uuid]) -> Result<Vec<Uuid>, BoardError> {
        if ids.contains(&self.id) {
            return Err(BoardError::Invalid("a card cannot link to itself"));
        }
        let mut ids = ids.to_vec();
        ids.sort_unstable();
        ids.dedup();
        if ids.len() > MAX_DEPS {
            return Err(BoardError::Invalid("too many linked cards"));
        }
        Ok(ids)
    }

    /// Record the forge issue this card is synced with, and the text both
    /// sides agreed on at that moment.
    pub fn link(&self, issue: u64, synced_text: Option<u64>, now_ms: i64) -> Self {
        Self {
            issue: Some(issue),
            synced_text,
            ..self.next(now_ms)
        }
    }

    /// Record that title and body match the linked issue (hash `synced_text`).
    pub fn mark_synced(&self, synced_text: u64, now_ms: i64) -> Self {
        Self {
            synced_text: Some(synced_text),
            ..self.next(now_ms)
        }
    }

    /// Bind the card to one logical execution. Replaying the same durable link
    /// adopts it; a different link is refused so duplicate submitters cannot fork work.
    pub fn bind_execution(&self, link: ExecutionLink, now_ms: i64) -> Result<Self, BoardError> {
        link.validate()?;
        match &self.execution {
            Some(existing) if existing == &link => Ok(self.clone()),
            Some(_) => Err(BoardError::Refused(
                "card already has a logical execution".into(),
            )),
            None => Ok(Self {
                execution: Some(link),
                ..self.next(now_ms)
            }),
        }
    }

    /// Hand the card's execution to a replacement assignment generation.
    ///
    /// A compare-and-set on the generation: the stored link must still carry
    /// `expected_generation`, `link` must name the same logical execution
    /// (execution, goal, BBP task, attempt, assignment), and its generation
    /// must advance. Replaying a recovery that already landed returns the card
    /// unchanged; a stale actor presenting an older generation is refused and
    /// can never overwrite the newer link.
    pub fn recover_execution(
        &self,
        expected_generation: u64,
        link: ExecutionLink,
        now_ms: i64,
    ) -> Result<Self, BoardError> {
        link.validate()?;
        let Some(existing) = &self.execution else {
            return Err(BoardError::Refused(
                "card has no execution to recover".into(),
            ));
        };
        if existing == &link {
            return Ok(self.clone());
        }
        if !existing.same_execution(&link) {
            return Err(BoardError::Refused(
                "card already has a logical execution".into(),
            ));
        }
        if existing.generation != expected_generation {
            return Err(BoardError::Refused(format!(
                "execution generation is {}, not {expected_generation}",
                existing.generation
            )));
        }
        if link.generation <= expected_generation {
            return Err(BoardError::Invalid(
                "recovery must advance the execution generation",
            ));
        }
        Ok(Self {
            execution: Some(link),
            ..self.next(now_ms)
        })
    }

    /// Append a comment.
    ///
    /// # Errors
    /// If `text` is blank.
    pub fn comment(&self, by: &Actor, text: &str, now_ms: i64) -> Result<Self, BoardError> {
        if text.trim().is_empty() {
            return Err(BoardError::Invalid("comment is empty"));
        }
        let mut next = self.next(now_ms);
        next.comments.push(Comment {
            by: by.name.clone(),
            at_ms: now_ms,
            text: text.to_owned(),
        });
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actor(name: &str, caps: &[Capability]) -> Actor {
        Actor {
            name: name.into(),
            caps: caps.iter().copied().collect(),
            projects: None,
        }
    }

    fn flow() -> Workflow {
        Workflow::default()
    }

    fn card() -> Card {
        Card::new("t", "", vec![], 0).expect("valid")
    }

    #[test]
    fn reaching_done_stamps_the_ship_time() {
        let human = actor("nano", &[Capability::Triage, Capability::Merge]);
        let c = card()
            .move_to(&flow(), &Column::READY, &human, 1)
            .expect("triage");
        assert_eq!(c.done_ms, None);
        let c = c
            .move_freely(&flow(), &Column::DONE, &human, 7)
            .expect("done");
        assert_eq!(c.done_ms, Some(7));
    }

    #[test]
    fn blank_title_rejected() {
        assert_eq!(
            Card::new("  ", "", vec![], 0),
            Err(BoardError::Invalid("title is empty"))
        );
    }

    #[test]
    fn happy_path_backlog_to_done() {
        let human = actor("nano", &[Capability::Triage, Capability::Merge]);
        let dev = actor("dev", &[Capability::Build]);
        let qa = actor("qa", &[Capability::Test]);
        let rev = actor("rev", &[Capability::Review, Capability::Merge]);

        let c = card()
            .move_to(&flow(), &Column::READY, &human, 1)
            .expect("triage");
        let c = c
            .move_to(&flow(), &Column::BUILDING, &dev, 2)
            .expect("start");
        let c = c.claim(&dev, 3).expect("claim");
        let c = c.move_to(&flow(), &Column::QA, &dev, 4).expect("to qa");
        let c = c.claim(&qa, 5).expect("claim");
        let c = c
            .move_to(&flow(), &Column::REVIEW, &qa, 6)
            .expect("to review");
        let c = c.claim(&rev, 7).expect("claim");
        let c = c.move_to(&flow(), &Column::DONE, &rev, 8).expect("merge");
        assert_eq!(c.column, Column::DONE);
        assert_eq!(c.claim, None);
        assert_eq!(c.rev, 8);
    }

    #[test]
    fn capability_is_enforced() {
        let dev = actor("dev", &[Capability::Build]);
        assert_eq!(
            card().move_to(&flow(), &Column::READY, &dev, 1),
            Err(BoardError::Forbidden(Capability::Triage))
        );
    }

    #[test]
    fn unknown_and_terminal_moves_rejected() {
        let all = actor(
            "a",
            &[Capability::Triage, Capability::Build, Capability::Merge],
        );
        assert_eq!(
            card().move_to(&flow(), &Column::DONE, &all, 1),
            Err(BoardError::NoSuchMove {
                from: Column::BACKLOG,
                to: Column::DONE
            })
        );
        let mut done = card();
        done.column = Column::DONE;
        assert_eq!(
            done.move_to(&flow(), &Column::READY, &all, 1),
            Err(BoardError::Terminal)
        );
    }

    #[test]
    fn open_rules_allow_any_move_but_keep_done_terminal() {
        let builder = actor("claude-cloud", &[Capability::Build]);
        let mut c = card();
        c.column = Column::REVIEW;
        c.claim = Some("someone-else".into());
        let c = c
            .move_freely(&flow(), &Column::READY, &builder, 1)
            .expect("out of review");
        assert_eq!((&c.column, c.claim.as_deref()), (&Column::READY, None));
        let c = c
            .move_freely(&flow(), &Column::DONE, &builder, 2)
            .expect("ready to done");
        assert_eq!(
            c.move_freely(&flow(), &Column::BACKLOG, &builder, 3),
            Err(BoardError::Terminal)
        );
    }

    #[test]
    fn open_rules_refuse_a_same_column_move_and_an_actor_without_capabilities() {
        let builder = actor("b", &[Capability::Build]);
        assert_eq!(
            card().move_freely(&flow(), &Column::BACKLOG, &builder, 1),
            Err(BoardError::NoSuchMove {
                from: Column::BACKLOG,
                to: Column::BACKLOG
            })
        );
        assert_eq!(
            card().move_freely(&flow(), &Column::READY, &actor("none", &[]), 1),
            Err(BoardError::Invalid("moving a card needs some capability"))
        );
    }

    #[test]
    fn working_column_requires_claim() {
        let dev = actor("dev", &[Capability::Build]);
        let other = actor("other", &[Capability::Build]);
        let mut c = card();
        c.column = Column::BUILDING;
        assert_eq!(
            c.move_to(&flow(), &Column::QA, &dev, 1),
            Err(BoardError::NotClaimant)
        );
        let c = c.claim(&dev, 1).expect("claim");
        assert_eq!(c.claim(&other, 2), Err(BoardError::Claimed("dev".into())));
        assert_eq!(
            c.move_to(&flow(), &Column::QA, &other, 2),
            Err(BoardError::NotClaimant)
        );
        assert_eq!(c.release(&other, 2), Err(BoardError::Claimed("dev".into())));
        assert!(c.release(&dev, 2).expect("release").claim.is_none());
    }

    #[test]
    fn close_records_merged_work_as_done() {
        let dev = actor("dev", &[Capability::Build]);
        let closer = actor("closer", &[Capability::Build, Capability::Close]);
        for from in [Column::BUILDING, Column::QA] {
            let mut c = card();
            c.column = from;
            let held = c.claim(&dev, 1).expect("claim");
            assert_eq!(
                held.move_to(&flow(), &Column::DONE, &dev, 2),
                Err(BoardError::Forbidden(Capability::Close))
            );
            assert_eq!(
                held.move_to(&flow(), &Column::DONE, &closer, 2),
                Err(BoardError::NotClaimant),
                "close still needs the claim"
            );
            let c = c.claim(&closer, 1).expect("claim");
            assert_eq!(
                c.move_to(&flow(), &Column::DONE, &closer, 2)
                    .expect("close")
                    .column,
                Column::DONE
            );
        }
        for from in [Column::BACKLOG, Column::READY, Column::BLOCKED] {
            let mut c = card();
            c.column = from.clone();
            assert_eq!(
                c.move_to(&flow(), &Column::DONE, &closer, 1),
                Err(BoardError::NoSuchMove {
                    from,
                    to: Column::DONE
                }),
                "only started work can be closed"
            );
        }
    }

    #[test]
    fn triage_holder_overrides_a_claim() {
        let dev = actor("dev", &[Capability::Build]);
        let human = actor("nano", &[Capability::Triage]);
        let mut c = card();
        c.column = Column::BUILDING;
        let c = c.claim(&dev, 1).expect("claim");
        assert!(
            c.release(&human, 2)
                .expect("override release")
                .claim
                .is_none()
        );
        assert_eq!(
            c.move_to(&flow(), &Column::BLOCKED, &human, 2)
                .expect("override move")
                .column,
            Column::BLOCKED
        );
        let other = actor("other", &[Capability::Build]);
        assert_eq!(
            c.move_to(&flow(), &Column::BLOCKED, &other, 2),
            Err(BoardError::NotClaimant)
        );
    }

    #[test]
    fn anyone_may_block() {
        let nobody = actor("n", &[]);
        let c = card()
            .move_to(&flow(), &Column::BLOCKED, &nobody, 1)
            .expect("block");
        assert_eq!(c.column, Column::BLOCKED);
    }

    #[test]
    fn text_hash_is_stable_and_separates_title_from_body() {
        assert_eq!(text_hash("a", "b"), text_hash("a", "b"));
        assert_ne!(
            text_hash("ab", ""),
            text_hash("a", "b"),
            "the boundary matters"
        );
        assert_ne!(text_hash("a", "b"), text_hash("a", "c"));
        // Forges add trailing newlines and CRLFs; neither is a change.
        assert_eq!(text_hash(" a ", "x\r\ny\n\n"), text_hash("a", "x\ny"));
        // Pinned (values from an independent Python FNV-1a): the hash is persisted on
        // cards, so it must never change.
        assert_eq!(text_hash("", ""), 0xaf64_724c_8602_eb6e);
        assert_eq!(text_hash("ab", "cd"), 0x20ba_0030_25a7_acc0);
    }

    #[test]
    fn event_ids_are_strictly_increasing_even_when_the_clock_is_not() {
        let far_future = Uuid::from_u128(u128::MAX >> 8);
        let next = next_event_id(Some(far_future));
        assert!(next > far_future, "forced above the previous id");
        let mut last = None;
        for _ in 0..1000 {
            let id = next_event_id(last);
            assert!(last.is_none_or(|l| id > l));
            last = Some(id);
        }
    }

    #[test]
    fn link_sets_issue_and_bumps_rev() {
        let c = card().link(7, Some(99), 3);
        assert_eq!(
            (c.issue, c.synced_text, c.rev, c.updated_ms),
            (Some(7), Some(99), 1, 3)
        );
        assert_eq!(c.mark_synced(5, 4).synced_text, Some(5));
    }

    #[test]
    fn execution_link_is_idempotent_but_never_replaced() {
        let link = ExecutionLink {
            execution: "execution".into(),
            goal: "goal".into(),
            bbp_task: "bbp".into(),
            attempt: "attempt".into(),
            assignment: "project-orch".into(),
            generation: 1,
        };
        let c = card().bind_execution(link.clone(), 1).expect("bind");
        assert_eq!(c.bind_execution(link.clone(), 2).expect("adopt"), c);
        let mut replacement = link;
        replacement.attempt = "another".into();
        assert!(matches!(
            c.bind_execution(replacement, 2),
            Err(BoardError::Refused(_))
        ));
    }

    #[test]
    fn execution_recovery_is_a_generation_compare_and_set() {
        let link = ExecutionLink {
            execution: "execution".into(),
            goal: "goal".into(),
            bbp_task: "bbp".into(),
            attempt: "attempt".into(),
            assignment: "project-orch".into(),
            generation: 1,
        };
        let at = |generation| ExecutionLink {
            generation,
            ..link.clone()
        };
        assert!(matches!(
            card().recover_execution(1, at(2), 1),
            Err(BoardError::Refused(_))
        ));
        let c = card().bind_execution(link.clone(), 1).expect("bind");
        let mut other = at(2);
        other.attempt = "another".into();
        assert!(matches!(
            c.recover_execution(1, other, 2),
            Err(BoardError::Refused(_))
        ));
        let recovered = c.recover_execution(1, at(2), 2).expect("recover");
        assert_eq!(recovered.execution, Some(at(2)));
        assert_eq!(recovered.rev, c.rev + 1);
        assert_eq!(
            recovered.recover_execution(2, at(1), 3),
            Err(BoardError::Invalid(
                "recovery must advance the execution generation"
            ))
        );
        // Replay is a no-op; the stale generation can neither recover nor bind.
        assert_eq!(
            recovered.recover_execution(1, at(2), 3),
            Ok(recovered.clone())
        );
        assert!(matches!(
            recovered.recover_execution(1, at(3), 3),
            Err(BoardError::Refused(_))
        ));
        assert!(matches!(
            recovered.bind_execution(link, 3),
            Err(BoardError::Refused(_))
        ));
    }

    #[test]
    fn cards_stored_before_issue_links_still_load() {
        let mut v = serde_json::to_value(card()).expect("json");
        v.as_object_mut().expect("object").remove("issue");
        let c: Card = serde_json::from_value(v).expect("decode");
        assert_eq!(c.issue, None);
    }

    #[test]
    fn edit_and_comment_bump_rev() {
        let a = actor("a", &[]);
        let c = card()
            .edit(Some("new"), None, Some(vec!["x".into()]), 5)
            .expect("edit");
        assert_eq!((c.title.as_str(), c.rev, c.updated_ms), ("new", 1, 5));
        let c = c.comment(&a, "hi", 6).expect("comment");
        assert_eq!((c.comments.len(), c.rev), (1, 2));
        assert!(c.comment(&a, " ", 7).is_err());
        assert!(c.edit(Some(" "), None, None, 7).is_err());
    }
}
