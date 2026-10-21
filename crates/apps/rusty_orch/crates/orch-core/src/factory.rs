//! Durable factory identities and the card-to-execution reconciliation model.
//!
//! The Millboard adapter persists [`ExecutionLink`] beside its card and restores
//! this aggregate before it asks a task host to open work.  This crate does not
//! know a provider, a process, or a board transport: those are deliberately
//! adapter concerns.

use std::collections::{BTreeMap, BTreeSet};

/// A provider-independent identity selected by configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// Stable configuration key, not a provider session or model name.
    pub id: String,
    /// The executable/model/session descriptor interpreted by an adapter.
    pub target: String,
}

/// The authority and finite resources granted to an assignment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    /// Projects in which this assignment may work.
    pub projects: Vec<String>,
    /// Maximum descendants, including indirect descendants.
    pub delegation_budget: u32,
    /// Maximum calls attributable to this assignment.
    pub call_budget: u32,
}

/// A configured project, task, or worker orchestrator assignment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    /// Stable assignment id.
    pub id: String,
    /// Incremented whenever this assignment is replaced or revoked.
    pub generation: u64,
    /// Selected identity; it is intentionally separate from assignment id.
    pub identity: Identity,
    /// Parent assignment, when this was delegated.
    pub parent: Option<String>,
    /// Authority granted to this assignment.
    pub grant: Grant,
    /// A retired assignment cannot accept commands or children.
    pub active: bool,
}

/// Stable identifiers spanning a Millboard card, Orch goal, BBP task and host attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionLink {
    /// Millboard's card UUID, represented without imposing Millboard on core.
    pub card: String,
    /// Stable logical execution id. Repeated submissions retain this value.
    pub execution: String,
    /// Orch goal id.
    pub goal: String,
    /// BBP task id.
    pub bbp_task: String,
    /// Recoverable task-host attempt id.
    pub attempt: String,
    /// Assignment id and generation that own the attempt.
    pub assignment: String,
    /// Generation the host must present for a command.
    pub generation: u64,
    /// Reconciled lifecycle state.
    pub state: ExecutionState,
}

/// The common terminal vocabulary for Millboard and Orch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionState {
    /// Created or recovered and eligible for host polling.
    Running,
    /// A human/BBP gate is waiting.
    Waiting,
    /// Finished successfully.
    Completed,
    /// Failed without a recoverable host state.
    Failed,
    /// Cancelled by the card owner or host.
    Cancelled,
}

impl ExecutionState {
    /// Whether no more commands may be accepted for this execution.
    pub fn terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

/// Why a factory operation was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactoryError {
    /// A required id was empty or duplicated.
    Invalid(&'static str),
    /// The requested assignment does not exist.
    UnknownAssignment,
    /// A command carried an old assignment generation.
    StaleGeneration,
    /// An assignment or ancestor has been retired.
    Retired,
    /// A child exceeds its parent's project, call, or delegation grant.
    DelegationOutsideGrant,
    /// A card already has a different logical execution.
    CardAlreadyLinked,
    /// The card's existing execution cannot be replaced.
    Terminal,
}

/// In-memory aggregate rebuilt from the durable card/execution records by a storage adapter.
#[derive(Debug, Default)]
pub struct Factory {
    assignments: BTreeMap<String, Assignment>,
    /// Generation of the parent which authorized each child. Kept out of the
    /// public assignment DTO so adapters can continue to construct it directly.
    parent_generations: BTreeMap<String, u64>,
    links: BTreeMap<String, ExecutionLink>,
}

impl Factory {
    /// Register a top-level configurable project or task orchestrator.
    pub fn assign(&mut self, assignment: Assignment) -> Result<(), FactoryError> {
        if assignment.id.is_empty()
            || assignment.identity.id.is_empty()
            || assignment.generation == 0
        {
            return Err(FactoryError::Invalid(
                "assignment id, identity id, and generation are required",
            ));
        }
        if assignment.parent.is_some() {
            return Err(FactoryError::Invalid("use delegate for child assignments"));
        }
        if self.assignments.contains_key(&assignment.id) {
            return Err(FactoryError::Invalid("assignment id already exists"));
        }
        self.assignments.insert(assignment.id.clone(), assignment);
        Ok(())
    }

