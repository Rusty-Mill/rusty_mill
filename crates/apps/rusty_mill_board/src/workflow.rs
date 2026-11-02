//! The board's workflow: which columns exist, in what order, what each is for, and the moves
//! between them with the capability each needs. Pure data an admin edits (Settings > Columns,
//! kept in `workflow.json`); [`Workflow::default`] is the board's original seven columns.
//!
//! What a column *does* comes from its [`Kind`], not its id, so an admin can add, rename,
//! reorder and remove columns without code changes. Three kinds are anchors with fixed ids
//! that storage relies on ([`Column::BACKLOG`], [`Column::DONE`], [`Column::BLOCKED`]); they
//! may be relabelled and reordered but not removed.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::domain::{BoardError, Capability, Column};

/// Most columns a workflow may have.
pub const MAX_COLUMNS: usize = 24;
/// Most transitions a workflow may have.
pub const MAX_TRANSITIONS: usize = 200;
/// Longest column label, in characters.
pub const MAX_LABEL: usize = 40;

/// What a column is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Collected ideas, not yet approved. Exactly one, id `backlog`.
    Backlog,
    /// Approved work waiting to be picked up (`card_take` enters it into a work column).
    Queue,
    /// Work in progress: the card is claimed, and leaving needs the claim.
    Work,
    /// Shipped or closed. Terminal. Exactly one, id `done`.
    Done,
    /// Stuck; needs a human. Any open card may move here. Exactly one, id `blocked`.
    Blocked,
}

impl Kind {
    /// The id an anchor kind must have; `None` for kinds an admin may add freely.
    fn anchor(self) -> Option<Column> {
        match self {
            Self::Backlog => Some(Column::BACKLOG),
            Self::Done => Some(Column::DONE),
            Self::Blocked => Some(Column::BLOCKED),
            Self::Queue | Self::Work => None,
        }
    }
}

/// One column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColumnDef {
    /// Stable id, stored on cards.
    pub id: Column,
    /// What people see.
    pub label: String,
    /// What the column is for.
    pub kind: Kind,
    /// Capabilities whose holders `card_take` cards from here. Empty: whoever holds the
    /// capability of a transition out of it (other than to Blocked, and other than `Close`).
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub takers: BTreeSet<Capability>,
}

/// A move between two columns and the capability it needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transition {
    /// Source.
    pub from: Column,
    /// Destination.
    pub to: Column,
    /// Needed to make the move.
    pub cap: Capability,
    /// A human approval: also needs `Admin`, even with movement rules off.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub gate: bool,
}

/// The columns and moves, as stored.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    columns: Vec<ColumnDef>,
    transitions: Vec<Transition>,
}

/// A validated workflow: build it with [`Workflow::new`] (or deserialize it, which validates
/// the same way), so a workflow in hand always has its anchors and consistent moves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Raw")]
pub struct Workflow {
    columns: Vec<ColumnDef>,
    transitions: Vec<Transition>,
}

impl TryFrom<Raw> for Workflow {
    type Error = String;
    fn try_from(raw: Raw) -> Result<Self, String> {
        Self::new(raw.columns, raw.transitions)
    }
}

impl Default for Workflow {
    /// The board's original columns and transition table.
    fn default() -> Self {
        use Capability::*;
        let col = |id: Column, label: &str, kind: Kind| ColumnDef {
            id,
            label: label.to_owned(),
            kind,
            takers: BTreeSet::new(),
        };
        let mv = |from: Column, to: Column, cap: Capability| Transition {
            from,
            to,
            cap,
            gate: false,
        };
        Self {
            columns: vec![
                col(Column::BACKLOG, "Backlog", Kind::Backlog),
                col(Column::READY, "Ready", Kind::Queue),
                col(Column::BUILDING, "Building", Kind::Work),
                col(Column::QA, "QA", Kind::Work),
                col(Column::REVIEW, "Review", Kind::Work),
                col(Column::DONE, "Done", Kind::Done),
                col(Column::BLOCKED, "Blocked", Kind::Blocked),
            ],
            transitions: vec![
                mv(Column::BACKLOG, Column::READY, Triage),
                mv(Column::BLOCKED, Column::READY, Triage),
                mv(Column::READY, Column::BUILDING, Build),
                mv(Column::BUILDING, Column::QA, Build),
                mv(Column::QA, Column::BUILDING, Test),
                mv(Column::QA, Column::REVIEW, Test),
                mv(Column::REVIEW, Column::BUILDING, Review),
                mv(Column::REVIEW, Column::DONE, Merge),
                mv(Column::BUILDING, Column::DONE, Close),
                mv(Column::QA, Column::DONE, Close),
            ],
        }
    }
}

