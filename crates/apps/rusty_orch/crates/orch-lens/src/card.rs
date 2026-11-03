//! The runner's view of one claim: what the shell reads from BBP (the claim
//! card plus each evidence message's note), with orch's data class on every
//! part. Pure data and its text rendering.

use rusty_bbp::{ClaimStatus, Direction, EvidenceKind, MsgId, PrincipalId, Stakes};

use crate::class::{admit, DataClass};

/// One claim as a lens may be shown it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub id: MsgId,
    pub text: String,
    pub domains: Vec<String>,
    pub status: ClaimStatus,
    pub score: i64,
    /// The score that accepts this claim (strict for high stakes or
    /// irreversible claims).
    pub accept_at: i64,
    pub stakes: Stakes,
    pub reversible: bool,
    /// The question this claim answers, when it is a candidate.
    pub answers: Option<MsgId>,
    pub deps: Vec<MsgId>,
    /// Evidence units spent and the claim's budget.
    pub spent: u64,
    pub budget: u64,
    pub evidence: Vec<Item>,
    /// For a question: its candidate claims.
    pub candidates: Vec<MsgId>,
    pub class: DataClass,
}

/// One evidence item on a card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub msg: MsgId,
    pub author: PrincipalId,
    pub direction: Direction,
    pub kind: EvidenceKind,
    pub delta: i64,
    pub note: String,
    pub class: DataClass,
}

impl Card {
    /// True when both directions have evidence.
    pub fn disputed(&self) -> bool {
        let has = |d| self.evidence.iter().any(|i| i.direction == d);
        has(Direction::Support) && has(Direction::Counter)
    }

    /// Budget left, in quarters of the whole: `true` under 25%.
    pub fn budget_low(&self) -> bool {
        self.budget.saturating_sub(self.spent).saturating_mul(4) < self.budget
    }

    /// Has `author` posted evidence here?
    pub fn has_evidence_by(&self, author: &PrincipalId) -> bool {
        self.evidence.iter().any(|i| &i.author == author)
    }
}

/// Which evidence a rendered card shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Show<'a> {
    /// Only the viewer's own items: a blind first pass.
    OwnOnly(&'a PrincipalId),
    All,
}

/// The card as prompt text. Items above `clearance` are left out, as are
/// other principals' items under [`Show::OwnOnly`]; the count of what was
/// left out is not shown, so a hidden item is not hinted at.
pub fn render(card: &Card, show: Show<'_>, clearance: DataClass) -> String {
    let mut out = format!(
        "CLAIM {} {} score {:+} (accepts at {:+}), budget {}/{}, stakes {}, {}\n",
        card.id,
        card.status.name(),
        card.score,
        card.accept_at,
        card.spent,
        card.budget,
        stakes(card.stakes),
        if card.reversible {
            "reversible"
        } else {
            "irreversible"
        },
    );
    out.push_str(&format!("domains: {}\n", card.domains.join(", ")));
    if let Some(q) = card.answers {
        out.push_str(&format!("answers: {q}\n"));
    }
    if !card.deps.is_empty() {
        out.push_str(&format!("deps: {}\n", ids(&card.deps)));
    }
    if !card.candidates.is_empty() {
        out.push_str(&format!("candidates: {}\n", ids(&card.candidates)));
    }
    out.push_str(&format!("text: {}\n", card.text));
    let shown: Vec<&Item> = card
        .evidence
        .iter()
        .filter(|i| admit(i.class, clearance))
        .filter(|i| match show {
            Show::OwnOnly(me) => &i.author == me,
            Show::All => true,
        })
        .collect();
    match (show, shown.is_empty()) {
        (Show::OwnOnly(_), true) => out.push_str("evidence: hidden for your first pass\n"),
        (_, true) => out.push_str("evidence: none\n"),
        (_, false) => {
            out.push_str("evidence:\n");
            for i in shown {
                out.push_str(&item(i));
            }
        }
    }
    out
}

