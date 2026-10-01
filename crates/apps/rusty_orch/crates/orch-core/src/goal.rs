//! Validated goal contracts.
//!
//! A [`GoalDraft`] is what arrives at the boundary (CLI, JSON, chat). It is
//! loose on purpose. A [`Goal`] can only be built from a draft via
//! [`Goal::try_from`], so every `Goal` in the system is guaranteed to have an
//! outcome, at least one acceptance criterion, an explicit out-of-scope
//! statement, and a complete budget.

use std::fmt;
use std::num::{NonZeroU32, NonZeroU64};
use std::time::Duration;

use crate::Text;

/// What the orchestrator does when it hits an unresolved question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopRule {
    /// Halt and report back for a decision.
    Checkpoint,
    /// Continue, recording assumptions on the blackboard.
    BestEffort,
}

/// Spend ceiling for one goal. Both limits are non-zero by construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    wall_clock: NonZeroU64,
    max_calls: NonZeroU32,
    stop: StopRule,
}

impl Budget {
    /// Maximum wall-clock time for the whole goal.
    pub fn wall_clock(&self) -> Duration {
        Duration::from_secs(self.wall_clock.get())
    }

    /// Maximum number of model calls across all agents.
    pub fn max_calls(&self) -> NonZeroU32 {
        self.max_calls
    }

    /// Behaviour on an unresolved question.
    pub fn stop(&self) -> StopRule {
        self.stop
    }
}

/// Unvalidated goal as received at the boundary.
///
/// `out_of_scope` is an `Option` so "not stated" (`None`, rejected) differs
/// from "explicitly nothing excluded" (`Some(vec![])`, accepted).
#[derive(Debug, Clone, Default)]
pub struct GoalDraft {
    pub outcome: Option<String>,
    pub done_when: Vec<String>,
    pub in_scope: Vec<String>,
    pub out_of_scope: Option<Vec<String>>,
    pub constraints: Vec<String>,
    pub refs: Vec<String>,
    pub wall_clock_secs: Option<u64>,
    pub max_calls: Option<u32>,
    pub stop: Option<StopRule>,
}

/// A validated goal contract. Construct with [`Goal::try_from`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Goal {
    outcome: Text,
    done_when: Vec<Text>,
    in_scope: Vec<Text>,
    out_of_scope: Vec<Text>,
    constraints: Vec<Text>,
    refs: Vec<Text>,
    budget: Budget,
}

impl Goal {
    /// One-sentence outcome.
    pub fn outcome(&self) -> &Text {
        &self.outcome
    }

    /// Acceptance criteria; never empty.
    pub fn done_when(&self) -> &[Text] {
        &self.done_when
    }

    /// Explicitly in-scope items; may be empty.
    pub fn in_scope(&self) -> &[Text] {
        &self.in_scope
    }

    /// Explicitly out-of-scope items; empty only if stated as such.
    pub fn out_of_scope(&self) -> &[Text] {
        &self.out_of_scope
    }

    /// Non-negotiable constraints.
    pub fn constraints(&self) -> &[Text] {
        &self.constraints
    }

    /// Pointers (paths, ADRs, blackboard IDs) — never pasted content.
    pub fn refs(&self) -> &[Text] {
        &self.refs
    }

    /// Spend ceiling and stop rule.
    pub fn budget(&self) -> &Budget {
        &self.budget
    }
}

/// A field of the goal template, named as the user writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Outcome,
    DoneWhen,
    InScope,
    OutOfScope,
    Constraints,
    Refs,
    WallClock,
    MaxCalls,
    StopRule,
}

impl fmt::Display for Field {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Outcome => "GOAL",
            Self::DoneWhen => "DONE WHEN",
            Self::InScope => "SCOPE (in)",
            Self::OutOfScope => "SCOPE (out)",
            Self::Constraints => "CONSTRAINTS",
            Self::Refs => "REFS",
            Self::WallClock => "BUDGET (wall clock)",
            Self::MaxCalls => "BUDGET (max calls)",
            Self::StopRule => "BUDGET (stop rule)",
        };
        f.write_str(label)
    }
}