    /// Replace an assignment, retiring the former generation atomically from the caller's view.
    pub fn replace(&mut self, assignment: Assignment) -> Result<(), FactoryError> {
        let previous = self
            .assignments
            .get(&assignment.id)
            .ok_or(FactoryError::UnknownAssignment)?;
        if assignment.generation <= previous.generation {
            return Err(FactoryError::StaleGeneration);
        }
        if assignment.parent != previous.parent {
            return Err(FactoryError::Invalid("replacement changes parent"));
        }
        if let Some(parent) = assignment.parent.as_deref() {
            self.validate_child_grant(parent, &assignment, Some(&assignment.id))?;
        }
        self.validate_reserved_capacity(&assignment)?;
        self.assignments.insert(assignment.id.clone(), assignment);
        Ok(())
    }

    /// Retire an assignment by replacing its generation. Descendants become unusable too.
    pub fn retire(&mut self, id: &str, generation: u64) -> Result<(), FactoryError> {
        let assignment = self
            .assignments
            .get_mut(id)
            .ok_or(FactoryError::UnknownAssignment)?;
        if assignment.generation != generation {
            return Err(FactoryError::StaleGeneration);
        }
        assignment.generation += 1;
        assignment.active = false;
        Ok(())
    }

    /// Add a worker assignment bounded by its active parent grant.
    pub fn delegate(
        &mut self,
        parent: &str,
        generation: u64,
        child: Assignment,
    ) -> Result<(), FactoryError> {
        self.authorize(parent, generation)?;
        if child.id.is_empty() || child.generation == 0 {
            return Err(FactoryError::Invalid(
                "child id and generation are required",
            ));
        }
        if self.assignments.contains_key(&child.id) {
            return Err(FactoryError::Invalid("assignment id already exists"));
        }
        self.validate_child_grant(parent, &child, None)?;
        self.validate_reserved_capacity(&child)?;
        let child_id = child.id.clone();
        self.assignments.insert(child_id.clone(), child);
        self.parent_generations.insert(child_id, generation);
        Ok(())
    }

    /// Initiate a card exactly once or adopt its previous durable execution after restart.
    pub fn initiate_or_adopt(
        &mut self,
        link: ExecutionLink,
    ) -> Result<ExecutionLink, FactoryError> {
        if let Some(existing) = self.links.get(&link.card) {
            if existing.execution != link.execution
                || existing.goal != link.goal
                || existing.bbp_task != link.bbp_task
                || existing.attempt != link.attempt
                || existing.assignment != link.assignment
            {
                return Err(FactoryError::CardAlreadyLinked);
            }
            if existing.generation == link.generation {
                return Ok(existing.clone());
            }
            if existing.state.terminal() || link.generation < existing.generation {
                return Err(FactoryError::StaleGeneration);
            }
            self.authorize(&link.assignment, link.generation)?;
            let existing = self
                .links
                .get_mut(&link.card)
                .ok_or(FactoryError::Invalid("card has no execution"))?;
            existing.generation = link.generation;
            return Ok(existing.clone());
        }
        self.authorize(&link.assignment, link.generation)?;
        if link.card.is_empty()
            || link.execution.is_empty()
            || link.goal.is_empty()
            || link.bbp_task.is_empty()
            || link.attempt.is_empty()
        {
            return Err(FactoryError::Invalid("execution identifiers are required"));
        }
        self.links.insert(link.card.clone(), link.clone());
        Ok(link)
    }

    /// Reconcile a host observation. Terminal state is monotonic, and stale commands fail closed.
    pub fn reconcile(
        &mut self,
        card: &str,
        assignment: &str,
        generation: u64,
        state: ExecutionState,
    ) -> Result<ExecutionLink, FactoryError> {
        self.authorize(assignment, generation)?;
        let link = self
            .links
            .get_mut(card)
            .ok_or(FactoryError::Invalid("card has no execution"))?;
        if link.assignment != assignment || link.generation != generation {
            return Err(FactoryError::StaleGeneration);
        }
        if link.state.terminal() && link.state != state {
            return Err(FactoryError::Terminal);
        }
        link.state = state;
        Ok(link.clone())
    }

    /// The durable link to save with a card.
    pub fn link(&self, card: &str) -> Option<&ExecutionLink> {
        self.links.get(card)
    }

    fn authorize(&self, id: &str, generation: u64) -> Result<(), FactoryError> {
        let assignment = self
            .assignments
            .get(id)
            .ok_or(FactoryError::UnknownAssignment)?;
        if assignment.generation != generation {
            return Err(FactoryError::StaleGeneration);
        }
        let mut current = assignment;
        let mut seen = BTreeSet::new();
        loop {
            if !seen.insert(current.id.as_str()) {
                return Err(FactoryError::Invalid(
                    "assignment ancestry contains a cycle",
                ));
            }
            if !current.active {
                return Err(FactoryError::Retired);
            }
            let Some(parent) = current.parent.as_deref() else {
                return Ok(());
            };
            let parent_assignment = self.assignments.get(parent).ok_or(FactoryError::Retired)?;
            if self.parent_generations.get(&current.id) != Some(&parent_assignment.generation) {
                return Err(FactoryError::StaleGeneration);
            }
            current = parent_assignment;
        }
    }

