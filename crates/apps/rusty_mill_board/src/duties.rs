//! Separation of duties (#447, `docs/design/factory-roles/design.md`): who authored, reviewed
//! and approved each card, and the rules that keep one hand from doing all three.
//!
//! The ledger lives on the card ([`Card::duties`]), so it is as durable as the card in either
//! store. The rules bind every actor except an `Admin` holder: that is the owner's override
//! until claude-cloud is narrowed (Nano, 2026-11-25, D2 "not yet"), and the ledger records its
//! actions too, so a self-review stays visible.
//!
//! - **Authors** are actors that claimed the card in a work column whose takers include a maker
//!   capability (plan, architect, build) they hold.
//! - **Review** (a forward move made with `review`, or [`review`]) is refused from an author, or
//!   from an actor sharing an author's provider.
//! - **Approval** ([`approve`]) crosses a gate with `approve`. An approver is never an author,
//!   and a gate out of a work column needs a passing review after the last authoring claim. A
//!   gate into Done (merge) or out of Blocked (unblock) is filed as pending: only an `Admin`
//!   holder's confirmation (Nano's tap) completes it (D1).

use serde::{Deserialize, Serialize};

use crate::domain::{Actor, BoardError, Capability, Card, Column, text_hash};
use crate::workflow::{Kind, Workflow};

/// Longest `verbatim` an approval keeps, in bytes.
pub const MAX_VERBATIM: usize = 2000;
/// Longest `source`, `note` or PR head, in bytes.
pub const MAX_SHORT: usize = 400;
/// Most ledger entries a card keeps; older entries past it are refused, never dropped silently.
pub const MAX_DUTIES: usize = 200;

/// One entry of a card's ledger. The provider is the actor's at the time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "duty", rename_all = "snake_case")]
pub enum Duty {
    /// Claimed the card to make something in `column`.
    Author {
        /// Who.
        actor: String,
        /// Their provider, if their credential names one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider: Option<String>,
        /// Where.
        column: Column,
        /// When, Unix ms.
        at_ms: i64,
    },
    /// Reviewed the work made in `column`.
    Review {
        /// Who.
        actor: String,
        /// Their provider.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider: Option<String>,
        /// The column whose work was reviewed.
        column: Column,
        /// Passed, or asked for changes.
        pass: bool,
        /// What the reviewer said; empty for a review move.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        note: String,
        /// When.
        at_ms: i64,
    },
    /// Crossed, or asked to cross, the gate `from` -> `to`.
    Approval {
        /// Who filed it.
        actor: String,
        /// Their provider.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider: Option<String>,
        /// Gate source.
        from: Column,
        /// Gate destination.
        to: Column,
        /// [`text_hash`] of the title and body read back to the approver.
        digest: u64,
        /// The readback and the owner's answer, verbatim. Empty for a plain admin move.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        verbatim: String,
        /// Where the owner said it (a message link or id), unverified audit data.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        source: String,
        /// For a merge gate: the pull request head the approval is for.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pr_head: Option<String>,
        /// Waits for an `Admin` holder's confirmation.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        pending: bool,
        /// Who confirmed it, when it needed confirming.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        confirmed_by: Option<String>,
        /// The card left the gate while this waited, so it can no longer be confirmed.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        expired: bool,
        /// When.
        at_ms: i64,
    },
}

/// What an [`approve`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The card should now move through the gate.
    Moves,
    /// Recorded (filed, confirmed, or the owner's own for a merge); the card stays.
    Stays,
    /// The owner read text edited since the gate's filings: they expired, nothing was approved
    /// and the card stays. The owner approves again on a fresh readback.
    Expired,
}

/// The most verdicts the instances of one credential keep on one column's work: each
/// instance's own verdict stands (a sibling cannot clear another's fail), and this bounds them.
pub const MAX_HAND_VERDICTS: usize = 4;

/// What an approver read back and the owner said, for [`approve`].
#[derive(Debug, Clone, Copy)]
pub struct Approval<'a> {
    /// The gate destination.
    pub to: &'a Column,
    /// [`text_hash`] of the title and body that were read back.
    pub digest: u64,
    /// The readback and the owner's answer.
    pub verbatim: &'a str,
    /// Where the owner said it.
    pub source: &'a str,
    /// For the merge gate: the pull request head.
    pub pr_head: Option<&'a str>,
}

