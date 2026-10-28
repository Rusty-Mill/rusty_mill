//! Pure board domain: cards, columns, capabilities and the transition table.
//!
//! No I/O and no clock: callers pass `now_ms`. Every operation takes a card
//! by reference and returns the next card, so an illegal move is an `Err`,
//! never a half-applied mutation.

use std::collections::BTreeSet;
use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A board column, in workflow order (the order mirrors super-board's).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Column {
    /// Collected ideas, not yet approved.
    Backlog,
    /// Approved and waiting for a builder.
    Ready,
    /// Being implemented.
    Building,
    /// Being verified.
    Qa,
    /// Awaiting review and merge.
    Review,
    /// Merged. Terminal.
    Done,
    /// Stuck; needs a human.
    Blocked,
}

impl Column {
    /// All columns, in display order.
    pub const ALL: [Column; 7] = [
        Column::Backlog,
        Column::Ready,
        Column::Building,
        Column::Qa,
        Column::Review,
        Column::Done,
        Column::Blocked,
    ];
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
    /// Verify.
    Qa,
    /// Review code.
    Review,
    /// Merge a reviewed card to Done.
    Merge,
    /// Record as Done a card from Building or QA whose work already merged
    /// outside the board's flow (the PR was reviewed and merged on the forge).
    Close,
    /// Add a project (never change or remove one).
    Project,
    /// Change the board's settings: agents and their credentials, repositories, projects, roles.
    Admin,
}