/// One reason a draft was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    /// Required field absent or blank.
    Missing(Field),
    /// A list entry at `index` is blank.
    Blank { field: Field, index: usize },
    /// A budget limit was zero.
    Zero(Field),
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(field) => write!(f, "{field} is required"),
            Self::Blank { field, index } => write!(f, "{field} entry {index} is blank"),
            Self::Zero(field) => write!(f, "{field} must be greater than zero"),
        }
    }
}

/// All problems found in a draft, not just the first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalError(pub Vec<Problem>);

impl fmt::Display for GoalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let lines: Vec<String> = self.0.iter().map(Problem::to_string).collect();
        write!(f, "goal rejected: {}", lines.join("; "))
    }
}

impl std::error::Error for GoalError {}

/// Accumulates problems so the user sees every fix needed in one pass.
#[derive(Default)]
struct Checker {
    problems: Vec<Problem>,
}

impl Checker {
    fn require<T>(&mut self, value: Option<T>, problem: Problem) -> Option<T> {
        if value.is_none() {
            self.problems.push(problem);
        }
        value
    }

    fn list(&mut self, field: Field, raw: &[String], required: bool) -> Vec<Text> {
        if required && raw.is_empty() {
            self.problems.push(Problem::Missing(field));
        }
        let mut out = Vec::with_capacity(raw.len());
        for (index, entry) in raw.iter().enumerate() {
            match Text::new(entry) {
                Some(text) => out.push(text),
                None => self.problems.push(Problem::Blank { field, index }),
            }
        }
        out
    }
}

impl TryFrom<GoalDraft> for Goal {
    type Error = GoalError;