fn owner(by: &Actor) -> bool {
    by.caps.contains(&Capability::Admin)
}

const MAKERS: [Capability; 3] = [Capability::Plan, Capability::Architect, Capability::Build];

/// Whether `by` claiming in `column` makes them an author.
fn authoring(flow: &Workflow, column: &Column, by: &Actor) -> bool {
    let Some(def) = flow.column(column) else {
        return false;
    };
    def.kind == Kind::Work
        && MAKERS
            .iter()
            .any(|m| def.takers.contains(m) && by.caps.contains(m))
}

fn push(card: &Card, duty: Duty) -> Result<Card, BoardError> {
    push_up_to(card, duty, MAX_DUTIES)
}

/// [`push`] for the owner: past a ledger others filled, so a full ledger never locks them out.
fn push_owner(card: &Card, duty: Duty) -> Result<Card, BoardError> {
    push_up_to(card, duty, 2 * MAX_DUTIES)
}

fn push_up_to(card: &Card, duty: Duty, max: usize) -> Result<Card, BoardError> {
    if card.duties.len() >= max {
        return Err(BoardError::Refused(format!(
            "this card's duty ledger is full ({max} entries)"
        )));
    }
    let mut next = card.clone();
    next.duties.push(duty);
    Ok(next)
}

/// `duty` at the end of the ledger in place of the entry `same` picks, so a repeated fact moves
/// to its newest place (the ledger stays in time order) and never grows the ledger; a new fact
/// is [`push`]ed.
fn upsert(card: &Card, duty: Duty, same: impl Fn(&Duty) -> bool) -> Result<Card, BoardError> {
    let Some(i) = card.duties.iter().position(same) else {
        return push(card, duty);
    };
    let mut next = card.clone();
    next.duties.remove(i);
    next.duties.push(duty);
    Ok(next)
}

/// A review by `actor` of the work in `column`.
fn review_of<'a>(actor: &'a str, column: &'a Column) -> impl Fn(&Duty) -> bool + 'a {
    move |d| matches!(d, Duty::Review { actor: a, column: c, .. } if a == actor && c == column)
}

/// `entry`, a verdict by `by` on `column`, in place of `by`'s own earlier one. Each instance
/// keeps its own verdict, so another instance of the credential cannot clear its fail; a
/// credential's instances keep at most [`MAX_HAND_VERDICTS`] on one column.
fn verdict(card: &Card, by: &Actor, column: &Column, entry: Duty) -> Result<Card, BoardError> {
    let mine = review_of(&by.name, column);
    let siblings = card
        .duties
        .iter()
        .filter(|d| matches!(d, Duty::Review { actor, column: c, .. } if hand(actor) == hand(&by.name) && c == column))
        .count();
    if !card.duties.iter().any(&mine) && siblings >= MAX_HAND_VERDICTS {
        return Err(BoardError::Refused(format!(
            "{MAX_HAND_VERDICTS} instances of {} already gave a verdict on this work",
            hand(&by.name)
        )));
    }
    upsert(card, entry, mine)
}

/// The credential behind an actor name: its instances (`claude-cli/a1`) and registered agents
/// (`claude-cloud#3f9a1c`) are one hand. Only an admin makes credentials, so keying the ledger
/// by hand bounds it by credentials, not by labels a caller picks.
fn hand(name: &str) -> &str {
    name.split(['/', '#']).next().unwrap_or(name)
}

/// `card` claimed by `by`: an [`Duty::Author`] entry when the claim is to make something,
/// unless the last entry already says the same. A repeat author of the column (any instance of
/// the same credential) moves to the end, with the newest name and provider (reviews before it
/// no longer count, R4), so claim cycles and labels do not grow the ledger.
///
/// # Errors
/// If the ledger is full.
pub fn on_claim(card: &Card, flow: &Workflow, by: &Actor, now_ms: i64) -> Result<Card, BoardError> {
    if !authoring(flow, &card.column, by) {
        return Ok(card.clone());
    }
    let repeat = matches!(card.duties.last(),
        Some(Duty::Author { actor, column, .. }) if *actor == by.name && *column == card.column);
    if repeat {
        return Ok(card.clone());
    }
    let column = card.column.clone();
    let entry = Duty::Author {
        actor: by.name.clone(),
        provider: by.provider.clone(),
        column: column.clone(),
        at_ms: now_ms,
    };
    upsert(
        card,
        entry,
        |d| matches!(d, Duty::Author { actor, column: c, .. } if hand(actor) == hand(&by.name) && *c == column),
    )
}

