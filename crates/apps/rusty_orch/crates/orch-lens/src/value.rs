//! The value score (#596): how much resolving a claim is worth now. Pure
//! integer arithmetic over the cards and the runner's state, so the same
//! board always scores the same.
//!
//! `value = uncertainty × stakes × reversibility × dependents × 100 / cost`
//!
//! - **Uncertainty**: the accept line − |score|, at least 1 while the
//!   claim is open. A claim past its accept line can still be open
//!   (authors, grounding, a holdout run), so it keeps a floor rather than
//!   starving.
//! - **Stakes**: [`Stakes::factor`], the ratio budgets use (D71).
//! - **Reversibility**: 1, or [`IRREVERSIBLE`] (D71).
//! - **Dependents**: 1 + open claims that depend on this one, directly or
//!   through others, capped at [`DEPENDENTS_CAP`].
//! - **Cost**: 1 + this claim's jobs in flight (D72), + [`UNBUILT`] when no
//!   candidate names it. A question is never built, so it never pays
//!   [`UNBUILT`]: its proposals are what make candidates. Today's round
//!   runs its jobs one at a time and none is in flight when it picks, so
//!   the in-flight term only bites once a runner picks with jobs running.
//!
//! Settled claims and claims with no budget left score 0. The weights may
//! change in shadow mode and freeze before the paired runs (D36).

use std::collections::{BTreeMap, BTreeSet};

use rusty_bbp::{ClaimStatus, MsgId};

use crate::card::Card;
use crate::schedule::{is_question, State};

/// The reversibility weight of an irreversible claim (D71).
pub const IRREVERSIBLE: u64 = 2;
/// The dependents term never exceeds this.
pub const DEPENDENTS_CAP: u64 = 8;
/// Added to the cost while nothing is built for the claim.
pub const UNBUILT: u64 = 3;

/// Every card's value, by claim id.
pub fn values(cards: &[Card], state: &State) -> BTreeMap<MsgId, u64> {
    let dependents = dependents(cards);
    cards
        .iter()
        .map(|c| {
            let deps = dependents.get(&c.id).copied().unwrap_or(0);
            (c.id, value(c, deps, in_flight(state, c.id)))
        })
        .collect()
}

/// One card's value, given its open dependents and its jobs in flight.
pub fn value(card: &Card, dependents: u64, in_flight: u64) -> u64 {
    if card.status != ClaimStatus::Open || card.spent >= card.budget {
        return 0;
    }
    let reversibility = if card.reversible { 1 } else { IRREVERSIBLE };
    let unbuilt = !card.built && !is_question(card);
    let cost = 1 + in_flight + if unbuilt { UNBUILT } else { 0 };
    uncertainty(card)
        .saturating_mul(card.stakes.factor())
        .saturating_mul(reversibility)
        .saturating_mul((1 + dependents).min(DEPENDENTS_CAP))
        .saturating_mul(100)
        / cost
}

/// The accept line − |score|, at least 1.
fn uncertainty(card: &Card) -> u64 {
    card.accept_at
        .unsigned_abs()
        .saturating_sub(card.score.unsigned_abs())
        .max(1)
}

/// For each claim, how many open claims depend on it, directly or
/// transitively. Each dependent counts once per claim, cycles included.
fn dependents(cards: &[Card]) -> BTreeMap<MsgId, u64> {
    let deps: BTreeMap<MsgId, &[MsgId]> = cards.iter().map(|c| (c.id, &c.deps[..])).collect();
    let mut counts = BTreeMap::new();
    for card in cards.iter().filter(|c| c.status == ClaimStatus::Open) {
        for reached in closure(card.id, &deps) {
            *counts.entry(reached).or_insert(0) += 1;
        }
    }
    counts
}

/// Every claim `from` depends on through `deps`, not counting itself.
fn closure(from: MsgId, deps: &BTreeMap<MsgId, &[MsgId]>) -> BTreeSet<MsgId> {
    let mut seen = BTreeSet::new();
    let mut todo: Vec<MsgId> = deps.get(&from).map(|d| d.to_vec()).unwrap_or_default();
    while let Some(id) = todo.pop() {
        if id == from || !seen.insert(id) {
            continue;
        }
        todo.extend(deps.get(&id).into_iter().flat_map(|d| d.iter().copied()));
    }
    seen
}

