//! The on-disk shape of a goal's assignment record: its orchestrator
//! assignments, the parent generations that authorized them, and its
//! execution links.
//!
//! A separate record from the goal snapshot, so the v1 snapshot layout is
//! unchanged and a generation can be made durable before any host command
//! runs under it. Encoded by the engine's positional codec like the
//! snapshot: any layout change needs a new schema tag.

use orch_core::factory::{
    Assignment, Delegation, ExecutionLink, ExecutionState, Factory, Grant, Identity,
};
use orch_core::GoalId;
use rusty_multimodal_db_engine::generic::traits::{
    IndexedField, Record, ScannableField, SchemaTag,
};
use serde::{Deserialize, Serialize};

use crate::record::{key, ById, Revision};
use crate::StoreError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssignmentRecord {
    id: i64,
    pub revision: i64,
    assignments: Vec<AssignmentRow>,
    links: Vec<LinkRow>,
}

impl Record for AssignmentRecord {
    type Id = i64;
    fn id(&self) -> i64 {
        self.id
    }
}

impl SchemaTag for AssignmentRecord {
    const SCHEMA_TAG: &'static str = "rusty_orch::AssignmentRecord::v1";
}

impl IndexedField<ById> for AssignmentRecord {
    type IndexValue = i64;
    fn indexed_value(&self) -> &i64 {
        &self.id
    }
}

impl ScannableField<Revision> for AssignmentRecord {
    type ScanValue = i64;
    fn scannable_value(&self) -> i64 {
        self.revision
    }
    fn set_scannable_value(&mut self, value: i64) {
        self.revision = value;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AssignmentRow {
    id: String,
    generation: u64,
    identity: String,
    target: String,
    parent: Option<(String, u64)>,
    projects: Vec<String>,
    delegation_budget: u32,
    call_budget: u32,
    active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LinkRow {
    card: String,
    execution: String,
    goal: String,
    bbp_task: String,
    attempt: String,
    assignment: String,
    generation: u64,
    state: StateRow,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum StateRow {
    Running,
    Waiting,
    Completed,
    Failed,
    Cancelled,
}

impl AssignmentRecord {
    pub fn from_factory(goal: GoalId, factory: &Factory, revision: u64) -> Self {
        let assignments = factory
            .delegations()
            .into_iter()
            .map(|d| {
                let a = d.assignment;
                AssignmentRow {
                    parent: a.parent.zip(d.parent_generation),
                    id: a.id,
                    generation: a.generation,
                    identity: a.identity.id,
                    target: a.identity.target,
                    projects: a.grant.projects,
                    delegation_budget: a.grant.delegation_budget,
                    call_budget: a.grant.call_budget,
                    active: a.active,
                }
            })
            .collect();
        let links = factory
            .links()
            .map(|l| LinkRow {
                card: l.card.clone(),
                execution: l.execution.clone(),
                goal: l.goal.clone(),
                bbp_task: l.bbp_task.clone(),
                attempt: l.attempt.clone(),
                assignment: l.assignment.clone(),
                generation: l.generation,
                state: match l.state {
                    ExecutionState::Running => StateRow::Running,
                    ExecutionState::Waiting => StateRow::Waiting,
                    ExecutionState::Completed => StateRow::Completed,
                    ExecutionState::Failed => StateRow::Failed,
                    ExecutionState::Cancelled => StateRow::Cancelled,
                },
            })
            .collect();
        Self {
            id: key(goal),
            revision: revision as i64,
            assignments,
            links,
        }
    }

    /// Rebuild through [`Factory::restore`], which re-checks every
    /// invariant instead of trusting the record.
    pub fn into_factory(self) -> Result<Factory, StoreError> {
        let delegations = self
            .assignments
            .into_iter()
            .map(|row| {
                let (parent, parent_generation) = row.parent.unzip();
                Delegation {
                    assignment: Assignment {
                        id: row.id,
                        generation: row.generation,
                        identity: Identity {
                            id: row.identity,
                            target: row.target,
                        },
                        parent,
                        grant: Grant {
                            projects: row.projects,
                            delegation_budget: row.delegation_budget,
                            call_budget: row.call_budget,
                        },
                        active: row.active,
                    },
                    parent_generation,
                }
            })
            .collect();
        let links = self
            .links
            .into_iter()
            .map(|row| ExecutionLink {
                card: row.card,
                execution: row.execution,
                goal: row.goal,
                bbp_task: row.bbp_task,
                attempt: row.attempt,
                assignment: row.assignment,
                generation: row.generation,
                state: match row.state {
                    StateRow::Running => ExecutionState::Running,
                    StateRow::Waiting => ExecutionState::Waiting,
                    StateRow::Completed => ExecutionState::Completed,
                    StateRow::Failed => ExecutionState::Failed,
                    StateRow::Cancelled => ExecutionState::Cancelled,
                },
            })
            .collect();
        Factory::restore(delegations, links)
            .map_err(|e| StoreError::Corrupt(format!("assignments: {e:?}")))
    }
}