/// The author `by` may not review or approve for: one of the same credential ([`hand`]), or
/// the same provider.
fn same_hand<'a>(card: &'a Card, by: &Actor) -> Option<&'a str> {
    card.duties.iter().find_map(|d| match d {
        Duty::Author {
            actor, provider, ..
        } => {
            let same_provider = provider.is_some() && *provider == by.provider;
            (hand(actor) == hand(&by.name) || same_provider).then_some(actor.as_str())
        }
        _ => None,
    })
}

fn refuse_same_hand(card: &Card, by: &Actor, what: &str) -> Result<(), BoardError> {
    if owner(by) {
        return Ok(());
    }
    match same_hand(card, by) {
        Some(author) => Err(BoardError::Refused(format!(
            "separation of duties: {} may not {what} this card, which {author} authored (same actor or provider)",
            by.name
        ))),
        None => Ok(()),
    }
}

/// Whether `from` -> `to` is a review: a forward move every listed way of which needs `review`.
pub fn is_review_move(flow: &Workflow, from: &Column, to: &Column) -> bool {
    let index = |c: &Column| flow.columns().iter().position(|d| d.id == *c);
    let forward = matches!((index(from), index(to)), (Some(f), Some(t)) if t > f);
    let Ok(Some(ts)) = flow.transition(from, to) else {
        return false;
    };
    forward && ts.iter().all(|t| t.cap == Capability::Review)
}

/// Refuse a move of `card` to `to` by `by` that breaks a duty rule: a review move by an author
/// or a provider an author shares.
///
/// # Errors
/// [`BoardError::Refused`], naming the rule.
pub fn check_move(card: &Card, flow: &Workflow, to: &Column, by: &Actor) -> Result<(), BoardError> {
    if is_review_move(flow, &card.column, to) {
        refuse_same_hand(card, by, "review")?;
    }
    Ok(())
}

/// Record what a move from `from` did: a passing review for a review move, an approval for a
/// gate an `Admin` holder crossed with `card_move`. `moved` is the card after the move, and
/// `by` the actor it was made as (a recorder counts as holding `close`, so its close is not an
/// approval).
///
/// # Errors
/// If the ledger is full.
pub fn on_move(
    moved: &Card,
    flow: &Workflow,
    from: &Column,
    by: &Actor,
    now_ms: i64,
) -> Result<Card, BoardError> {
    if is_review_move(flow, from, &moved.column) {
        let entry = Duty::Review {
            actor: by.name.clone(),
            provider: by.provider.clone(),
            column: from.clone(),
            pass: true,
            note: String::new(),
            at_ms: now_ms,
        };
        return verdict(moved, by, from, entry);
    }
    if !through_gate(flow, from, &moved.column, by) {
        return Ok(moved.clone());
    }
    push_owner(
        moved,
        Duty::Approval {
            actor: by.name.clone(),
            provider: by.provider.clone(),
            from: from.clone(),
            to: moved.column.clone(),
            digest: text_hash(&moved.title, &moved.body),
            verbatim: String::new(),
            source: String::new(),
            pr_head: None,
            pending: false,
            confirmed_by: None,
            expired: false,
            at_ms: now_ms,
        },
    )
}

/// Whether `by`'s move `from` -> `to` went through a gate: some route is a gate, and no ungated
/// route is open to them (a `close` into Done beside the merge gate is not an approval).
fn through_gate(flow: &Workflow, from: &Column, to: &Column, by: &Actor) -> bool {
    let Ok(Some(ts)) = flow.transition(from, to) else {
        return false;
    };
    ts.iter().any(|t| t.gate) && !ts.iter().any(|t| !t.gate && by.caps.contains(&t.cap))
}

/// Expire the pending approvals of a card that changed column: what was filed for a gate the
/// card has left can no longer be confirmed, even if it comes back.
pub fn on_leave(before: &Card, mut after: Card) -> Card {
    if before.column == after.column {
        return after;
    }
    for d in &mut after.duties {
        if let Duty::Approval {
            pending: pending @ true,
            expired,
            ..
        } = d
        {
            *pending = false;
            *expired = true;
        }
    }
    after
}

