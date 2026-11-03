//! Tiering and routing: which providers a job may use, in order.

use orch_core::AgentId;
use rusty_bbp::Stakes;

use crate::card::Card;
use crate::class::admit;
use crate::config::{Lens, LensConfig};

/// Evidence items needed before a dispute counts as contested.
pub const CONTESTED_MIN_ITEMS: usize = 4;

/// Cheap models for routine evidence, stronger ones for contested claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Routine,
    Contested,
}

/// A claim's tier. Under a quarter of its budget left, always routine.
/// Otherwise contested when stakes are high, it is irreversible, or the
/// evidence runs both ways with the score under half the accept line after
/// at least [`CONTESTED_MIN_ITEMS`] items.
pub fn tier(card: &Card) -> Tier {
    if card.budget_low() {
        return Tier::Routine;
    }
    let split = card.disputed()
        && card.evidence.len() >= CONTESTED_MIN_ITEMS
        && card.score.unsigned_abs().saturating_mul(2) < card.accept_at.unsigned_abs();
    if card.stakes == Stakes::High || !card.reversible || split {
        Tier::Contested
    } else {
        Tier::Routine
    }
}

/// The lens's providers for `tier`, keeping only those cleared for the
/// claim's class, in fallback order. Empty means no cleared provider: the
/// job does not run.
pub fn route(config: &LensConfig, lens: &Lens, card: &Card) -> Vec<AgentId> {
    let list = match tier(card) {
        Tier::Routine => &lens.routine,
        Tier::Contested => &lens.contested,
    };
    list.iter()
        .copied()
        .filter(|id| {
            config
                .providers
                .get(id)
                .is_some_and(|p| admit(card.class, p.max_class))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::fixtures::{card, item};
    use crate::class::DataClass;
    use rusty_bbp::Direction;

    fn disputed(score: i64) -> Card {
        let mut c = card(
            (0..4)
                .map(|i| {
                    let d = if i % 2 == 0 {
                        Direction::Support
                    } else {
                        Direction::Counter
                    };
                    item(10 + i, "x", d, DataClass::Internal)
                })
                .collect(),
        );
        c.score = score;
        c
    }

    #[test]
    fn tiers() {
        assert_eq!(tier(&card(vec![])), Tier::Routine);
        let mut high = card(vec![]);
        high.stakes = Stakes::High;
        assert_eq!(tier(&high), Tier::Contested);
        let mut one_way = card(vec![]);
        one_way.reversible = false;
        assert_eq!(tier(&one_way), Tier::Contested);
        assert_eq!(tier(&disputed(3)), Tier::Contested, "|3| < 8/2");
        assert_eq!(tier(&disputed(4)), Tier::Routine, "|4| is half the line");
        let mut three = disputed(0);
        three.evidence.pop();
        assert_eq!(tier(&three), Tier::Routine, "under four items");
    }

    #[test]
    fn a_low_budget_forces_routine() {
        let mut c = card(vec![]);
        c.stakes = Stakes::High;
        c.spent = 13;
        assert_eq!(tier(&c), Tier::Routine);
    }

    fn id(s: &str) -> AgentId {
        AgentId::new(s).expect("id")
    }

    #[test]
    fn routes_follow_the_tier_and_drop_uncleared_providers() {
        let config = LensConfig::parse(crate::config::tests::STARTER).expect("config");
        let adversary = config.lens("adversary").expect("lens");
        assert_eq!(
            route(&config, adversary, &card(vec![])),
            [id("codex"), id("local")]
        );
        let mut contested = card(vec![]);
        contested.stakes = Stakes::High;
        assert_eq!(
            route(&config, adversary, &contested),
            [id("claude"), id("local-high")]
        );
        let mut sensitive = card(vec![]);
        sensitive.class = DataClass::Sensitive;
        assert_eq!(route(&config, adversary, &sensitive), [id("local")]);
        let research = config.lens("research").expect("lens");
        assert!(
            route(&config, research, &sensitive).is_empty(),
            "no cleared provider: the job does not run"
        );
    }
}