    fn validate_child_grant(
        &self,
        parent: &str,
        child: &Assignment,
        replacing: Option<&str>,
    ) -> Result<(), FactoryError> {
        if child.parent.as_deref() != Some(parent) || child.id == parent {
            return Err(FactoryError::DelegationOutsideGrant);
        }
        let parent_assignment = self
            .assignments
            .get(parent)
            .ok_or(FactoryError::UnknownAssignment)?;
        let (mut descendants, mut calls) = (0_u32, 0_u32);
        for existing in self.assignments.values().filter(|assignment| {
            assignment.parent.as_deref() == Some(parent)
                && replacing != Some(assignment.id.as_str())
        }) {
            descendants = descendants
                .checked_add(1 + existing.grant.delegation_budget)
                .ok_or(FactoryError::DelegationOutsideGrant)?;
            calls = calls
                .checked_add(existing.grant.call_budget)
                .ok_or(FactoryError::DelegationOutsideGrant)?;
        }
        descendants = descendants
            .checked_add(1 + child.grant.delegation_budget)
            .ok_or(FactoryError::DelegationOutsideGrant)?;
        calls = calls
            .checked_add(child.grant.call_budget)
            .ok_or(FactoryError::DelegationOutsideGrant)?;
        if descendants > parent_assignment.grant.delegation_budget
            || calls > parent_assignment.grant.call_budget
            || child
                .grant
                .projects
                .iter()
                .any(|project| !parent_assignment.grant.projects.contains(project))
        {
            return Err(FactoryError::DelegationOutsideGrant);
        }
        Ok(())
    }

