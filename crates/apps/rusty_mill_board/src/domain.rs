//! Pure board domain: cards, columns, capabilities and the transition table.
//!
//! No I/O and no clock: callers pass `now_ms`. Every operation takes a card
//! by reference and returns the next card, so an illegal move is an `Err`,
//! never a half-applied mutation.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A board column, in workflow order (the order mirrors super-board's).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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
}

/// Who is acting, and what they may do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    /// Stable name, recorded in claims and history.
    pub name: String,
    /// Capabilities held.
    pub caps: BTreeSet<Capability>,
}

/// The capability needed to move a card `from` -> `to`.
/// `Ok(None)` means any actor may; `Err` means the move does not exist.
fn required(from: Column, to: Column) -> Result<Option<Capability>, BoardError> {
    use Column::*;
    match (from, to) {
        (Done, _) => Err(BoardError::Terminal),
        (_, Blocked) => Ok(None),
        (Backlog, Ready) | (Blocked, Ready) => Ok(Some(Capability::Triage)),
        (Ready, Building) | (Building, Qa) => Ok(Some(Capability::Build)),
        (Qa, Building) | (Qa, Review) => Ok(Some(Capability::Qa)),
        (Review, Building) => Ok(Some(Capability::Review)),
        (Review, Done) => Ok(Some(Capability::Merge)),
        _ => Err(BoardError::NoSuchMove { from, to }),
    }
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
    /// Discussion, oldest first.
    pub comments: Vec<Comment>,
    /// Creation time, Unix ms.
    pub created_ms: i64,
    /// Last change, Unix ms.
    pub updated_ms: i64,
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
}

/// One entry in the append-only log. `id` is a UUIDv7, so ids sort in
/// commit order and double as a polling cursor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    /// Time-ordered identity and cursor.
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

impl Event {
    /// The event describing `card` just after `kind` happened.
    pub fn of(card: &Card, kind: EventKind, actor: &str) -> Self {
        Self {
            id: Uuid::now_v7(),
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
    /// If `title` is blank.
    pub fn new(title: &str, body: &str, tags: Vec<String>, now_ms: i64) -> Result<Self, BoardError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(BoardError::Invalid("title is empty"));
        }
        Ok(Self {
            id: Uuid::new_v4(),
            rev: 0,
            title: title.to_owned(),
            body: body.to_owned(),
            column: Column::Backlog,
            claim: None,
            tags,
            comments: Vec::new(),
            created_ms: now_ms,
            updated_ms: now_ms,
        })
    }

    fn next(&self, now_ms: i64) -> Self {
        Self { rev: self.rev + 1, updated_ms: now_ms, ..self.clone() }
    }

    /// Take the claim.
    ///
    /// # Errors
    /// If another actor already holds it.
    pub fn claim(&self, by: &Actor, now_ms: i64) -> Result<Self, BoardError> {
        match &self.claim {
            Some(holder) if *holder != by.name => Err(BoardError::Claimed(holder.clone())),
            _ => Ok(Self { claim: Some(by.name.clone()), ..self.next(now_ms) }),
        }
    }

    /// Drop the claim; only its holder may.
    ///
    /// # Errors
    /// If someone else holds it.
    pub fn release(&self, by: &Actor, now_ms: i64) -> Result<Self, BoardError> {
        match &self.claim {
            Some(holder) if *holder != by.name => Err(BoardError::Claimed(holder.clone())),
            _ => Ok(Self { claim: None, ..self.next(now_ms) }),
        }
    }

    /// Move to `to`. The claim is cleared on arrival, so the next stage's
    /// worker claims it afresh. Leaving a worked column (Building, Qa, Review)
    /// requires holding the claim.
    ///
    /// # Errors
    /// If the move does not exist, the actor lacks the capability, or the
    /// actor is not the claim holder.
    pub fn move_to(&self, to: Column, by: &Actor, now_ms: i64) -> Result<Self, BoardError> {
        if let Some(cap) = required(self.column, to)?
            && !by.caps.contains(&cap)
        {
            return Err(BoardError::Forbidden(cap));
        }
        let worked = matches!(self.column, Column::Building | Column::Qa | Column::Review);
        if worked && self.claim.as_deref() != Some(&by.name) {
            return Err(BoardError::NotClaimant);
        }
        Ok(Self { column: to, claim: None, ..self.next(now_ms) })
    }

    /// Replace title/body/tags (each optional).
    ///
    /// # Errors
    /// If the new title is blank.
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
            next.tags = t;
        }
        Ok(next)
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
        next.comments.push(Comment { by: by.name.clone(), at_ms: now_ms, text: text.to_owned() });
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actor(name: &str, caps: &[Capability]) -> Actor {
        Actor { name: name.into(), caps: caps.iter().copied().collect() }
    }

    fn card() -> Card {
        Card::new("t", "", vec![], 0).expect("valid")
    }

    #[test]
    fn blank_title_rejected() {
        assert_eq!(Card::new("  ", "", vec![], 0), Err(BoardError::Invalid("title is empty")));
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
        let all = actor("a", &[Capability::Triage, Capability::Build, Capability::Merge]);
        assert_eq!(
            card().move_to(Column::Done, &all, 1),
            Err(BoardError::NoSuchMove { from: Column::Backlog, to: Column::Done })
        );
        let mut done = card();
        done.column = Column::Done;
        assert_eq!(done.move_to(Column::Ready, &all, 1), Err(BoardError::Terminal));
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
        assert_eq!(c.move_to(Column::Qa, &other, 2), Err(BoardError::NotClaimant));
        assert_eq!(c.release(&other, 2), Err(BoardError::Claimed("dev".into())));
        assert!(c.release(&dev, 2).expect("release").claim.is_none());
    }

    #[test]
    fn anyone_may_block() {
        let nobody = actor("n", &[]);
        let c = card().move_to(Column::Blocked, &nobody, 1).expect("block");
        assert_eq!(c.column, Column::Blocked);
    }

    #[test]
    fn edit_and_comment_bump_rev() {
        let a = actor("a", &[]);
        let c = card().edit(Some("new"), None, Some(vec!["x".into()]), 5).expect("edit");
        assert_eq!((c.title.as_str(), c.rev, c.updated_ms), ("new", 1, 5));
        let c = c.comment(&a, "hi", 6).expect("comment");
        assert_eq!((c.comments.len(), c.rev), (1, 2));
        assert!(c.comment(&a, " ", 7).is_err());
        assert!(c.edit(Some(" "), None, None, 7).is_err());
    }
}
