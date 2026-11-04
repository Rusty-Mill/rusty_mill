//! Scheduling policies (#596, D36): which of the scheduled jobs a round
//! runs, and in what order. The #594 rules in [`crate::schedule`] decide
//! which jobs may exist; a policy only picks among them, and
//! [`crate::round::round`] drops any job it returns that was not scheduled.
//!
//! - [`EventDriven`]: every job, in schedule order. Today's behaviour and
//!   the baseline.
//! - [`ValueDriven`]: only the jobs of the highest-ranked claims, by value
//!   ([`crate::value`]) aged by the rounds each has waited. The rest wait
//!   for a later round, by which time other evidence may have settled them.
//!   Unlike the baseline, it never runs a claim worth 0 (settled or out of
//!   budget).

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroUsize;

use rusty_bbp::MsgId;

use crate::card::Card;
use crate::schedule::{Job, State};
use crate::value::values;

/// Picks and orders a round's jobs.
pub trait Policy {
    /// The jobs to run, in order: a subset of `jobs`, never a new one.
    fn pick(&mut self, cards: &[Card], state: &State, jobs: Vec<Job>) -> Vec<Job>;
}

/// Every job, in schedule order.
#[derive(Debug, Clone, Copy, Default)]
pub struct EventDriven;

impl Policy for EventDriven {
    fn pick(&mut self, _: &[Card], _: &State, jobs: Vec<Job>) -> Vec<Job> {
        jobs
    }
}

/// The jobs of the `claims` highest-ranked claims, highest first. A
/// claim's rank is its value × (1 + rounds it had jobs but was passed
/// over), so a claim that keeps winning without settling (its providers
/// failing, say) cannot starve the rest. Ties go to the lower id, which in
/// BBP is the older claim; a claim's own jobs keep their schedule order.
#[derive(Debug, Clone)]
pub struct ValueDriven {
    claims: NonZeroUsize,
    waited: BTreeMap<MsgId, u64>,
}

impl ValueDriven {
    pub fn new(claims: NonZeroUsize) -> Self {
        Self {
            claims,
            waited: BTreeMap::new(),
        }
    }
}

impl Policy for ValueDriven {
    fn pick(&mut self, cards: &[Card], state: &State, mut jobs: Vec<Job>) -> Vec<Job> {
        let worth = values(cards, state);
        let wanting: BTreeSet<MsgId> = jobs
            .iter()
            .map(|j| j.key.claim)
            .filter(|c| worth.get(c).is_some_and(|v| *v > 0))
            .collect();
        let rank = |c: &MsgId| {
            let waited = self.waited.get(c).copied().unwrap_or(0);
            worth
                .get(c)
                .copied()
                .unwrap_or(0)
                .saturating_mul(1 + waited)
        };
        let mut ranked: Vec<MsgId> = wanting.iter().copied().collect();
        // Stable over ascending ids, so equal ranks keep the older claim first.
        ranked.sort_by_key(|c| std::cmp::Reverse(rank(c)));
        ranked.truncate(self.claims.get());
        let order: BTreeMap<MsgId, usize> = ranked.into_iter().zip(0..).collect();
        self.waited.retain(|c, _| wanting.contains(c));
        for c in &wanting {
            if order.contains_key(c) {
                self.waited.remove(c);
            } else {
                *self.waited.entry(*c).or_insert(0) += 1;
            }
        }
        jobs.retain(|j| order.contains_key(&j.key.claim));
        jobs.sort_by_key(|j| order.get(&j.key.claim).copied());
        jobs
    }
}

#[cfg(test)]
mod tests {
    use rusty_bbp::Stakes;

    use super::*;
    use crate::card::fixtures::card;
    use crate::config::tests::STARTER;
    use crate::config::LensConfig;
    use crate::schedule::schedule;

    fn plain(n: u64, stakes: Stakes) -> Card {
        let mut c = card(vec![]);
        c.id = MsgId(n);
        c.answers = None;
        c.deps = vec![];
        c.stakes = stakes;
        c
    }

    fn jobs(cards: &[Card]) -> Vec<Job> {
        let config = LensConfig::parse(STARTER).expect("config");
        schedule(&config, cards, &State::default()).0
    }

    fn claims(jobs: &[Job]) -> Vec<u64> {
        let mut out: Vec<u64> = jobs.iter().map(|j| j.key.claim.0).collect();
        out.dedup();
        out
    }

    fn top(n: usize) -> ValueDriven {
        ValueDriven::new(NonZeroUsize::new(n).expect("nonzero"))
    }

    #[test]
    fn event_driven_keeps_the_schedule() {
        let cards = [plain(1, Stakes::Low), plain(2, Stakes::High)];
        let all = jobs(&cards);
        assert_eq!(
            EventDriven.pick(&cards, &State::default(), all.clone()),
            all
        );
    }

    #[test]
    fn value_driven_runs_the_top_claims_first() {
        let cards = [
            plain(1, Stakes::Low),
            plain(2, Stakes::High),
            plain(3, Stakes::Medium),
        ];
        let all = jobs(&cards);
        assert_eq!(claims(&all), [1, 2, 3]);
        let picked = top(2).pick(&cards, &State::default(), all.clone());
        assert_eq!(claims(&picked), [2, 3]);
        // A claim's own jobs keep their order and none is invented.
        let own = |js: &[Job]| {
            js.iter()
                .filter(|j| j.key.claim == MsgId(2))
                .cloned()
                .collect::<Vec<_>>()
        };
        assert_eq!(own(&picked), own(&all));
        assert!(picked.iter().all(|j| all.contains(j)));
    }

    #[test]
    fn ties_go_to_the_older_claim() {
        let cards = [plain(5, Stakes::Medium), plain(4, Stakes::Medium)];
        let picked = top(1).pick(&cards, &State::default(), jobs(&cards));
        assert_eq!(claims(&picked), [4]);
    }

    #[test]
    fn a_claim_worth_nothing_gets_no_job() {
        let mut spent = plain(1, Stakes::High);
        spent.spent = spent.budget;
        let cards = [spent, plain(2, Stakes::Low)];
        let picked = top(5).pick(&cards, &State::default(), jobs(&cards));
        assert_eq!(claims(&picked), [2]);
    }

    #[test]
    fn a_passed_over_claim_ages_into_a_round() {
        // Claim 2 is worth 4x claim 1; with one claim a round, claim 1
        // still runs within four rounds even though nothing settles.
        let cards = [plain(1, Stakes::Low), plain(2, Stakes::High)];
        let mut policy = top(1);
        let picks: Vec<Vec<u64>> = (0..4)
            .map(|_| claims(&policy.pick(&cards, &State::default(), jobs(&cards))))
            .collect();
        assert_eq!(picks[0], [2]);
        assert!(picks.iter().any(|p| p == &[1]), "{picks:?}");
    }
}