    fn validate_reserved_capacity(&self, assignment: &Assignment) -> Result<(), FactoryError> {
        let (mut descendants, mut calls) = (0_u32, 0_u32);
        for child in self
            .assignments
            .values()
            .filter(|child| child.parent.as_deref() == Some(assignment.id.as_str()))
        {
            descendants = descendants
                .checked_add(1 + child.grant.delegation_budget)
                .ok_or(FactoryError::DelegationOutsideGrant)?;
            calls = calls
                .checked_add(child.grant.call_budget)
                .ok_or(FactoryError::DelegationOutsideGrant)?;
        }
        if descendants > assignment.grant.delegation_budget || calls > assignment.grant.call_budget
        {
            return Err(FactoryError::DelegationOutsideGrant);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn assignment(
        id: &str,
        parent: Option<&str>,
        generation: u64,
        projects: &[&str],
        delegates: u32,
        calls: u32,
    ) -> Assignment {
        Assignment {
            id: id.into(),
            generation,
            identity: Identity {
                id: format!("{id}-identity"),
                target: "configured-adapter".into(),
            },
            parent: parent.map(str::to_owned),
            grant: Grant {
                projects: projects.iter().map(|s| (*s).into()).collect(),
                delegation_budget: delegates,
                call_budget: calls,
            },
            active: true,
        }
    }
    fn link(generation: u64) -> ExecutionLink {
        ExecutionLink {
            card: "card-1".into(),
            execution: "exec-1".into(),
            goal: "goal-1".into(),
            bbp_task: "bbp-1".into(),
            attempt: "attempt-1".into(),
            assignment: "project".into(),
            generation,
            state: ExecutionState::Running,
        }
    }
    #[test]
    fn duplicate_and_restart_adopt_one_execution() {
        let mut f = Factory::default();
        f.assign(assignment("project", None, 1, &["orch"], 3, 10))
            .unwrap();
        assert_eq!(
            f.initiate_or_adopt(link(1)).unwrap(),
            f.initiate_or_adopt(link(1)).unwrap()
        );
        let mut other = link(1);
        other.execution = "exec-2".into();
        assert_eq!(
            f.initiate_or_adopt(other),
            Err(FactoryError::CardAlreadyLinked)
        );
    }
    #[test]
    fn delegation_is_bounded_and_parent_retirement_cascades() {
        let mut f = Factory::default();
        f.assign(assignment("project", None, 1, &["orch"], 3, 10))
            .unwrap();
        f.delegate(
            "project",
            1,
            assignment("worker", Some("project"), 1, &["orch"], 2, 4),
        )
        .unwrap();
        assert_eq!(
            f.delegate(
                "project",
                1,
                assignment("bad", Some("project"), 1, &["other"], 2, 4)
            ),
            Err(FactoryError::DelegationOutsideGrant)
        );
        f.retire("project", 1).unwrap();
        assert_eq!(
            f.reconcile("card-1", "worker", 1, ExecutionState::Waiting),
            Err(FactoryError::StaleGeneration)
        );
    }
    #[test]
    fn replacement_and_terminal_reconciliation_reject_stale_commands() {
        let mut f = Factory::default();
        f.assign(assignment("project", None, 1, &["orch"], 3, 10))
            .unwrap();
        f.initiate_or_adopt(link(1)).unwrap();
        f.replace(assignment("project", None, 2, &["orch"], 3, 10))
            .unwrap();
        assert_eq!(
            f.reconcile("card-1", "project", 1, ExecutionState::Waiting),
            Err(FactoryError::StaleGeneration)
        );
        let mut adopted = f.link("card-1").unwrap().clone();
        adopted.generation = 2;
        assert_eq!(f.initiate_or_adopt(adopted).unwrap().generation, 2);
        assert_eq!(
            f.reconcile("card-1", "project", 2, ExecutionState::Waiting)
                .unwrap()
                .state,
            ExecutionState::Waiting
        );
    }
    #[test]
    fn duplicate_ids_and_aggregate_budgets_are_rejected() {
        let mut f = Factory::default();
        f.assign(assignment("project", None, 1, &["orch"], 2, 5))
            .unwrap();
        assert_eq!(
            f.assign(assignment("project", None, 2, &["orch"], 2, 5)),
            Err(FactoryError::Invalid("assignment id already exists"))
        );
        f.delegate(
            "project",
            1,
            assignment("one", Some("project"), 1, &["orch"], 0, 3),
        )
        .unwrap();
        assert_eq!(
            f.delegate(
                "project",
                1,
                assignment("two", Some("project"), 1, &["orch"], 0, 3)
            ),
            Err(FactoryError::DelegationOutsideGrant)
        );
        f.delegate(
            "project",
            1,
            assignment("two", Some("project"), 1, &["orch"], 0, 2),
        )
        .unwrap();
        assert_eq!(
            f.delegate(
                "project",
                1,
                assignment("three", Some("project"), 1, &["orch"], 0, 0)
            ),
            Err(FactoryError::DelegationOutsideGrant)
        );
    }
    #[test]
    fn replacement_cannot_escalate_and_ancestor_generation_fences_children() {
        let mut f = Factory::default();
        f.assign(assignment("project", None, 1, &["orch"], 3, 10))
            .unwrap();
        f.delegate(
            "project",
            1,
            assignment("worker", Some("project"), 1, &["orch"], 1, 4),
        )
        .unwrap();
        assert_eq!(
            f.replace(assignment("worker", Some("project"), 2, &["other"], 1, 4)),
            Err(FactoryError::DelegationOutsideGrant)
        );
        f.replace(assignment("project", None, 2, &["orch"], 3, 10))
            .unwrap();
        assert_eq!(f.authorize("worker", 1), Err(FactoryError::StaleGeneration));
    }
    #[test]
    fn malformed_cycle_fails_closed() {
        let mut f = Factory::default();
        f.assign(assignment("one", None, 1, &["orch"], 2, 2))
            .unwrap();
        f.delegate("one", 1, assignment("two", Some("one"), 1, &["orch"], 1, 1))
            .unwrap();
        f.assignments.get_mut("one").unwrap().parent = Some("two".into());
        f.parent_generations.insert("one".into(), 1);
        assert_eq!(
            f.authorize("two", 1),
            Err(FactoryError::Invalid(
                "assignment ancestry contains a cycle"
            ))
        );
    }
    #[test]
    fn host_states_reconcile_once_and_terminal_states_are_monotonic() {
        let mut f = Factory::default();
        f.assign(assignment("project", None, 1, &["orch"], 3, 10))
            .unwrap();
        f.initiate_or_adopt(link(1)).unwrap();
        assert_eq!(
            f.reconcile("card-1", "project", 1, ExecutionState::Waiting)
                .unwrap()
                .state,
            ExecutionState::Waiting
        );
        assert_eq!(
            f.reconcile("card-1", "project", 1, ExecutionState::Completed)
                .unwrap()
                .state,
            ExecutionState::Completed
        );
        assert_eq!(
            f.reconcile("card-1", "project", 1, ExecutionState::Cancelled),
            Err(FactoryError::Terminal)
        );
    }
}
