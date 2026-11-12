//! Traceability: cards split into derived pieces (each piece names its parent) and cards counted
//! towards a milestone, with progress rolled up from them. Nothing here is stored: a rollup is
//! derived from the cards each time. Pure: the caller supplies the cards.
//!
//! A parent never closes itself. When every child is Done it is flagged `ready_to_close`, and
//! someone closes it (Nano, 2026-11-06, on cards #766 and #771).

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use uuid::Uuid;

use crate::domain::{BoardError, Card, Column};
use crate::milestones::Milestone;

/// The deepest chain of parents the board follows; deeper links are refused.
pub const MAX_DEPTH: usize = 32;

/// Progress derived from a parent's direct children, or a milestone's cards.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Rollup {
    /// How many there are.
    pub total: usize,
    /// How many shipped: Done and not dropped.
    pub done: usize,
    /// How many were closed without shipping ([`Card::close`](crate::domain::Card::close)).
    pub dropped: usize,
    /// How many are Blocked.
    pub blocked: usize,
    /// Every one is Done, at least one shipped, and the parent (or milestone) is still open:
    /// close it. Children that were all dropped leave nothing to close it on.
    pub ready_to_close: bool,
}

impl Rollup {
    fn of<'a>(cards: impl Iterator<Item = &'a Card>, closed: bool) -> Self {
        let mut r = Self::default();
        for c in cards {
            r.total += 1;
            let finished = c.column == Column::DONE;
            r.done += usize::from(finished && c.closed.is_none());
            r.dropped += usize::from(finished && c.closed.is_some());
            r.blocked += usize::from(c.column == Column::BLOCKED);
        }
        r.ready_to_close = r.done > 0 && r.done + r.dropped == r.total && !closed;
        r
    }
}

/// The rollup of `card`'s direct children among `cards`.
pub fn rollup(cards: &[Card], card: &Card) -> Rollup {
    Rollup::of(
        cards.iter().filter(|c| c.parent == Some(card.id)),
        card.column == Column::DONE,
    )
}

/// The rollup of the cards linked to `milestone`.
pub fn milestone_rollup(cards: &[Card], milestone: &Milestone) -> Rollup {
    Rollup::of(
        cards.iter().filter(|c| c.milestone == Some(milestone.id)),
        milestone.done_ms.is_some(),
    )
}

/// A milestone with the progress of its cards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MilestoneView {
    /// The milestone.
    #[serde(flatten)]
    pub milestone: Milestone,
    /// Its cards' progress.
    pub progress: Rollup,
}

/// Each of `milestones` with its rollup among `cards`.
pub fn with_progress(cards: &[Card], milestones: Vec<Milestone>) -> Vec<MilestoneView> {
    milestones
        .into_iter()
        .map(|m| MilestoneView {
            progress: milestone_rollup(cards, &m),
            milestone: m,
        })
        .collect()
}

/// Refuse `parent` as the parent of card `id` when it would close a loop or pass
/// [`MAX_DEPTH`]. `parent_of` reads a card's parent; a card that is gone ends the chain.
///
/// # Errors
/// [`BoardError::Invalid`] on a loop or a chain too deep, or what `parent_of` returns.
pub fn check_parent(
    id: Uuid,
    parent: Uuid,
    mut parent_of: impl FnMut(Uuid) -> Result<Option<Uuid>, BoardError>,
) -> Result<(), BoardError> {
    let mut at = Some(parent);
    for _ in 0..MAX_DEPTH {
        let Some(card) = at else { return Ok(()) };
        if card == id {
            return Err(BoardError::Invalid(
                "a card cannot be a parent of its own parent",
            ));
        }
        at = parent_of(card)?;
    }
    Err(BoardError::Invalid("the chain of parents is too deep"))
}

/// A card as a trace shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TraceCard {
    /// Identity.
    pub id: Uuid,
    /// Board-wide number, if assigned.
    pub number: Option<u64>,
    /// Short title.
    pub title: String,
    /// Current column.
    pub column: Column,
    /// Its project.
    pub project: String,
}

impl TraceCard {
    fn of(card: &Card) -> Self {
        Self {
            id: card.id,
            number: card.number,
            title: card.title.clone(),
            column: card.column.clone(),
            project: card.project.clone(),
        }
    }
}

