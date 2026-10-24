//! The on-disk shape of a goal's assignment record: its orchestrator
//! assignments, the parent generations that authorized them, and its
//! execution links.
//!
//! A separate record from the goal snapshot, so the v1 snapshot layout is
//! unchanged and a generation can be made durable before any host command
//! runs under it. Encoded by the engine's positional codec like the
//! snapshot: any layout change needs a new schema tag.
//!
//! v2 adds each grant's lease and lives in its own file. A goal saved only
//! as v1 is still read, as unleased, which is what v1 grants were; every
//! save writes v2, so the v1 file is never written again.

use orch_core::factory::{
    Assignment, Delegation, ExecutionLink, ExecutionState, Factory, Grant, Identity, Lease,
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
    const SCHEMA_TAG: &'static str = "rusty_orch::AssignmentRecord::v2";
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
    lease: Option<LeaseRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LeaseRow {
    read: Vec<String>,
    write: Vec<String>,
    expires: Option<u64>,
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
                    lease: a.grant.lease.map(|l| LeaseRow {
                        read: l.read,
                        write: l.write,
                        expires: l.expires,
                    }),
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
                            lease: row.lease.map(|l| Lease {
                                read: l.read,
                                write: l.write,
                                expires: l.expires,
                            }),
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

/// The record before leases: read for goals not saved since, never written.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegacyAssignmentRecord {
    id: i64,
    pub revision: i64,
    assignments: Vec<LegacyAssignmentRow>,
    links: Vec<LinkRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacyAssignmentRow {
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

impl Record for LegacyAssignmentRecord {
    type Id = i64;
    fn id(&self) -> i64 {
        self.id
    }
}

impl SchemaTag for LegacyAssignmentRecord {
    const SCHEMA_TAG: &'static str = "rusty_orch::AssignmentRecord::v1";
}

impl IndexedField<ById> for LegacyAssignmentRecord {
    type IndexValue = i64;
    fn indexed_value(&self) -> &i64 {
        &self.id
    }
}

impl ScannableField<Revision> for LegacyAssignmentRecord {
    type ScanValue = i64;
    fn scannable_value(&self) -> i64 {
        self.revision
    }
    fn set_scannable_value(&mut self, value: i64) {
        self.revision = value;
    }
}

impl From<LegacyAssignmentRecord> for AssignmentRecord {
    fn from(legacy: LegacyAssignmentRecord) -> Self {
        let assignments = legacy
            .assignments
            .into_iter()
            .map(|row| AssignmentRow {
                id: row.id,
                generation: row.generation,
                identity: row.identity,
                target: row.target,
                parent: row.parent,
                projects: row.projects,
                delegation_budget: row.delegation_budget,
                call_budget: row.call_budget,
                active: row.active,
                lease: None,
            })
            .collect();
        Self {
            id: legacy.id,
            revision: legacy.revision,
            assignments,
            links: legacy.links,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;
    use rusty_multimodal_db_engine::generic::query::GetById;
    use rusty_multimodal_db_engine::generic::GenericMmapStore;
    use rusty_multimodal_db_engine::test_support::fresh_temp_dir;

    #[test]
    fn a_v1_record_loads_unleased_and_the_next_save_writes_v2() {
        let dir = fresh_temp_dir("orch_store_legacy").expect("temp dir");
        let goal = GoalId::from_raw(1);
        let legacy = LegacyAssignmentRecord {
            id: key(goal),
            revision: 3,
            assignments: vec![LegacyAssignmentRow {
                id: "root".into(),
                generation: 2,
                identity: "orch".into(),
                target: "rusty_orch run".into(),
                parent: None,
                projects: vec![],
                delegation_budget: 0,
                call_budget: 4,
                active: true,
            }],
            links: vec![],
        };
        let path = dir.join("assignments.mmap");
        drop(GenericMmapStore::<_, ById, Revision>::create(vec![legacy], &path).expect("v1"));
        let before = std::fs::read(&path).expect("v1 bytes");

        let mut store = Store::open(&dir).expect("open");
        let mut factory = store.load_factory(goal).expect("v1 loads");
        let root = factory.assignment("root").expect("root");
        assert_eq!((root.generation, &root.grant.lease), (2, &None));
        let lease = Lease {
            read: vec!["/src".into()],
            write: vec![],
            expires: Some(9),
        };
        let mut grant = root.grant.clone();
        grant.lease = Some(lease.clone());
        let identity = root.identity.clone();
        assert_eq!(factory.take_over("root", identity, grant), Ok(3));
        store.save_factory(goal, &factory).expect("save v2");
        assert_eq!(
            store.assignments.get(key(goal)).map(|r| r.revision),
            Some(4)
        );
        drop(store);

        assert_eq!(std::fs::read(&path).expect("v1 bytes"), before);
        let loaded = Store::open(&dir).expect("reopen").load_factory(goal);
        let loaded = loaded.expect("v2 loads");
        assert_eq!(loaded, factory);
        assert_eq!(
            loaded
                .assignment("root")
                .and_then(|a| a.grant.lease.clone()),
            Some(lease)
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