/// `card` as a new revision at `now_ms`, for a ledger change that is not also a move or claim
/// (those bump the revision themselves).
fn bumped(mut card: Card, now_ms: i64) -> Card {
    card.rev += 1;
    card.updated_ms = now_ms;
    card
}

fn check_len(s: &str, max: usize, what: &'static str) -> Result<(), BoardError> {
    if s.len() > max {
        return Err(BoardError::Invalid(what));
    }
    Ok(())
}

/// `by` reviews the work of the card's current column: passed, or changes asked for.
///
/// # Errors
/// Without `review`, outside a work column, a note that is too long, or by an author or a
/// provider an author shares.
pub fn review(
    card: &Card,
    flow: &Workflow,
    by: &Actor,
    pass: bool,
    note: &str,
    now_ms: i64,
) -> Result<Card, BoardError> {
    if !by.caps.contains(&Capability::Review) {
        return Err(BoardError::Forbidden(Capability::Review));
    }
    if flow.kind(&card.column) != Some(Kind::Work) {
        return Err(BoardError::Refused(
            "a review is of the work in a work column".into(),
        ));
    }
    check_len(note, MAX_SHORT, "a review note is at most 400 bytes")?;
    refuse_same_hand(card, by, "review")?;
    let entry = Duty::Review {
        actor: by.name.clone(),
        provider: by.provider.clone(),
        column: card.column.clone(),
        pass,
        note: note.to_owned(),
        at_ms: now_ms,
    };
    // A reviewer's newer verdict on the same work replaces their older one, so reviews cannot
    // fill the ledger.
    let next = verdict(card, by, &card.column, entry)?;
    Ok(bumped(next, now_ms))
}

/// Whose reviews a gate out of `from` rests on (R4): `from`'s own when makers work there
/// (Architecting); else `from`'s verdicts and those of the work last authored (V&Ving's, and
/// the Building review, for the merge gate).
fn reviewed_work(flow: &Workflow, card: &Card, from: &Column) -> Vec<Column> {
    let makes = flow
        .column(from)
        .is_some_and(|d| MAKERS.iter().any(|m| d.takers.contains(m)));
    let authored = last_author(card).and_then(|i| match &card.duties[i] {
        Duty::Author { column, .. } => Some(column.clone()),
        _ => None,
    });
    match authored {
        Some(work) if !makes && work != *from => vec![work, from.clone()],
        _ => vec![from.clone()],
    }
}

fn last_author(card: &Card) -> Option<usize> {
    card.duties
        .iter()
        .rposition(|d| matches!(d, Duty::Author { .. }))
}

/// Whether the reviews of `columns` since the card's last authoring claim pass (R4): at least
/// one passes and none asks for changes (each reviewer's newest verdict is the one kept). A
/// review of an earlier column does not carry a later gate.
fn reviewed_since_authored(card: &Card, columns: &[Column]) -> bool {
    let after = last_author(card).map_or(0, |i| i + 1);
    let verdicts = card.duties[after..].iter().filter_map(|d| match d {
        Duty::Review { column, pass, .. } if columns.contains(column) => Some(*pass),
        _ => None,
    });
    let (mut passed, mut failed) = (false, false);
    for pass in verdicts {
        passed |= pass;
        failed |= !pass;
    }
    passed && !failed
}

/// Whether the gate `from` -> `to` needs the owner's confirmation: a merge (into Done) or an
/// unblock (out of Blocked).
fn needs_confirmation(flow: &Workflow, from: &Column, to: &Column) -> bool {
    flow.kind(to) == Some(Kind::Done) || flow.kind(from) == Some(Kind::Blocked)
}