/// A card, its rollup and its children, each a node in turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TraceNode {
    /// The card.
    #[serde(flatten)]
    pub card: TraceCard,
    /// Its direct children's progress.
    pub rollup: Rollup,
    /// Its children, by number.
    pub children: Vec<TraceNode>,
}

/// A card's place in its decomposition: the chain up to its root, and the tree below it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Trace {
    /// Its parent's chain, root first; empty for a root.
    pub ancestors: Vec<TraceCard>,
    /// The card and everything split from it.
    pub tree: TraceNode,
}

/// The trace of `card` among `cards`. Cards `keep` refuses (outside the caller's scope) are left
/// out with what hangs below or above them, though rollups still count them.
pub fn trace(cards: &[Card], card: &Card, keep: impl Fn(&Card) -> bool) -> Trace {
    let by_id: BTreeMap<Uuid, &Card> = cards.iter().map(|c| (c.id, c)).collect();
    let mut kids: BTreeMap<Uuid, Vec<&Card>> = BTreeMap::new();
    for c in cards {
        if let Some(p) = c.parent {
            kids.entry(p).or_default().push(c);
        }
    }
    kids.values_mut()
        .for_each(|v| v.sort_by_key(|c| (c.number.is_none(), c.number, c.created_ms, c.id)));
    let mut seen = BTreeSet::from([card.id]);
    Trace {
        ancestors: ancestors(&by_id, card, &keep, &mut seen),
        tree: node(&kids, card, &keep, &mut seen, 0),
    }
}

fn ancestors(
    by_id: &BTreeMap<Uuid, &Card>,
    card: &Card,
    keep: &impl Fn(&Card) -> bool,
    seen: &mut BTreeSet<Uuid>,
) -> Vec<TraceCard> {
    let mut chain = Vec::new();
    let mut at = card.parent;
    while let Some(c) = at.and_then(|id| by_id.get(&id)) {
        if chain.len() == MAX_DEPTH || !keep(c) || !seen.insert(c.id) {
            break;
        }
        chain.push(TraceCard::of(c));
        at = c.parent;
    }
    chain.reverse();
    chain
}