fn item(i: &Item) -> String {
    let dir = match i.direction {
        Direction::Support => "support",
        Direction::Counter => "counter",
    };
    let kind = match i.kind {
        EvidenceKind::Executed => "executed",
        EvidenceKind::Cited => "cited",
        EvidenceKind::Opinion => "opinion",
    };
    format!(
        "- {} {dir} {kind} {:+} by {}: {}\n",
        i.msg, i.delta, i.author, i.note
    )
}

fn stakes(s: Stakes) -> &'static str {
    match s {
        Stakes::Low => "low",
        Stakes::Medium => "medium",
        Stakes::High => "high",
    }
}

fn ids(ids: &[MsgId]) -> String {
    ids.iter()
        .map(MsgId::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;

    pub fn item(msg: u64, author: &str, direction: Direction, class: DataClass) -> Item {
        Item {
            msg: MsgId(msg),
            author: PrincipalId(author.to_owned()),
            direction,
            kind: EvidenceKind::Cited,
            delta: match direction {
                Direction::Support => 2,
                Direction::Counter => -2,
            },
            note: format!("note {msg}"),
            class,
        }
    }

    pub fn card(evidence: Vec<Item>) -> Card {
        Card {
            id: MsgId(7),
            text: "Hash by size first".to_owned(),
            domains: vec!["dedup".to_owned()],
            status: ClaimStatus::Open,
            score: 0,
            accept_at: 8,
            stakes: Stakes::Medium,
            reversible: true,
            answers: Some(MsgId(3)),
            deps: vec![MsgId(4)],
            spent: 2,
            budget: 16,
            evidence,
            candidates: vec![],
            class: DataClass::Internal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{card, item};
    use super::*;

    fn me() -> PrincipalId {
        PrincipalId("adversary".to_owned())
    }

    #[test]
    fn blind_shows_only_own_items() {
        let c = card(vec![
            item(9, "operator", Direction::Support, DataClass::Internal),
            item(10, "adversary", Direction::Counter, DataClass::Internal),
        ]);
        let blind = render(&c, Show::OwnOnly(&me()), DataClass::Internal);
        assert!(!blind.contains("msg:9"), "{blind}");
        assert!(
            blind.contains("msg:10 counter cited -2 by adversary"),
            "{blind}"
        );
        let informed = render(&c, Show::All, DataClass::Internal);
        assert!(informed.contains("msg:9 support"), "{informed}");
    }

    #[test]
    fn a_blind_card_with_nothing_of_ours_says_hidden() {
        let c = card(vec![item(
            9,
            "operator",
            Direction::Support,
            DataClass::Internal,
        )]);
        let blind = render(&c, Show::OwnOnly(&me()), DataClass::Internal);
        assert!(
            blind.contains("evidence: hidden for your first pass"),
            "{blind}"
        );
        assert!(!blind.contains("note 9"));
    }

    #[test]
    fn items_above_clearance_are_left_out_without_a_trace() {
        let c = card(vec![item(
            9,
            "operator",
            Direction::Counter,
            DataClass::Sensitive,
        )]);
        let shown = render(&c, Show::All, DataClass::Internal);
        assert!(
            !shown.contains("msg:9") && !shown.contains("note 9"),
            "{shown}"
        );
        assert!(shown.contains("evidence: none"));
    }

    #[test]
    fn header_names_the_facts_tiering_uses() {
        let shown = render(&card(vec![]), Show::All, DataClass::Internal);
        assert!(shown.starts_with(
            "CLAIM msg:7 OPEN score +0 (accepts at +8), budget 2/16, stakes medium, reversible\n"
        ));
        assert!(shown.contains("answers: msg:3\ndeps: msg:4\n"));
    }

    #[test]
    fn budget_low_is_under_a_quarter_left() {
        let mut c = card(vec![]);
        c.spent = 12;
        assert!(!c.budget_low(), "4 of 16 left is exactly a quarter");
        c.spent = 13;
        assert!(c.budget_low());
    }
}