/// `by` approves the gate from the card's column to `a.to`, on what was read back to the owner.
/// Returns the card and the [`Outcome`]: whether it should now move to `a.to` (the caller
/// makes the move, so the claim and Done time follow the board's usual arrival, and its
/// revision then; a card that stays is returned as a new revision already).
///
/// - With `Admin` (the owner): confirms a pending approval for this gate, or records a new one.
///   The card moves, except through the merge gate, whose approval is recorded for the PR head
///   and the card is closed after the merge as usual.
/// - With `approve` only (the proxy): refused from an author, and out of a work column without
///   a passing review after the last authoring claim. A merge or an unblock is filed pending;
///   any other gate moves.
///
/// # Errors
/// A missing capability, a move that is not a gate, a card that changed since the readback
/// (`digest`), an empty or oversized `verbatim` or `source`, a merge approval without a PR
/// head, or a broken duty rule.
pub fn approve(
    card: &Card,
    flow: &Workflow,
    by: &Actor,
    a: &Approval<'_>,
    now_ms: i64,
) -> Result<(Card, Outcome), BoardError> {
    let (next, outcome) = approve_in(card, flow, by, a, now_ms)?;
    let next = match outcome {
        Outcome::Moves => next,
        Outcome::Stays | Outcome::Expired => bumped(next, now_ms),
    };
    Ok((next, outcome))
}

/// [`Outcome::Moves`] when `moves`, else [`Outcome::Stays`].
fn moves_if(moves: bool) -> Outcome {
    if moves {
        Outcome::Moves
    } else {
        Outcome::Stays
    }
}

fn approve_in(
    card: &Card,
    flow: &Workflow,
    by: &Actor,
    a: &Approval<'_>,
    now_ms: i64,
) -> Result<(Card, Outcome), BoardError> {
    let is_owner = owner(by);
    if !is_owner && !by.caps.contains(&Capability::Approve) {
        return Err(BoardError::Forbidden(Capability::Approve));
    }
    let from = card.column.clone();
    let gate = matches!(flow.transition(&from, a.to)?, Some(ts) if ts.iter().any(|t| t.gate));
    if !gate {
        return Err(BoardError::Refused(format!(
            "{from} -> {} is not a gate",
            a.to
        )));
    }
    if a.digest != text_hash(&card.title, &card.body) {
        return Err(BoardError::Refused(
            "the card's title or body changed since it was read back: read it back again".into(),
        ));
    }
    if a.verbatim.trim().is_empty() || a.source.trim().is_empty() {
        return Err(BoardError::Invalid("an approval needs verbatim and source"));
    }
    check_len(a.verbatim, MAX_VERBATIM, "verbatim is at most 2000 bytes")?;
    check_len(a.source, MAX_SHORT, "source is at most 400 bytes")?;
    let merge = flow.kind(a.to) == Some(Kind::Done);
    let pr_head = a.pr_head.map(str::trim).filter(|h| !h.is_empty());
    if merge && pr_head.is_none() {
        return Err(BoardError::Invalid("a merge approval needs pr_head"));
    }
    if let Some(h) = pr_head {
        check_len(h, MAX_SHORT, "pr_head is at most 400 bytes")?;
    }
    if is_owner {
        return confirm_or_record(card, by, &from, a, pr_head, merge, now_ms);
    }
    refuse_same_hand(card, by, "approve")?;
    if flow.kind(&from) == Some(Kind::Work) {
        let work = reviewed_work(flow, card, &from);
        if !reviewed_since_authored(card, &work) {
            let work: Vec<&str> = work.iter().map(Column::as_str).collect();
            return Err(BoardError::Refused(format!(
                "separation of duties: this gate needs a passing review of the work in {}, and none asking for changes, after the last authoring claim (card_review)",
                work.join(" or ")
            )));
        }
    }
    let pending = needs_confirmation(flow, &from, a.to);
    let entry = approval(by, &from, a, pr_head, pending, None, now_ms);
    // A new filing replaces the filer's pending or expired one for this gate, so refiling and
    // gate cycles do not grow the ledger.
    let next = upsert(card, entry, |d| {
        matches!(d, Duty::Approval { actor, from: f, to, pending, expired, .. }
            if (*pending || *expired) && hand(actor) == hand(&by.name) && *f == from && to == a.to)
    })?;
    Ok((next, moves_if(!pending)))
}

fn approval(
    by: &Actor,
    from: &Column,
    a: &Approval<'_>,
    pr_head: Option<&str>,
    pending: bool,
    confirmed_by: Option<String>,
    now_ms: i64,
) -> Duty {
    Duty::Approval {
        actor: by.name.clone(),
        provider: by.provider.clone(),
        from: from.clone(),
        to: a.to.clone(),
        digest: a.digest,
        verbatim: a.verbatim.to_owned(),
        source: a.source.to_owned(),
        pr_head: pr_head.map(str::to_owned),
        pending,
        confirmed_by,
        expired: false,
        at_ms: now_ms,
    }
}