impl Workflow {
    /// A workflow of `columns` (in display order) and `transitions`.
    ///
    /// # Errors
    /// The reason, if: a label is blank or too long; an id repeats; an anchor (backlog, done,
    /// blocked) is missing, repeated or has the wrong id; a transition names an unknown
    /// column, goes nowhere, leaves Done, enters Blocked (always allowed, so never listed) or
    /// repeats with the same capability; or there are too many columns or transitions.
    pub fn new(columns: Vec<ColumnDef>, transitions: Vec<Transition>) -> Result<Self, String> {
        if columns.len() > MAX_COLUMNS {
            return Err(format!("at most {MAX_COLUMNS} columns"));
        }
        if transitions.len() > MAX_TRANSITIONS {
            return Err(format!("at most {MAX_TRANSITIONS} transitions"));
        }
        let columns: Vec<ColumnDef> = columns
            .into_iter()
            .map(|c| ColumnDef {
                label: c.label.trim().to_owned(),
                ..c
            })
            .collect();
        check_columns(&columns)?;
        let flow = Self {
            columns,
            transitions: Vec::new(),
        };
        check_transitions(&flow, &transitions)?;
        Ok(Self {
            transitions,
            ..flow
        })
    }

    /// The columns, in display order.
    pub fn columns(&self) -> &[ColumnDef] {
        &self.columns
    }

    /// The transitions, in the order given.
    pub fn transitions(&self) -> &[Transition] {
        &self.transitions
    }

    /// Column `id`, if the workflow has it.
    pub fn column(&self, id: &Column) -> Option<&ColumnDef> {
        self.columns.iter().find(|c| c.id == *id)
    }

    /// What column `id` is for, if the workflow has it.
    pub fn kind(&self, id: &Column) -> Option<Kind> {
        self.column(id).map(|c| c.kind)
    }

    /// The transitions `from` -> `to`: one per capability that may make the move (say, a
    /// gated merge and a `Close` for work merged elsewhere), never empty. `Ok(None)`: no
    /// capability needed (into Blocked).
    ///
    /// # Errors
    /// [`BoardError::Terminal`] out of Done, [`BoardError::NoSuchMove`] for a move the
    /// workflow lacks (including to or from a column it does not have).
    pub fn transition(
        &self,
        from: &Column,
        to: &Column,
    ) -> Result<Option<Vec<&Transition>>, BoardError> {
        if *from == Column::DONE {
            return Err(BoardError::Terminal);
        }
        let no_such = || BoardError::NoSuchMove {
            from: from.clone(),
            to: to.clone(),
        };
        if from == to || self.column(from).is_none() {
            return Err(no_such());
        }
        if self.kind(to) == Some(Kind::Blocked) {
            return Ok(None);
        }
        let found: Vec<&Transition> = self
            .transitions
            .iter()
            .filter(|t| t.from == *from && t.to == *to)
            .collect();
        if found.is_empty() {
            return Err(no_such());
        }
        Ok(Some(found))
    }

    /// The gates a move `from` -> `to` jumps over without taking: forward gates (`g.from`
    /// before `g.to` in display order) with `from` at or before `g.from` and `to` at or after
    /// `g.to`. Blocked has no place in the order, so a card leaving it counts as starting from
    /// the beginning: it crosses every gate up to `to`. Used with movement rules off, so that
    /// skipping columns is no way around a human approval.
    pub fn gates_crossed(&self, from: &Column, to: &Column) -> Vec<&Transition> {
        let rank = |id: &Column| {
            self.columns
                .iter()
                .filter(|c| c.kind != Kind::Blocked)
                .position(|c| c.id == *id)
        };
        let Some(dest) = rank(to) else {
            return Vec::new();
        };
        let start = if self.kind(from) == Some(Kind::Blocked) {
            Some(0)
        } else {
            rank(from)
        };
        let Some(start) = start else {
            return Vec::new();
        };
        self.transitions
            .iter()
            .filter(|g| g.gate)
            .filter(|g| match (rank(&g.from), rank(&g.to)) {
                (Some(f), Some(t)) => f < t && start <= f && dest >= t,
                _ => false,
            })
            .collect()
    }

    /// Columns where work is picked up (queues and work columns), in the order a worker
    /// should look: nearest Done first, so work in progress is finished before more starts.
    pub fn work_queues(&self) -> Vec<Column> {
        self.columns
            .iter()
            .rev()
            .filter(|c| matches!(c.kind, Kind::Queue | Kind::Work))
            .map(|c| c.id.clone())
            .collect()
    }