    fn try_from(draft: GoalDraft) -> Result<Self, Self::Error> {
        let mut c = Checker::default();

        let outcome = c.require(
            draft.outcome.as_deref().and_then(Text::new),
            Problem::Missing(Field::Outcome),
        );
        let done_when = c.list(Field::DoneWhen, &draft.done_when, true);
        let in_scope = c.list(Field::InScope, &draft.in_scope, false);
        let out_of_scope = match c.require(draft.out_of_scope, Problem::Missing(Field::OutOfScope))
        {
            Some(items) => c.list(Field::OutOfScope, &items, false),
            None => Vec::new(),
        };
        let constraints = c.list(Field::Constraints, &draft.constraints, false);
        let refs = c.list(Field::Refs, &draft.refs, false);

        let wall_clock = c
            .require(draft.wall_clock_secs, Problem::Missing(Field::WallClock))
            .and_then(|secs| c.require(NonZeroU64::new(secs), Problem::Zero(Field::WallClock)));
        let max_calls = c
            .require(draft.max_calls, Problem::Missing(Field::MaxCalls))
            .and_then(|n| c.require(NonZeroU32::new(n), Problem::Zero(Field::MaxCalls)));
        let stop = c.require(draft.stop, Problem::Missing(Field::StopRule));

        match (outcome, wall_clock, max_calls, stop) {
            (Some(outcome), Some(wall_clock), Some(max_calls), Some(stop))
                if c.problems.is_empty() =>
            {
                Ok(Self {
                    outcome,
                    done_when,
                    in_scope,
                    out_of_scope,
                    constraints,
                    refs,
                    budget: Budget {
                        wall_clock,
                        max_calls,
                        stop,
                    },
                })
            }
            _ => Err(GoalError(c.problems)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    fn valid() -> GoalDraft {
        GoalDraft {
            outcome: Some("  Add CSV export to the column view.  ".into()),
            done_when: strings(&[
                "export_csv round-trips fixture",
                "tests cover empty + unicode",
            ]),
            in_scope: strings(&["column view"]),
            out_of_scope: Some(strings(&["row view", "graph view"])),
            constraints: strings(&["no unwrap"]),
            refs: strings(&["crates/core/src/views/column.rs", "board:mmdb"]),
            wall_clock_secs: Some(2700),
            max_calls: Some(12),
            stop: Some(StopRule::Checkpoint),
        }
    }

    fn problems(draft: GoalDraft) -> Vec<Problem> {
        Goal::try_from(draft)
            .expect_err("draft should be rejected")
            .0
    }

    #[test]
    fn accepts_complete_draft_and_trims() {
        let goal = Goal::try_from(valid()).expect("valid draft");
        assert_eq!(
            goal.outcome().as_str(),
            "Add CSV export to the column view."
        );
        assert_eq!(goal.done_when().len(), 2);
        assert_eq!(goal.budget().wall_clock(), Duration::from_secs(2700));
        assert_eq!(goal.budget().max_calls().get(), 12);
        assert_eq!(goal.budget().stop(), StopRule::Checkpoint);
    }

    #[test]
    fn empty_draft_reports_every_required_field() {
        assert_eq!(
            problems(GoalDraft::default()),
            vec![
                Problem::Missing(Field::Outcome),
                Problem::Missing(Field::DoneWhen),
                Problem::Missing(Field::OutOfScope),
                Problem::Missing(Field::WallClock),
                Problem::Missing(Field::MaxCalls),
                Problem::Missing(Field::StopRule),
            ]
        );
    }

    #[test]
    fn rejects_missing_done_when() {
        let draft = GoalDraft {
            done_when: vec![],
            ..valid()
        };
        assert_eq!(problems(draft), vec![Problem::Missing(Field::DoneWhen)]);
    }

    #[test]
    fn rejects_missing_budget() {
        let draft = GoalDraft {
            wall_clock_secs: None,
            max_calls: None,
            stop: None,
            ..valid()
        };
        assert_eq!(
            problems(draft),
            vec![
                Problem::Missing(Field::WallClock),
                Problem::Missing(Field::MaxCalls),
                Problem::Missing(Field::StopRule),
            ]
        );
    }

    #[test]
    fn rejects_zero_limits() {
        let draft = GoalDraft {
            wall_clock_secs: Some(0),
            max_calls: Some(0),
            ..valid()
        };
        assert_eq!(
            problems(draft),
            vec![
                Problem::Zero(Field::WallClock),
                Problem::Zero(Field::MaxCalls)
            ]
        );
    }

    #[test]
    fn whitespace_outcome_counts_as_missing() {
        let draft = GoalDraft {
            outcome: Some("   ".into()),
            ..valid()
        };
        assert_eq!(problems(draft), vec![Problem::Missing(Field::Outcome)]);
    }

    #[test]
    fn reports_blank_list_entry_by_index() {
        let draft = GoalDraft {
            done_when: strings(&["ok", " ", "ok"]),
            ..valid()
        };
        assert_eq!(
            problems(draft),
            vec![Problem::Blank {
                field: Field::DoneWhen,
                index: 1
            }]
        );
    }

    #[test]
    fn unstated_out_of_scope_rejected_but_explicit_empty_accepted() {
        let unstated = GoalDraft {
            out_of_scope: None,
            ..valid()
        };
        assert_eq!(
            problems(unstated),
            vec![Problem::Missing(Field::OutOfScope)]
        );

        let explicit = GoalDraft {
            out_of_scope: Some(vec![]),
            ..valid()
        };
        assert!(Goal::try_from(explicit)
            .expect("explicit empty")
            .out_of_scope()
            .is_empty());
    }

    #[test]
    fn error_message_uses_template_labels() {
        let draft = GoalDraft {
            done_when: vec![],
            max_calls: Some(0),
            ..valid()
        };
        let msg = Goal::try_from(draft).expect_err("rejected").to_string();
        assert_eq!(
            msg,
            "goal rejected: DONE WHEN is required; BUDGET (max calls) must be greater than zero"
        );
    }
}