fn node(
    kids: &BTreeMap<Uuid, Vec<&Card>>,
    card: &Card,
    keep: &impl Fn(&Card) -> bool,
    seen: &mut BTreeSet<Uuid>,
    depth: usize,
) -> TraceNode {
    let mine = kids.get(&card.id).map(Vec::as_slice).unwrap_or_default();
    let rollup = Rollup::of(mine.iter().copied(), card.column == Column::DONE);
    let mut children = Vec::new();
    if depth < MAX_DEPTH {
        for c in mine {
            if keep(c) && seen.insert(c.id) {
                children.push(node(kids, c, keep, seen, depth + 1));
            }
        }
    }
    TraceNode {
        card: TraceCard::of(card),
        rollup,
        children,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(n: u64, parent: Option<&Card>, column: Column) -> Card {
        let mut c = Card::new(&format!("c{n}"), "", Vec::new(), 1).expect("card");
        c.number = Some(n);
        c.parent = parent.map(|p| p.id);
        c.column = column;
        c
    }

    #[test]
    fn dropped_children_are_counted_apart_and_alone_never_make_a_parent_ready() {
        use crate::domain::{CloseReason, Closure};
        let parent = card(1, None, Column::BACKLOG);
        let dropped = |n| Card {
            closed: Some(Closure {
                reason: CloseReason::WontDo,
                at_ms: 1,
            }),
            ..card(n, Some(&parent), Column::DONE)
        };
        let mut cards = vec![parent.clone(), dropped(2), dropped(3)];
        let r = rollup(&cards, &parent);
        assert_eq!((r.total, r.done, r.dropped), (2, 0, 2));
        assert!(!r.ready_to_close, "nothing shipped: nothing to close it on");
        cards.push(card(4, Some(&parent), Column::DONE));
        assert!(
            rollup(&cards, &parent).ready_to_close,
            "one shipped, the rest dropped"
        );
    }

    #[test]
    fn a_parent_rolls_up_its_direct_children_and_flags_ready_to_close() {
        let arc = card(1, None, Column::BACKLOG);
        let s2 = card(2, Some(&arc), Column::BACKLOG);
        let a = card(3, Some(&s2), Column::DONE);
        let b = card(4, Some(&s2), Column::BLOCKED);
        let mut cards = vec![arc.clone(), s2.clone(), a, b.clone()];
        assert_eq!(
            rollup(&cards, &s2),
            Rollup {
                total: 2,
                done: 1,
                dropped: 0,
                blocked: 1,
                ready_to_close: false
            }
        );
        assert_eq!(rollup(&cards, &arc).total, 1, "only direct children count");
        cards[3].column = Column::DONE;
        assert!(rollup(&cards, &s2).ready_to_close);
        let closed = Card {
            column: Column::DONE,
            ..s2
        };
        assert!(
            !rollup(&cards, &closed).ready_to_close,
            "a closed parent is not flagged"
        );
        assert!(
            !rollup(&cards, &b).ready_to_close,
            "a leaf is never ready to close"
        );
    }

    #[test]
    fn a_milestone_rolls_up_its_cards() {
        let m = Milestone::new("p", "Beta", 10).expect("milestone");
        let mut a = card(1, None, Column::DONE);
        a.milestone = Some(m.id);
        let mut cards = vec![a, card(2, None, Column::BACKLOG)];
        assert!(milestone_rollup(&cards, &m).ready_to_close);
        cards[1].milestone = Some(m.id);
        let r = milestone_rollup(&cards, &m);
        assert_eq!((r.total, r.done, r.ready_to_close), (2, 1, false));
        let reached = Milestone {
            done_ms: Some(5),
            ..m
        };
        cards[1].column = Column::DONE;
        assert!(!milestone_rollup(&cards, &reached).ready_to_close);
    }

    #[test]
    fn check_parent_refuses_loops_and_depth() {
        let (a, b, c) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let parents = BTreeMap::from([(b, a), (c, b)]);
        let up = |id: Uuid| Ok(parents.get(&id).copied());
        assert!(check_parent(Uuid::new_v4(), c, up).is_ok());
        assert!(
            check_parent(a, c, up).is_err(),
            "a under c, which is under a"
        );
        let deep = |_| Ok(Some(Uuid::new_v4()));
        assert!(check_parent(a, b, deep).is_err());
    }

    #[test]
    fn a_trace_shows_the_chain_up_and_the_tree_down() {
        let arc = card(1, None, Column::BACKLOG);
        let s2 = card(2, Some(&arc), Column::BACKLOG);
        let s2b = card(4, Some(&s2), Column::DONE);
        let s2a = card(3, Some(&s2), Column::BACKLOG);
        let leaf = card(5, Some(&s2a), Column::BACKLOG);
        let cards = vec![arc, s2.clone(), s2b, s2a.clone(), leaf];
        let t = trace(&cards, &s2a, |_| true);
        let nums = |v: &[TraceCard]| v.iter().map(|c| c.number).collect::<Vec<_>>();
        assert_eq!(nums(&t.ancestors), [Some(1), Some(2)], "root first");
        assert_eq!(t.tree.children.len(), 1);
        let t = trace(&cards, &s2, |_| true);
        let kids: Vec<_> = t.tree.children.iter().map(|n| n.card.number).collect();
        assert_eq!(kids, [Some(3), Some(4)], "children by number");
        assert_eq!(t.tree.children[0].children[0].card.number, Some(5));
        let hidden = trace(&cards, &s2, |c| c.number != Some(3));
        assert_eq!(hidden.tree.children.len(), 1, "out of scope left out");
        assert_eq!(hidden.tree.rollup.total, 2, "but still counted");
    }

    #[test]
    fn a_stored_loop_does_not_hang_a_trace() {
        let mut a = card(1, None, Column::BACKLOG);
        let b = card(2, Some(&a), Column::BACKLOG);
        a.parent = Some(b.id);
        let cards = vec![a.clone(), b];
        let t = trace(&cards, &a, |_| true);
        assert_eq!(t.ancestors.len(), 1);
        assert!(t.tree.children.is_empty(), "each card is shown once");
    }
}