    /// The work column a card in queue `column` enters when taken: the first transition from
    /// it into a work column. `None` for a column that is not a queue.
    pub fn entry_column(&self, column: &Column) -> Option<&Column> {
        if self.kind(column) != Some(Kind::Queue) {
            return None;
        }
        self.transitions
            .iter()
            .find(|t| t.from == *column && self.kind(&t.to) == Some(Kind::Work))
            .map(|t| &t.to)
    }

    /// Whether someone holding `caps` takes cards from `column` (the test for "this card is
    /// for my role"): one of its [`ColumnDef::takers`], or, when it names none, the capability
    /// of a move out of it. `Close` records work finished elsewhere, so it never makes a card
    /// someone's work.
    pub fn actionable(&self, column: &Column, caps: &BTreeSet<Capability>) -> bool {
        let Some(def) = self.column(column) else {
            return false;
        };
        if !def.takers.is_empty() {
            return !def.takers.is_disjoint(caps);
        }
        self.transitions.iter().any(|t| {
            t.from == *column
                && self.kind(&t.to) != Some(Kind::Blocked)
                && t.cap != Capability::Close
                && caps.contains(&t.cap)
        })
    }
}

fn check_columns(columns: &[ColumnDef]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for c in columns {
        if c.label.is_empty() || c.label.chars().count() > MAX_LABEL {
            return Err(format!(
                "column {}: the label must be 1-{MAX_LABEL} characters",
                c.id
            ));
        }
        if !seen.insert(&c.id) {
            return Err(format!("column {} appears twice", c.id));
        }
        let anchor_id = [Column::BACKLOG, Column::DONE, Column::BLOCKED].contains(&c.id);
        match c.kind.anchor() {
            Some(id) if id != c.id => {
                return Err(format!("only column {id} may be of kind {:?}", c.kind));
            }
            None if anchor_id => {
                return Err(format!("column {} must keep its own kind", c.id));
            }
            _ => {}
        }
    }
    for id in [Column::BACKLOG, Column::DONE, Column::BLOCKED] {
        if !seen.contains(&id) {
            return Err(format!("column {id} cannot be removed"));
        }
    }
    Ok(())
}