/// The owner's approval: confirm the newest pending filing for this gate made on the text they
/// read, else record their own. Filings on text edited since are not what the owner read: they
/// expire and the card stays, so the owner approves again on a fresh readback (no lockout, and
/// no gate crossed on unread text). A merge filing for another PR head leaves the owner's
/// approval of this head recorded on its own.
fn confirm_or_record(
    card: &Card,
    by: &Actor,
    from: &Column,
    a: &Approval<'_>,
    pr_head: Option<&str>,
    merge: bool,
    now_ms: i64,
) -> Result<(Card, Outcome), BoardError> {
    let filed = |d: &Duty| matches!(d, Duty::Approval { from: f, to, pending: true, .. } if f == from && to == a.to);
    let read = card.duties.iter().rposition(|d| {
        filed(d) && matches!(d, Duty::Approval { digest, .. } if *digest == a.digest)
    });
    if let Some(i) = read {
        let mut next = card.clone();
        if let Some(Duty::Approval {
            pending,
            confirmed_by,
            pr_head: filed_head,
            ..
        }) = next.duties.get_mut(i)
        {
            // A different PR head is a new approval.
            if merge && filed_head.as_deref() != pr_head {
                let mine = approval(by, from, a, pr_head, false, None, now_ms);
                return push_owner(card, mine).map(|c| (c, Outcome::Stays));
            }
            *pending = false;
            *confirmed_by = Some(by.name.clone());
        }
        return Ok((next, moves_if(!merge)));
    }
    if card.duties.iter().any(filed) {
        let mut next = card.clone();
        for d in next.duties.iter_mut().filter(|d| filed(d)) {
            if let Duty::Approval {
                pending, expired, ..
            } = d
            {
                *pending = false;
                *expired = true;
            }
        }
        return Ok((next, Outcome::Expired));
    }
    let mine = approval(by, from, a, pr_head, false, None, now_ms);
    push_owner(card, mine).map(|c| (c, moves_if(!merge)))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::workflow::{ColumnDef, Transition};

    fn col(id: &str, kind: Kind, takers: &[Capability]) -> ColumnDef {
        ColumnDef {
            id: Column::new(id).expect("column"),
            label: id.to_owned(),
            kind,
            takers: takers.iter().copied().collect(),
        }
    }

    fn mv(from: &str, to: &str, cap: Capability, gate: bool) -> Transition {
        Transition {
            from: Column::new(from).expect("from"),
            to: Column::new(to).expect("to"),
            cap,
            gate,
        }
    }

    /// The live factory workflow's shape.
    fn flow() -> Workflow {
        use Capability::*;
        Workflow::new(
            vec![
                col("backlog", Kind::Backlog, &[]),
                col("ready", Kind::Queue, &[Plan]),
                col("planning", Kind::Work, &[Plan, Test, Review]),
                col("architecting", Kind::Work, &[Architect, Test, Review]),
                col("building", Kind::Work, &[Build, Test, Review]),
                col("v_and_ving", Kind::Work, &[Test, Review]),
                col("done", Kind::Done, &[]),
                col("blocked", Kind::Blocked, &[]),
            ],
            vec![
                mv("backlog", "ready", Triage, true),
                mv("ready", "planning", Plan, false),
                mv("planning", "architecting", Review, false),
                mv("architecting", "building", Triage, true),
                mv("building", "v_and_ving", Review, false),
                mv("v_and_ving", "building", Review, false),
                mv("v_and_ving", "done", Merge, true),
                mv("blocked", "building", Triage, true),
            ],
        )
        .expect("flow")
    }

    fn actor(name: &str, provider: Option<&str>, caps: &[Capability]) -> Actor {
        Actor {
            name: name.to_owned(),
            caps: caps.iter().copied().collect::<BTreeSet<_>>(),
            projects: None,
            provider: provider.map(str::to_owned),
        }
    }

    fn at(column: &str) -> Card {
        let mut c = Card::new("t", "b", vec![], 0).expect("card");
        c.column = Column::new(column).expect("column");
        c
    }

    fn approval_of<'a>(card: &Card, to: &'a Column) -> Approval<'a> {
        Approval {
            to,
            digest: text_hash(&card.title, &card.body),
            verbatim: "Approve the design for #1? yes",
            source: "cmsg_1",
            pr_head: None,
        }
    }

    fn maker() -> Actor {
        use Capability::*;
        actor(
            "cloud#1",
            Some("anthropic"),
            &[Plan, Architect, Build, Review, Test],
        )
    }

    fn proxy() -> Actor {
        actor("nano-proxy", Some("human"), &[Capability::Approve])
    }

    #[test]
    fn a_maker_claim_records_an_author_once() {
        let c = on_claim(&at("building"), &flow(), &maker(), 1).expect("claim");
        let c = on_claim(&c, &flow(), &maker(), 2).expect("again");
        assert_eq!(c.duties.len(), 1);
        let reviewer = actor(
            "dots",
            Some("openai"),
            &[Capability::Review, Capability::Test],
        );
        let c = on_claim(&c, &flow(), &reviewer, 3).expect("reviewer claim");
        assert_eq!(c.duties.len(), 1, "a reviewer's claim makes no author");
    }

    #[test]
    fn an_author_or_same_provider_review_is_refused_and_others_pass() {
        let c = on_claim(&at("building"), &flow(), &maker(), 1).expect("claim");
        let to = Column::new("v_and_ving").expect("col");
        assert!(
            check_move(&c, &flow(), &to, &maker()).is_err(),
            "self review"
        );
        let sibling = actor("cloud#2", Some("anthropic"), &[Capability::Review]);
        assert!(
            check_move(&c, &flow(), &to, &sibling).is_err(),
            "same provider"
        );
        let dots = actor("dots", Some("openai"), &[Capability::Review]);
        assert!(check_move(&c, &flow(), &to, &dots).is_ok());
        let unset = actor("local", None, &[Capability::Review]);
        assert!(
            check_move(&c, &flow(), &to, &unset).is_ok(),
            "providers unset: by credential only"
        );
        let agent = actor("cloud#3", None, &[Capability::Review]);
        assert!(
            check_move(&c, &flow(), &to, &agent).is_err(),
            "another agent of the author's credential"
        );
        let back = Column::new("building").expect("col");
        let mut there = c.clone();
        there.column = to.clone();
        assert!(
            check_move(&there, &flow(), &back, &maker()).is_ok(),
            "sending back is free"
        );
    }

    #[test]
    fn the_owner_is_not_bound_but_recorded() {
        let human = actor(
            "nano",
            Some("human"),
            &[Capability::Admin, Capability::Review],
        );
        let mut c = on_claim(&at("building"), &flow(), &human, 1).expect("claim");
        let to = Column::new("v_and_ving").expect("col");
        assert!(check_move(&c, &flow(), &to, &human).is_ok());
        c.column = to;
        let c = on_move(
            &c,
            &flow(),
            &Column::new("building").expect("col"),
            &human,
            2,
        )
        .expect("record");
        assert!(matches!(
            c.duties.last(),
            Some(Duty::Review { pass: true, .. })
        ));
    }

    #[test]
    fn the_proxy_crosses_a_design_gate_only_after_a_foreign_review() {
        let c = on_claim(&at("architecting"), &flow(), &maker(), 1).expect("claim");
        let to = Column::new("building").expect("col");
        let refused = approve(&c, &flow(), &proxy(), &approval_of(&c, &to), 2);
        assert!(refused.is_err(), "no review yet");
        assert!(
            review(&c, &flow(), &maker(), true, "", 2).is_err(),
            "self review"
        );
        let dots = actor("dots", Some("openai"), &[Capability::Review]);
        let c = review(&c, &flow(), &dots, true, "ok", 3).expect("review");
        let (c, moves) = approve(&c, &flow(), &proxy(), &approval_of(&c, &to), 4).expect("ok");
        assert_eq!(moves, Outcome::Moves);
        assert!(matches!(
            c.duties.last(),
            Some(Duty::Approval { pending: false, .. })
        ));
    }

    #[test]
    fn a_changes_review_does_not_open_the_gate() {
        let c = on_claim(&at("architecting"), &flow(), &maker(), 1).expect("claim");
        let dots = actor("dots", Some("openai"), &[Capability::Review]);
        let c = review(&c, &flow(), &dots, false, "redo", 2).expect("review");
        let to = Column::new("building").expect("col");
        assert!(approve(&c, &flow(), &proxy(), &approval_of(&c, &to), 3).is_err());
    }

    #[test]
    fn a_stale_readback_and_a_non_gate_are_refused() {
        let c = at("backlog");
        let to = Column::new("ready").expect("col");
        let mut stale = approval_of(&c, &to);
        stale.digest ^= 1;
        assert!(approve(&c, &flow(), &proxy(), &stale, 1).is_err());
        let c = at("ready");
        let planning = Column::new("planning").expect("col");
        assert!(approve(&c, &flow(), &proxy(), &approval_of(&c, &planning), 1).is_err());
    }

    #[test]
    fn a_proxy_merge_waits_for_the_owner_and_never_moves() {
        let c = on_claim(&at("building"), &flow(), &maker(), 1).expect("claim");
        let dots = actor("dots", Some("openai"), &[Capability::Review]);
        let mut c = on_move(
            &Card {
                column: Column::new("v_and_ving").expect("col"),
                ..c
            },
            &flow(),
            &Column::new("building").expect("col"),
            &dots,
            2,
        )
        .expect("review move");
        c.claim = None;
        let done = Column::DONE;
        let mut a = approval_of(&c, &done);
        assert!(approve(&c, &flow(), &proxy(), &a, 3).is_err(), "no PR head");
        a.pr_head = Some("abc123");
        let (c, moves) = approve(&c, &flow(), &proxy(), &a, 3).expect("filed");
        assert!(moves != Outcome::Moves);
        assert!(matches!(
            c.duties.last(),
            Some(Duty::Approval { pending: true, .. })
        ));
        let nano = actor("nano", Some("human"), &[Capability::Admin]);
        let (c, moves) = approve(&c, &flow(), &nano, &a, 4).expect("confirmed");
        assert!(
            moves != Outcome::Moves,
            "a merge approval never moves the card"
        );
        assert!(matches!(c.duties.last(),
            Some(Duty::Approval { pending: false, confirmed_by: Some(n), .. }) if n == "nano"));
    }

    #[test]
    fn a_proxy_unblock_is_pending_until_confirmed() {
        let c = at("blocked");
        let to = Column::new("building").expect("col");
        let (c, moves) = approve(&c, &flow(), &proxy(), &approval_of(&c, &to), 1).expect("filed");
        assert!(moves != Outcome::Moves);
        let nano = actor("nano", Some("human"), &[Capability::Admin]);
        let (_, moves) = approve(&c, &flow(), &nano, &approval_of(&c, &to), 2).expect("tap");
        assert_eq!(moves, Outcome::Moves);
    }

    #[test]
    fn an_author_cannot_approve_and_others_need_approve() {
        let mut approver = maker();
        approver.caps.insert(Capability::Approve);
        let c = on_claim(&at("architecting"), &flow(), &approver, 1).expect("claim");
        let dots = actor("dots", Some("openai"), &[Capability::Review]);
        let c = review(&c, &flow(), &dots, true, "", 2).expect("review");
        let to = Column::new("building").expect("col");
        assert!(approve(&c, &flow(), &approver, &approval_of(&c, &to), 3).is_err());
        assert!(matches!(
            approve(&c, &flow(), &dots, &approval_of(&c, &to), 3),
            Err(BoardError::Forbidden(Capability::Approve))
        ));
    }

    #[test]
    fn the_owner_writes_past_a_ledger_others_filled() {
        let mut c = at("blocked");
        c.duties = (0..MAX_DUTIES)
            .map(|i| Duty::Author {
                actor: format!("a{i}"),
                provider: None,
                column: Column::new("building").expect("column"),
                at_ms: 0,
            })
            .collect();
        let to = Column::new("building").expect("column");
        let filed = approve(&c, &flow(), &proxy(), &approval_of(&c, &to), 1);
        assert!(matches!(filed, Err(BoardError::Refused(m)) if m.contains("full")));
        let nano = actor("nano", None, &[Capability::Admin]);
        let (c, moves) = approve(&c, &flow(), &nano, &approval_of(&c, &to), 1).expect("owner");
        assert_eq!(moves, Outcome::Moves);
        assert_eq!(c.duties.len(), MAX_DUTIES + 1);
    }
}