fn in_flight(state: &State, claim: MsgId) -> u64 {
    let n = state.in_flight.iter().filter(|k| k.claim == claim).count();
    u64::try_from(n).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use rusty_bbp::Stakes;

    use super::*;
    use crate::card::fixtures::card;
    use crate::schedule::{JobKey, Slot};

    /// Open, medium, reversible, built, score 0 against lines ±8.
    fn base(id: u64, deps: &[u64]) -> Card {
        let mut c = card(vec![]);
        c.id = MsgId(id);
        c.deps = deps.iter().copied().map(MsgId).collect();
        c.built = true;
        c
    }

    #[test]
    fn the_terms_multiply() {
        let c = base(1, &[]);
        // uncertainty 8 × medium 2 × reversible 1 × dependents 1 × 100 / 1.
        assert_eq!(value(&c, 0, 0), 1_600);
        let mut high = c.clone();
        high.stakes = Stakes::High;
        assert_eq!(value(&high, 0, 0), 3_200);
        let mut once = c.clone();
        once.reversible = false;
        assert_eq!(value(&once, 0, 0), 3_200);
        assert_eq!(value(&c, 2, 0), 4_800);
    }

    #[test]
    fn cost_divides() {
        let mut c = base(1, &[]);
        assert_eq!(value(&c, 0, 1), 800);
        c.built = false;
        assert_eq!(value(&c, 0, 0), 400);
        assert_eq!(value(&c, 0, 1), 320);
    }

    #[test]
    fn a_question_is_not_charged_for_being_unbuilt() {
        let mut q = base(1, &[]);
        q.built = false;
        q.answers = None;
        q.domains = vec![crate::schedule::QUESTION_DOMAIN.to_owned()];
        assert_eq!(value(&q, 0, 0), 1_600);
    }

    #[test]
    fn uncertainty_is_the_line_less_the_scores_size() {
        let mut c = base(1, &[]);
        c.score = 5;
        assert_eq!(value(&c, 0, 0), 600);
        c.score = -7;
        assert_eq!(value(&c, 0, 0), 200);
    }

    #[test]
    fn an_open_claim_past_its_line_keeps_a_floor() {
        let mut c = base(1, &[]);
        c.score = 8;
        assert_eq!(value(&c, 0, 0), 200);
        c.score = i64::MAX;
        assert_eq!(value(&c, 0, 0), 200);
        c.score = i64::MIN;
        assert_eq!(value(&c, 0, 0), 200);
    }

    #[test]
    fn settled_or_spent_claims_are_worth_nothing() {
        let mut c = base(1, &[]);
        c.status = ClaimStatus::Accepted;
        assert_eq!(value(&c, 3, 0), 0);
        let mut spent = base(1, &[]);
        spent.spent = spent.budget;
        assert_eq!(value(&spent, 3, 0), 0);
    }

    #[test]
    fn dependents_are_transitive_capped_and_cycle_safe() {
        // 2 -> 1, 3 -> 2, 4 -> 3 and 4 -> 1: 1 has three dependents.
        let cards = [base(1, &[]), base(2, &[1]), base(3, &[2]), base(4, &[3, 1])];
        let d = dependents(&cards);
        assert_eq!(d.get(&MsgId(1)), Some(&3));
        assert_eq!(d.get(&MsgId(3)), Some(&1));
        assert_eq!(d.get(&MsgId(4)), None);
        let cycle = [base(1, &[2]), base(2, &[1])];
        assert_eq!(dependents(&cycle).get(&MsgId(1)), Some(&1));
        assert_eq!(value(&base(1, &[]), 20, 0), 1_600 * DEPENDENTS_CAP);
    }

    #[test]
    fn settled_dependents_do_not_count() {
        let mut done = base(2, &[1]);
        done.status = ClaimStatus::Accepted;
        assert_eq!(dependents(&[base(1, &[]), done]).get(&MsgId(1)), None);
    }

    #[test]
    fn values_read_jobs_in_flight() {
        let mut state = State::default();
        state.in_flight.insert(JobKey {
            lens: "operator".to_owned(),
            claim: MsgId(1),
            slot: Slot::Review,
        });
        let v = values(&[base(1, &[]), base(2, &[1])], &state);
        assert_eq!(v.get(&MsgId(1)), Some(&1_600));
        assert_eq!(v.get(&MsgId(2)), Some(&1_600));
    }
}