impl Capability {
    /// Every capability, in display order.
    pub const ALL: [Capability; 8] = [
        Capability::Triage,
        Capability::Build,
        Capability::Qa,
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
            Capability::Qa => "Verify built work: pass it to Review or send it back.",
            Capability::Review => "Send reviewed work back to Building.",
            Capability::Merge => "Move reviewed cards to Done.",
            Capability::Close => {
                "Move Building or QA cards whose PR already merged straight to Done."
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

/// The capability needed to move a card `from` -> `to`.
/// `Ok(None)` means any actor may; `Err` means the move does not exist.
pub fn required_capability(from: Column, to: Column) -> Result<Option<Capability>, BoardError> {
    use Column::*;
    match (from, to) {
        (Done, _) => Err(BoardError::Terminal),
        (_, Blocked) => Ok(None),
        (Backlog, Ready) | (Blocked, Ready) => Ok(Some(Capability::Triage)),
        (Ready, Building) | (Building, Qa) => Ok(Some(Capability::Build)),
        (Qa, Building) | (Qa, Review) => Ok(Some(Capability::Qa)),
        (Review, Building) => Ok(Some(Capability::Review)),
        (Review, Done) => Ok(Some(Capability::Merge)),
        (Building, Done) | (Qa, Done) => Ok(Some(Capability::Close)),
        _ => Err(BoardError::NoSuchMove { from, to }),
    }
}

/// Whether moves follow [`required_capability`] (the default), or are open: an admin's
/// temporary switch for when the table is in the way.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MoveRules {
    /// The transition table, its capabilities and the claim rule apply.
    #[default]
    Enforced,
    /// Any column to any other, by any actor holding a capability. Done stays terminal.
    Open,
}

/// Columns where work is picked up, in the order a worker should look:
/// finish what is nearest Done first.
pub const WORK_QUEUES: [Column; 4] = [Column::Review, Column::Qa, Column::Building, Column::Ready];

/// The column a worker moves a card into before working it, if it waits in a
/// pure queue (Ready) rather than a working column.
pub fn entry_column(column: Column) -> Option<Column> {
    (column == Column::Ready).then_some(Column::Building)
}

/// Whether someone holding `caps` can move a card forward out of `column`
/// (the test for "this card is for my role"). `Close` records work finished
/// elsewhere, so it never makes a card someone's work.
pub fn actionable(column: Column, caps: &BTreeSet<Capability>) -> bool {
    Column::ALL.iter().any(|to| {
        *to != Column::Blocked
            && matches!(required_capability(column, *to),
                Ok(Some(cap)) if cap != Capability::Close && caps.contains(&cap))
    })
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

/// Most cards one card may wait on.
pub const MAX_DEPS: usize = 32;

/// Whether `tag` is one the board keeps for itself in rusty_tick (ADR-0012):
/// `agent` puts a task on the board, `board:<column>` and `claim:<actor>` carry
/// its state. Tags match case-insensitively there, so here too.
pub fn reserved_tag(tag: &str) -> bool {
    let tag = tag.trim().to_lowercase();
    tag == "agent" || tag.starts_with("board:") || tag.starts_with("claim:")
}

fn check_tags(tags: &[String]) -> Result<(), BoardError> {
    if tags.iter().any(|t| reserved_tag(t)) {
        return Err(BoardError::Invalid(
            "tags `agent`, `board:…` and `claim:…` are the board's own",
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
    /// Optimistic-concurrency revision; `0` for a new card.
    pub rev: u64,
    /// Short title.
    pub title: String,
    /// Longer description (markdown).
    pub body: String,
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
    /// Creation time, Unix ms.
    pub created_ms: i64,
    /// Last change, Unix ms.
    pub updated_ms: i64,
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
            Self::NoSuchMove { from, to } => write!(f, "no move {from:?} -> {to:?}"),
            Self::Forbidden(c) => write!(f, "requires capability {c:?}"),
            Self::Claimed(by) => write!(f, "claimed by {by}"),
            Self::NotClaimant => write!(f, "only the claim holder may move this card"),
            Self::Conflict => write!(f, "card changed concurrently; retry"),
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
    /// Title/body brought in line with the linked issue.
    Synced,
    /// A factory execution was initiated or adopted.
    ExecutionLinked,
    /// A factory execution moved to a replacement assignment generation.
    ExecutionRecovered,
    /// Moved to another project.
    Assigned,
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
            column: card.column,
            at_ms: card.updated_ms,
        }
    }
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
            rev: 0,
            title: title.to_owned(),
            body: body.to_owned(),
            column: Column::Backlog,
            claim: None,
            tags,
            project: default_project(),
            comments: Vec::new(),
            priority: 0,
            deps: Vec::new(),
            sort_order: 0,
            issue: None,
            synced_text: None,
            execution: None,
            created_ms: now_ms,
            updated_ms: now_ms,
        })
    }

    fn next(&self, now_ms: i64) -> Self {
        Self {
            rev: self.rev + 1,
            updated_ms: now_ms,
            ..self.clone()
        }
    }

    /// Move the card to `project`, dropping any claim (the claimant may not be in scope there).
    ///
    /// # Errors
    /// If the card is Done.
    pub fn assign(&self, project: &str, now_ms: i64) -> Result<Self, BoardError> {
        if self.column == Column::Done {
            return Err(BoardError::Terminal);
        }
        Ok(Self {
            project: project.to_owned(),
            claim: None,
            ..self.next(now_ms)
        })
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

    /// Move to `to`. The claim is cleared on arrival, so the next stage's
    /// worker claims it afresh. Leaving a worked column (Building, Qa, Review)
    /// requires holding the claim, unless the actor holds `Triage` (the
    /// human override).
    ///
    /// # Errors
    /// If the move does not exist, the actor lacks the capability, or the
    /// actor is not the claim holder.
    pub fn move_to(&self, to: Column, by: &Actor, now_ms: i64) -> Result<Self, BoardError> {
        if let Some(cap) = required_capability(self.column, to)?
            && !by.caps.contains(&cap)
        {
            return Err(BoardError::Forbidden(cap));
        }
        let worked = matches!(self.column, Column::Building | Column::Qa | Column::Review);
        if worked
            && self.claim.as_deref() != Some(&by.name)
            && !by.caps.contains(&Capability::Triage)
        {
            return Err(BoardError::NotClaimant);
        }
        Ok(Self {
            column: to,
            claim: None,
            ..self.next(now_ms)
        })
    }

    /// Move to `to` with the movement rules off: no transition table, capability or claim
    /// check, only an actor holding some capability. The claim is cleared on arrival.
    ///
    /// # Errors
    /// If the card is Done, already in `to`, or the actor holds no capability.
    pub fn move_freely(&self, to: Column, by: &Actor, now_ms: i64) -> Result<Self, BoardError> {
        if self.column == Column::Done {
            return Err(BoardError::Terminal);
        }
        if self.column == to {
            return Err(BoardError::NoSuchMove { from: to, to });
        }
        if by.caps.is_empty() {
            return Err(BoardError::Invalid("moving a card needs some capability"));
        }
        Ok(Self {
            column: to,
            claim: None,
            ..self.next(now_ms)
        })
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

    /// Set the priority and the cards this one waits on (each optional).
    /// Repeated dependencies are dropped.
    ///
    /// # Errors
    /// A priority outside [`PRIORITIES`], a card depending on itself, or more
    /// than [`MAX_DEPS`] dependencies.
    pub fn plan(
        &self,
        priority: Option<u8>,
        deps: Option<Vec<Uuid>>,
        now_ms: i64,
    ) -> Result<Self, BoardError> {
        let mut next = self.next(now_ms);
        if let Some(p) = priority {
            if !PRIORITIES.contains(&p) {
                return Err(BoardError::Invalid("priority must be 0, 1, 3 or 5"));
            }
            next.priority = p;
        }
        if let Some(mut d) = deps {
            if d.contains(&self.id) {
                return Err(BoardError::Invalid("a card cannot depend on itself"));
            }
            d.sort_unstable();
            d.dedup();
            if d.len() > MAX_DEPS {
                return Err(BoardError::Invalid("too many dependencies"));
            }
            next.deps = d;
        }
        Ok(next)
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

    fn card() -> Card {
        Card::new("t", "", vec![], 0).expect("valid")
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
        let qa = actor("qa", &[Capability::Qa]);
        let rev = actor("rev", &[Capability::Review, Capability::Merge]);

        let c = card().move_to(Column::Ready, &human, 1).expect("triage");
        let c = c.move_to(Column::Building, &dev, 2).expect("start");
        let c = c.claim(&dev, 3).expect("claim");
        let c = c.move_to(Column::Qa, &dev, 4).expect("to qa");
        let c = c.claim(&qa, 5).expect("claim");
        let c = c.move_to(Column::Review, &qa, 6).expect("to review");
        let c = c.claim(&rev, 7).expect("claim");
        let c = c.move_to(Column::Done, &rev, 8).expect("merge");
        assert_eq!(c.column, Column::Done);
        assert_eq!(c.claim, None);
        assert_eq!(c.rev, 8);
    }

    #[test]
    fn capability_is_enforced() {
        let dev = actor("dev", &[Capability::Build]);
        assert_eq!(
            card().move_to(Column::Ready, &dev, 1),
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
            card().move_to(Column::Done, &all, 1),
            Err(BoardError::NoSuchMove {
                from: Column::Backlog,
                to: Column::Done
            })
        );
        let mut done = card();
        done.column = Column::Done;
        assert_eq!(
            done.move_to(Column::Ready, &all, 1),
            Err(BoardError::Terminal)
        );
    }

    #[test]
    fn open_rules_allow_any_move_but_keep_done_terminal() {
        let builder = actor("claude-cloud", &[Capability::Build]);
        let mut c = card();
        c.column = Column::Review;
        c.claim = Some("someone-else".into());
        let c = c
            .move_freely(Column::Ready, &builder, 1)
            .expect("out of review");
        assert_eq!((c.column, c.claim.as_deref()), (Column::Ready, None));
        let c = c
            .move_freely(Column::Done, &builder, 2)
            .expect("ready to done");
        assert_eq!(
            c.move_freely(Column::Backlog, &builder, 3),
            Err(BoardError::Terminal)
        );
    }

    #[test]
    fn open_rules_refuse_a_same_column_move_and_an_actor_without_capabilities() {
        let builder = actor("b", &[Capability::Build]);
        assert_eq!(
            card().move_freely(Column::Backlog, &builder, 1),
            Err(BoardError::NoSuchMove {
                from: Column::Backlog,
                to: Column::Backlog
            })
        );
        assert_eq!(
            card().move_freely(Column::Ready, &actor("none", &[]), 1),
            Err(BoardError::Invalid("moving a card needs some capability"))
        );
    }

    #[test]
    fn working_column_requires_claim() {
        let dev = actor("dev", &[Capability::Build]);
        let other = actor("other", &[Capability::Build]);
        let mut c = card();
        c.column = Column::Building;
        assert_eq!(c.move_to(Column::Qa, &dev, 1), Err(BoardError::NotClaimant));
        let c = c.claim(&dev, 1).expect("claim");
        assert_eq!(c.claim(&other, 2), Err(BoardError::Claimed("dev".into())));
        assert_eq!(
            c.move_to(Column::Qa, &other, 2),
            Err(BoardError::NotClaimant)
        );
        assert_eq!(c.release(&other, 2), Err(BoardError::Claimed("dev".into())));
        assert!(c.release(&dev, 2).expect("release").claim.is_none());
    }

    #[test]
    fn actionable_follows_the_transition_table() {
        let caps = |c: &[Capability]| c.iter().copied().collect::<BTreeSet<_>>();
        let builder = caps(&[Capability::Build]);
        assert!(actionable(Column::Ready, &builder) && actionable(Column::Building, &builder));
        assert!(!actionable(Column::Qa, &builder) && !actionable(Column::Review, &builder));
        let reviewer = caps(&[Capability::Review, Capability::Merge]);
        assert!(actionable(Column::Review, &reviewer) && !actionable(Column::Ready, &reviewer));
        assert!(
            !actionable(Column::Done, &caps(&[Capability::Merge])),
            "done is terminal"
        );
        assert_eq!(entry_column(Column::Ready), Some(Column::Building));
        assert_eq!(entry_column(Column::Qa), None);
        let closer = caps(&[Capability::Build, Capability::Close]);
        assert!(
            !actionable(Column::Qa, &closer),
            "close does not make QA work a builder's"
        );
    }

    #[test]
    fn close_records_merged_work_as_done() {
        let dev = actor("dev", &[Capability::Build]);
        let closer = actor("closer", &[Capability::Build, Capability::Close]);
        for from in [Column::Building, Column::Qa] {
            let mut c = card();
            c.column = from;
            let held = c.claim(&dev, 1).expect("claim");
            assert_eq!(
                held.move_to(Column::Done, &dev, 2),
                Err(BoardError::Forbidden(Capability::Close))
            );
            assert_eq!(
                held.move_to(Column::Done, &closer, 2),
                Err(BoardError::NotClaimant),
                "close still needs the claim"
            );
            let c = c.claim(&closer, 1).expect("claim");
            assert_eq!(
                c.move_to(Column::Done, &closer, 2).expect("close").column,
                Column::Done
            );
        }
        for from in [Column::Backlog, Column::Ready, Column::Blocked] {
            let mut c = card();
            c.column = from;
            assert_eq!(
                c.move_to(Column::Done, &closer, 1),
                Err(BoardError::NoSuchMove {
                    from,
                    to: Column::Done
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
        c.column = Column::Building;
        let c = c.claim(&dev, 1).expect("claim");
        assert!(
            c.release(&human, 2)
                .expect("override release")
                .claim
                .is_none()
        );
        assert_eq!(
            c.move_to(Column::Blocked, &human, 2)
                .expect("override move")
                .column,
            Column::Blocked
        );
        let other = actor("other", &[Capability::Build]);
        assert_eq!(
            c.move_to(Column::Blocked, &other, 2),
            Err(BoardError::NotClaimant)
        );
    }

    #[test]
    fn anyone_may_block() {
        let nobody = actor("n", &[]);
        let c = card().move_to(Column::Blocked, &nobody, 1).expect("block");
        assert_eq!(c.column, Column::Blocked);
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