fn check_transitions(flow: &Workflow, transitions: &[Transition]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for t in transitions {
        let name = format!("{} -> {}", t.from, t.to);
        if flow.column(&t.from).is_none() || flow.column(&t.to).is_none() {
            return Err(format!("{name}: no such column"));
        }
        if t.from == t.to {
            return Err(format!("{name}: a move must change column"));
        }
        if t.from == Column::DONE {
            return Err(format!("{name}: done cards cannot move"));
        }
        if flow.kind(&t.to) == Some(Kind::Blocked) {
            return Err(format!(
                "{name}: any card may be blocked, so this is never listed"
            ));
        }
        if !seen.insert((&t.from, &t.to, t.cap)) {
            return Err(format!("{name} appears twice"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(c: &[Capability]) -> BTreeSet<Capability> {
        c.iter().copied().collect()
    }

    fn col(id: &str, kind: Kind) -> ColumnDef {
        ColumnDef {
            id: Column::new(id).expect("id"),
            label: id.to_owned(),
            kind,
            takers: BTreeSet::new(),
        }
    }

    fn mv(from: &str, to: &str, cap: Capability) -> Transition {
        Transition {
            from: Column::new(from).expect("from"),
            to: Column::new(to).expect("to"),
            cap,
            gate: false,
        }
    }

    fn anchors() -> Vec<ColumnDef> {
        vec![
            col("backlog", Kind::Backlog),
            col("done", Kind::Done),
            col("blocked", Kind::Blocked),
        ]
    }

    #[test]
    fn default_is_the_original_table() {
        use Capability::*;
        let f = Workflow::default();
        let need = |a: &Column, b: &Column| f.transition(a, b).map(|t| t.map(|t| t[0].cap));
        assert_eq!(need(&Column::BACKLOG, &Column::READY), Ok(Some(Triage)));
        assert_eq!(need(&Column::REVIEW, &Column::DONE), Ok(Some(Merge)));
        assert_eq!(need(&Column::QA, &Column::BLOCKED), Ok(None));
        assert_eq!(
            need(&Column::DONE, &Column::READY),
            Err(BoardError::Terminal)
        );
        assert!(matches!(
            need(&Column::BACKLOG, &Column::DONE),
            Err(BoardError::NoSuchMove { .. })
        ));
        assert_eq!(
            f.work_queues(),
            [Column::REVIEW, Column::QA, Column::BUILDING, Column::READY]
        );
        assert_eq!(f.entry_column(&Column::READY), Some(&Column::BUILDING));
        assert_eq!(f.entry_column(&Column::BUILDING), None);
        let builder = caps(&[Build, Close, Project]);
        assert!(
            f.actionable(&Column::READY, &builder) && f.actionable(&Column::BUILDING, &builder)
        );
        assert!(!f.actionable(&Column::QA, &builder) && !f.actionable(&Column::REVIEW, &builder));
    }

    #[test]
    fn default_round_trips_through_json() {
        let f = Workflow::default();
        let json = serde_json::to_string(&f).expect("json");
        assert_eq!(serde_json::from_str::<Workflow>(&json).expect("parse"), f);
    }

    #[test]
    fn an_added_column_is_used_by_its_kind() {
        let mut cols = anchors();
        cols.insert(1, col("triaged", Kind::Queue));
        cols.insert(2, col("design", Kind::Work));
        let f = Workflow::new(
            cols,
            vec![
                mv("backlog", "triaged", Capability::Triage),
                mv("triaged", "design", Capability::Build),
                mv("design", "done", Capability::Merge),
            ],
        )
        .expect("valid");
        assert_eq!(
            f.work_queues(),
            [
                Column::new("design").expect("id"),
                Column::new("triaged").expect("id")
            ]
        );
        assert_eq!(
            f.entry_column(&Column::new("triaged").expect("id"))
                .map(Column::as_str),
            Some("design")
        );
    }

    #[test]
    fn takers_override_the_derived_ones() {
        let mut work = col("work", Kind::Work);
        work.takers = caps(&[Capability::Test]);
        let mut cols = anchors();
        cols.insert(1, work);
        let f = Workflow::new(cols, vec![mv("work", "done", Capability::Review)]).expect("valid");
        let id = Column::new("work").expect("id");
        assert!(f.actionable(&id, &caps(&[Capability::Test])));
        assert!(!f.actionable(&id, &caps(&[Capability::Review])));
    }

    #[test]
    fn invalid_workflows_are_refused_with_the_reason() {
        let refused = |cols: Vec<ColumnDef>, ts: Vec<Transition>| {
            Workflow::new(cols, ts).expect_err("refused")
        };
        assert!(refused(anchors()[..2].to_vec(), vec![]).contains("blocked cannot be removed"));
        let mut twice = anchors();
        twice.push(col("backlog", Kind::Backlog));
        assert!(refused(twice, vec![]).contains("twice"));
        let mut wrong = anchors();
        wrong.push(col("later", Kind::Backlog));
        assert!(refused(wrong, vec![]).contains("only column backlog"));
        let mut relabel = anchors();
        relabel[1].kind = Kind::Work;
        assert!(refused(relabel, vec![]).contains("own kind"));
        let mut blank = anchors();
        blank[0].label = "  ".into();
        assert!(refused(blank, vec![]).contains("label"));
        assert!(
            refused(
                anchors(),
                vec![mv("backlog", "nowhere", Capability::Triage)]
            )
            .contains("no such column")
        );
        assert!(
            refused(anchors(), vec![mv("done", "backlog", Capability::Triage)])
                .contains("cannot move")
        );
        assert!(
            refused(
                anchors(),
                vec![mv("backlog", "blocked", Capability::Triage)]
            )
            .contains("never listed")
        );
        assert!(
            refused(
                anchors(),
                vec![mv("backlog", "backlog", Capability::Triage)]
            )
            .contains("change column")
        );
        let dup = vec![
            mv("backlog", "done", Capability::Close),
            mv("backlog", "done", Capability::Close),
        ];
        assert!(refused(anchors(), dup).contains("twice"));
    }

    #[test]
    fn a_move_may_be_made_with_either_of_two_capabilities() {
        let mut merge = mv("backlog", "done", Capability::Merge);
        merge.gate = true;
        let f = Workflow::new(
            anchors(),
            vec![merge, mv("backlog", "done", Capability::Close)],
        )
        .expect("two ways to make one move");
        let caps: Vec<Capability> = f
            .transition(&Column::BACKLOG, &Column::DONE)
            .expect("move")
            .expect("needs a capability")
            .iter()
            .map(|t| t.cap)
            .collect();
        assert_eq!(caps, [Capability::Merge, Capability::Close]);
    }

    #[test]
    fn a_stored_workflow_is_validated_when_read() {
        let json =
            r#"{"columns":[{"id":"backlog","label":"B","kind":"backlog"}],"transitions":[]}"#;
        assert!(serde_json::from_str::<Workflow>(json).is_err());
        assert!(
            serde_json::from_str::<Workflow>(
                r#"{"columns":[{"id":"Bad","label":"x","kind":"queue"}],"transitions":[]}"#
            )
            .is_err()
        );
    }
}
