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
    /// Filesystem reach and lifetime of the assignment's workers. `None`
    /// leaves them bounded by the hosting configuration alone; a leased
    /// parent delegates only leases it covers, never `None`.
    pub lease: Option<Lease>,
}

/// A permission lease: the paths an assignment's workers may reach and when
/// that authority ends. A host confines workers to it; it never widens what
/// the host's own configuration allows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lease {
    /// Absolute, normalized paths workers may read.
    pub read: Vec<String>,
    /// Absolute, normalized paths workers may read and write.
    pub write: Vec<String>,
    /// Unix seconds at which the lease ends; `None` never ends.
    pub expires: Option<u64>,
}

impl Lease {
    /// Whether every path is absolute and normalized: no empty, `.` or `..`
    /// component and no trailing slash, so containment is a prefix test.
    pub fn valid(&self) -> bool {
        self.read.iter().chain(&self.write).all(|p| normalized(p))
    }

    /// Whether `child` asks for nothing beyond this lease: each write path
    /// inside one of ours, each read path inside one we read or write, and
    /// an end no later than ours.
    pub fn covers(&self, child: &Lease) -> bool {
        let reach = |paths: &[String], p: &String| paths.iter().any(|root| contains(root, p));
        child.write.iter().all(|p| reach(&self.write, p))
            && child
                .read
                .iter()
                .all(|p| reach(&self.read, p) || reach(&self.write, p))
            && match (self.expires, child.expires) {
                (None, _) => true,
                (Some(ours), Some(theirs)) => theirs <= ours,
                (Some(_), None) => false,
            }
    }

    /// Whether the lease still holds at unix second `now`.
    pub fn live(&self, now: u64) -> bool {
        !matches!(self.expires, Some(end) if now >= end)
    }
}

/// Whether normalized absolute `path` is `root` or lies beneath it.
pub fn contains(root: &str, path: &str) -> bool {
    root == "/"
        || path
            .strip_prefix(root)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// Whether `path` is absolute with no empty, `.` or `..` component.
pub fn normalized(path: &str) -> bool {
    path == "/"
        || path.strip_prefix('/').is_some_and(|rest| {
            rest.split('/')
                .all(|c| !c.is_empty() && c != "." && c != "..")
        })
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

/// One stored assignment and the parent generation that authorized it: the
/// unit a storage adapter saves and hands back to [`Factory::restore`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delegation {
    /// The assignment as last changed.
    pub assignment: Assignment,
    /// Generation of the parent that delegated or last re-granted it;
    /// `None` exactly when the assignment has no parent.
    pub parent_generation: Option<u64>,
}

/// In-memory aggregate rebuilt from the durable card/execution records by a storage adapter.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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
        valid_lease(&assignment.grant)?;
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
        valid_lease(&assignment.grant)?;
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
        assignment.generation = generation
            .checked_add(1)
            .ok_or(FactoryError::Invalid("assignment generation exhausted"))?;
        assignment.active = false;
        Ok(())
    }

    /// Re-grant a delegated child under its parent's current generation.
    ///
    /// Replacing a parent leaves every child delegated by the old generation
    /// stale; this is the one way back. The parent must be active at
    /// `parent_generation`, the child's generation must advance, and the new
    /// grant must fit the parent's. Children of the child stay bound to the
    /// child's former generation and need their own re-grant.
    pub fn regrant(
        &mut self,
        parent: &str,
        parent_generation: u64,
        child: Assignment,
    ) -> Result<(), FactoryError> {
        self.authorize(parent, parent_generation)?;
        let previous = self
            .assignments
            .get(&child.id)
            .ok_or(FactoryError::UnknownAssignment)?;
        if previous.parent.as_deref() != Some(parent) {
            return Err(FactoryError::Invalid("re-grant changes parent"));
        }
        if child.generation <= previous.generation {
            return Err(FactoryError::StaleGeneration);
        }
        self.validate_child_grant(parent, &child, Some(&child.id))?;
        self.validate_reserved_capacity(&child)?;
        let child_id = child.id.clone();
        self.assignments.insert(child_id.clone(), child);
        self.parent_generations.insert(child_id, parent_generation);
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
    ///
    /// A link with a higher, currently authorized generation moves a
    /// non-terminal execution to that generation in memory; the adapter
    /// persists it with Millboard's `card_execution_recover`, a
    /// compare-and-set on the previous generation.
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

    /// Every execution link, in card order.
    pub fn links(&self) -> impl Iterator<Item = &ExecutionLink> {
        self.links.values()
    }

    /// The current record of an assignment.
    pub fn assignment(&self, id: &str) -> Option<&Assignment> {
        self.assignments.get(id)
    }

    /// Every assignment with its authorizing parent generation, in id order.
    pub fn delegations(&self) -> Vec<Delegation> {
        self.assignments
            .values()
            .map(|assignment| Delegation {
                assignment: assignment.clone(),
                parent_generation: self.parent_generations.get(&assignment.id).copied(),
            })
            .collect()
    }

    /// Take over a top-level assignment: install it at generation 1, or
    /// advance it to the next generation so that every earlier holder is
    /// stale. The caller persists the result before issuing any command
    /// under the returned generation.
    pub fn take_over(
        &mut self,
        id: &str,
        identity: Identity,
        grant: Grant,
    ) -> Result<u64, FactoryError> {
        let next = |generation: u64| Assignment {
            id: id.to_owned(),
            generation,
            identity: identity.clone(),
            parent: None,
            grant: grant.clone(),
            active: true,
        };
        let Some(previous) = self.assignments.get(id) else {
            self.assign(next(1))?;
            return Ok(1);
        };
        if previous.parent.is_some() {
            return Err(FactoryError::Invalid(
                "a delegated assignment is re-granted by its parent",
            ));
        }
        if !previous.active {
            return Err(FactoryError::Retired);
        }
        let generation = previous
            .generation
            .checked_add(1)
            .ok_or(FactoryError::Invalid("assignment generation exhausted"))?;
        self.replace(next(generation))?;
        Ok(generation)
    }

    /// Rebuild a factory from stored delegations and links, re-checking the
    /// invariants the mutating operations maintain instead of trusting them:
    /// unique ids, known acyclic parents, parent generations no later than
    /// the parent's own, delegation and call budgets that cover every child,
    /// project scopes and leases of currently bound children inside their
    /// parent's, valid lease paths, and
    /// links naming a known assignment at a generation it has reached.
    pub fn restore(
        delegations: Vec<Delegation>,
        links: Vec<ExecutionLink>,
    ) -> Result<Self, FactoryError> {
        let mut factory = Self::default();
        for Delegation {
            assignment,
            parent_generation,
        } in delegations
        {
            if assignment.id.is_empty()
                || assignment.identity.id.is_empty()
                || assignment.generation == 0
            {
                return Err(FactoryError::Invalid(
                    "assignment id, identity id, and generation are required",
                ));
            }
            valid_lease(&assignment.grant)?;
            if assignment.parent.is_some() != parent_generation.is_some() {
                return Err(FactoryError::Invalid(
                    "a parent generation must accompany exactly the delegated assignments",
                ));
            }
            if let Some(generation) = parent_generation {
                factory
                    .parent_generations
                    .insert(assignment.id.clone(), generation);
            }
            if factory
                .assignments
                .insert(assignment.id.clone(), assignment)
                .is_some()
            {
                return Err(FactoryError::Invalid("assignment id already exists"));
            }
        }
        for assignment in factory.assignments.values() {
            factory.restored_ancestry(assignment)?;
            factory.validate_reserved_capacity(assignment)?;
        }
        for link in links {
            factory.restored_link(&link)?;
            if factory.links.insert(link.card.clone(), link).is_some() {
                return Err(FactoryError::CardAlreadyLinked);
            }
        }
        Ok(factory)
    }

    fn restored_ancestry(&self, assignment: &Assignment) -> Result<(), FactoryError> {
        let mut current = assignment;
        let mut seen = BTreeSet::new();
        while let Some(parent) = current.parent.as_deref() {
            if !seen.insert(current.id.as_str()) {
                return Err(FactoryError::Invalid(
                    "assignment ancestry contains a cycle",
                ));
            }
            let parent_assignment = self
                .assignments
                .get(parent)
                .ok_or(FactoryError::Invalid("assignment names an unknown parent"))?;
            let bound = self.parent_generations.get(&current.id).copied();
            if bound > Some(parent_assignment.generation) {
                return Err(FactoryError::Invalid(
                    "assignment is bound to a parent generation not yet reached",
                ));
            }
            if bound == Some(parent_assignment.generation)
                && !within_grant(&parent_assignment.grant, &current.grant)
            {
                return Err(FactoryError::DelegationOutsideGrant);
            }
            current = parent_assignment;
        }
        Ok(())
    }

    fn restored_link(&self, link: &ExecutionLink) -> Result<(), FactoryError> {
        if [
            &link.card,
            &link.execution,
            &link.goal,
            &link.bbp_task,
            &link.attempt,
        ]
        .iter()
        .any(|id| id.is_empty())
        {
            return Err(FactoryError::Invalid("execution identifiers are required"));
        }
        let owner = self
            .assignments
            .get(&link.assignment)
            .ok_or(FactoryError::UnknownAssignment)?;
        if link.generation == 0 || link.generation > owner.generation {
            return Err(FactoryError::Invalid(
                "link generation is outside its assignment's history",
            ));
        }
        Ok(())
    }

    /// Whether a command under `generation` of assignment `id` is current:
    /// the generation is the assignment's own, it and every ancestor are
    /// active, and each link of the ancestry is bound to its parent's
    /// current generation.
    pub fn authorize(&self, id: &str, generation: u64) -> Result<(), FactoryError> {
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

    /// Add one assignment's reservation (itself plus its descendants, and its
    /// calls) to running totals. Every step is checked: a budget at `u32::MAX`
    /// is refused, never wrapped.
    fn reserve(
        (descendants, calls): (u32, u32),
        grant: &Grant,
    ) -> Result<(u32, u32), FactoryError> {
        let descendants = grant
            .delegation_budget
            .checked_add(1)
            .and_then(|own| descendants.checked_add(own))
            .ok_or(FactoryError::DelegationOutsideGrant)?;
        let calls = calls
            .checked_add(grant.call_budget)
            .ok_or(FactoryError::DelegationOutsideGrant)?;
        Ok((descendants, calls))
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
        let mut totals = (0_u32, 0_u32);
        for existing in self.assignments.values().filter(|assignment| {
            assignment.parent.as_deref() == Some(parent)
                && replacing != Some(assignment.id.as_str())
        }) {
            totals = Self::reserve(totals, &existing.grant)?;
        }
        valid_lease(&child.grant)?;
        let (descendants, calls) = Self::reserve(totals, &child.grant)?;
        if descendants > parent_assignment.grant.delegation_budget
            || calls > parent_assignment.grant.call_budget
            || !within_grant(&parent_assignment.grant, &child.grant)
        {
            return Err(FactoryError::DelegationOutsideGrant);
        }
        Ok(())
    }

    fn validate_reserved_capacity(&self, assignment: &Assignment) -> Result<(), FactoryError> {
        let mut totals = (0_u32, 0_u32);
        for child in self
            .assignments
            .values()
            .filter(|child| child.parent.as_deref() == Some(assignment.id.as_str()))
        {
            totals = Self::reserve(totals, &child.grant)?;
        }
        let (descendants, calls) = totals;
        if descendants > assignment.grant.delegation_budget || calls > assignment.grant.call_budget
        {
            return Err(FactoryError::DelegationOutsideGrant);
        }
        Ok(())
    }
}

/// Whether `child`'s project scope and lease fit inside `parent`'s. An
/// unleased parent bounds no lease; a leased one admits only leases it covers.
fn within_grant(parent: &Grant, child: &Grant) -> bool {
    let lease = match (&parent.lease, &child.lease) {
        (None, _) => true,
        (Some(ours), Some(theirs)) => ours.covers(theirs),
        (Some(_), None) => false,
    };
    lease
        && child
            .projects
            .iter()
            .all(|project| parent.projects.contains(project))
}

fn valid_lease(grant: &Grant) -> Result<(), FactoryError> {
    if grant.lease.as_ref().is_some_and(|lease| !lease.valid()) {
        return Err(FactoryError::Invalid(
            "lease paths must be absolute and normalized",
        ));
    }
    Ok(())
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
                lease: None,
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
    #[test]
    fn budgets_at_the_integer_limit_are_refused_not_wrapped() {
        let mut f = Factory::default();
        f.assign(assignment(
            "project",
            None,
            1,
            &["orch"],
            u32::MAX,
            u32::MAX,
        ))
        .unwrap();
        // 1 + u32::MAX descendants cannot be represented: refused, no panic.
        assert_eq!(
            f.delegate(
                "project",
                1,
                assignment("greedy", Some("project"), 1, &["orch"], u32::MAX, 1)
            ),
            Err(FactoryError::DelegationOutsideGrant)
        );
        f.delegate(
            "project",
            1,
            assignment("max", Some("project"), 1, &["orch"], u32::MAX - 1, 1),
        )
        .unwrap();
        // The sibling total would pass u32::MAX.
        assert_eq!(
            f.delegate(
                "project",
                1,
                assignment("one-more", Some("project"), 1, &["orch"], 0, 1)
            ),
            Err(FactoryError::DelegationOutsideGrant)
        );
        // Calls overflow on their own as well.
        let mut g = Factory::default();
        g.assign(assignment("project", None, 1, &["orch"], 4, u32::MAX))
            .unwrap();
        g.delegate(
            "project",
            1,
            assignment("a", Some("project"), 1, &["orch"], 0, u32::MAX),
        )
        .unwrap();
        assert_eq!(
            g.delegate(
                "project",
                1,
                assignment("b", Some("project"), 1, &["orch"], 0, 1)
            ),
            Err(FactoryError::DelegationOutsideGrant)
        );
    }
    #[test]
    fn retiring_the_last_generation_is_refused_without_mutating() {
        let mut f = Factory::default();
        f.assign(assignment("project", None, u64::MAX, &["orch"], 1, 1))
            .unwrap();
        assert_eq!(
            f.retire("project", u64::MAX),
            Err(FactoryError::Invalid("assignment generation exhausted"))
        );
        assert_eq!(f.authorize("project", u64::MAX), Ok(()));
    }
    #[test]
    fn a_child_recovers_only_through_a_regrant_by_the_current_parent() {
        let mut f = Factory::default();
        f.assign(assignment("root", None, 1, &["orch"], 3, 10))
            .unwrap();
        f.delegate(
            "root",
            1,
            assignment("worker", Some("root"), 1, &["orch"], 0, 4),
        )
        .unwrap();
        f.replace(assignment("root", None, 2, &["orch"], 3, 10))
            .unwrap();
        // A plain replacement of the child keeps its old parent binding: stale.
        f.replace(assignment("worker", Some("root"), 2, &["orch"], 0, 4))
            .unwrap();
        assert_eq!(f.authorize("worker", 2), Err(FactoryError::StaleGeneration));
        // The old parent generation cannot re-grant.
        assert_eq!(
            f.regrant(
                "root",
                1,
                assignment("worker", Some("root"), 3, &["orch"], 0, 4)
            ),
            Err(FactoryError::StaleGeneration)
        );
        // Nor can a re-grant reuse the child's generation or broaden its grant.
        assert_eq!(
            f.regrant(
                "root",
                2,
                assignment("worker", Some("root"), 2, &["orch"], 0, 4)
            ),
            Err(FactoryError::StaleGeneration)
        );
        assert_eq!(
            f.regrant(
                "root",
                2,
                assignment("worker", Some("root"), 3, &["other"], 0, 4)
            ),
            Err(FactoryError::DelegationOutsideGrant)
        );
        f.regrant(
            "root",
            2,
            assignment("worker", Some("root"), 3, &["orch"], 0, 4),
        )
        .unwrap();
        assert_eq!(f.authorize("worker", 3), Ok(()));
        assert_eq!(f.authorize("worker", 2), Err(FactoryError::StaleGeneration));
        // A retired parent re-grants nothing.
        f.retire("root", 2).unwrap();
        assert_eq!(
            f.regrant(
                "root",
                3,
                assignment("worker", Some("root"), 4, &["orch"], 0, 4)
            ),
            Err(FactoryError::Retired)
        );
        assert_eq!(f.authorize("worker", 3), Err(FactoryError::StaleGeneration));
    }
    fn lease(read: &[&str], write: &[&str], expires: Option<u64>) -> Option<Lease> {
        let paths = |ps: &[&str]| ps.iter().map(|p| (*p).to_owned()).collect();
        Some(Lease {
            read: paths(read),
            write: paths(write),
            expires,
        })
    }
    fn leased(id: &str, parent: Option<&str>, lease: Option<Lease>) -> Assignment {
        let mut a = assignment(id, parent, 1, &["orch"], u32::from(parent.is_none()), 1);
        a.grant.lease = lease;
        a
    }
    #[test]
    fn lease_paths_are_normalized_and_contained_by_whole_components() {
        assert!(contains("/srv", "/srv"));
        assert!(contains("/srv", "/srv/a/b"));
        assert!(contains("/", "/srv"));
        assert!(!contains("/srv", "/srvx"));
        assert!(!contains("/srv/a", "/srv"));
        for bad in [
            "",
            "srv",
            "/srv/",
            "/srv//a",
            "/srv/./a",
            "/srv/../etc",
            "/..",
        ] {
            assert!(!normalized(bad), "{bad:?}");
        }
        assert!(normalized("/") && normalized("/srv/a.b/..c"));
        assert!(lease(&["/a"], &["/b"], None).is_some_and(|l| l.valid()));
        assert!(!lease(&["/a/../etc"], &[], None).is_some_and(|l| l.valid()));
    }
    #[test]
    fn a_lease_covers_only_narrower_reach_and_an_earlier_end() {
        let parent = lease(&["/r"], &["/w"], Some(100)).unwrap();
        let covers = |child: Option<Lease>| parent.covers(&child.unwrap());
        assert!(covers(lease(&["/r/x", "/w/y"], &["/w/z"], Some(100))));
        assert!(
            !covers(lease(&[], &["/r/x"], Some(50))),
            "write under a read root"
        );
        assert!(!covers(lease(&["/elsewhere"], &[], Some(50))));
        assert!(!covers(lease(&[], &[], Some(101))));
        assert!(
            !covers(lease(&[], &[], None)),
            "an endless lease outlives ours"
        );
        assert!(lease(&[], &["/"], None).unwrap().covers(&parent));
        assert!(parent.live(99) && !parent.live(100));
        assert!(lease(&[], &[], None).unwrap().live(u64::MAX));
    }
    #[test]
    fn delegation_never_widens_a_lease() {
        let mut f = Factory::default();
        f.assign(leased(
            "root",
            None,
            lease(&["/src"], &["/src/a"], Some(100)),
        ))
        .unwrap();
        for wider in [
            None,
            lease(&["/etc"], &[], Some(100)),
            lease(&[], &["/src"], Some(100)),
            lease(&[], &["/src/a"], Some(101)),
        ] {
            assert_eq!(
                f.delegate("root", 1, leased("child", Some("root"), wider)),
                Err(FactoryError::DelegationOutsideGrant)
            );
        }
        assert_eq!(
            f.delegate(
                "root",
                1,
                leased("child", Some("root"), lease(&["/src/../etc"], &[], Some(1)))
            ),
            Err(FactoryError::Invalid(
                "lease paths must be absolute and normalized"
            ))
        );
        f.delegate(
            "root",
            1,
            leased(
                "child",
                Some("root"),
                lease(&["/src/b"], &["/src/a/x"], Some(90)),
            ),
        )
        .unwrap();
        // Re-granting cannot widen it either.
        let mut root = leased("root", None, lease(&["/src"], &["/src/a"], Some(100)));
        root.generation = 2;
        f.replace(root).unwrap();
        let mut wider = leased("child", Some("root"), lease(&["/"], &[], Some(90)));
        wider.generation = 2;
        assert_eq!(
            f.regrant("root", 2, wider),
            Err(FactoryError::DelegationOutsideGrant)
        );
        // An unleased root delegates any valid lease.
        let mut g = Factory::default();
        g.assign(leased("root", None, None)).unwrap();
        g.delegate(
            "root",
            1,
            leased("child", Some("root"), lease(&["/"], &["/"], None)),
        )
        .unwrap();
        assert_eq!(
            g.assign(leased("bad", None, lease(&["relative"], &[], None))),
            Err(FactoryError::Invalid(
                "lease paths must be absolute and normalized"
            ))
        );
    }
    #[test]
    fn restore_refuses_a_bound_child_outside_its_parents_lease() {
        let root = Delegation {
            assignment: leased("root", None, lease(&["/src"], &[], Some(100))),
            parent_generation: None,
        };
        let child = |bound| Delegation {
            assignment: leased("child", Some("root"), lease(&["/etc"], &[], Some(100))),
            parent_generation: Some(bound),
        };
        let mut stale = root.clone();
        stale.assignment.generation = 2;
        assert_eq!(
            Factory::restore(vec![root, child(1)], vec![]),
            Err(FactoryError::DelegationOutsideGrant)
        );
        assert!(Factory::restore(vec![stale, child(1)], vec![]).is_ok());
    }
    fn restored(f: &Factory) -> Result<Factory, FactoryError> {
        Factory::restore(f.delegations(), f.links().cloned().collect())
    }
    #[test]
    fn a_restored_factory_keeps_every_fence_and_link() {
        let mut f = Factory::default();
        f.assign(assignment("root", None, 1, &["orch"], 3, 10))
            .unwrap();
        f.delegate(
            "root",
            1,
            assignment("stale", Some("root"), 1, &["orch"], 0, 2),
        )
        .unwrap();
        f.delegate(
            "root",
            1,
            assignment("current", Some("root"), 1, &["orch"], 0, 2),
        )
        .unwrap();
        f.replace(assignment("root", None, 2, &["orch"], 3, 10))
            .unwrap();
        f.regrant(
            "root",
            2,
            assignment("current", Some("root"), 2, &["orch"], 0, 2),
        )
        .unwrap();
        let mut owned = link(2);
        owned.assignment = "root".into();
        f.initiate_or_adopt(owned).unwrap();
        f.reconcile("card-1", "root", 2, ExecutionState::Completed)
            .unwrap();
        let g = restored(&f).unwrap();
        assert_eq!(g, f);
        assert_eq!(g.authorize("current", 2), Ok(()));
        assert_eq!(g.authorize("stale", 1), Err(FactoryError::StaleGeneration));
        assert_eq!(
            g.link("card-1").map(|l| l.state),
            Some(ExecutionState::Completed)
        );
    }
    #[test]
    fn restore_refuses_records_the_operations_could_not_have_produced() {
        let root = || Delegation {
            assignment: assignment("root", None, 2, &["orch"], 1, 4),
            parent_generation: None,
        };
        let child = |generation, projects: &[&str], calls| Delegation {
            assignment: assignment("child", Some("root"), 1, projects, 0, calls),
            parent_generation: generation,
        };
        let refused = |delegations: Vec<Delegation>, links: Vec<ExecutionLink>| {
            Factory::restore(delegations, links).unwrap_err()
        };
        assert_eq!(
            refused(vec![root(), child(None, &["orch"], 1)], vec![]),
            FactoryError::Invalid(
                "a parent generation must accompany exactly the delegated assignments"
            )
        );
        assert_eq!(
            refused(vec![root(), child(Some(3), &["orch"], 1)], vec![]),
            FactoryError::Invalid("assignment is bound to a parent generation not yet reached")
        );
        assert_eq!(
            refused(vec![root(), child(Some(2), &["other"], 1)], vec![]),
            FactoryError::DelegationOutsideGrant
        );
        // A stale child keeps the scope it was granted; only budgets bind it.
        assert!(Factory::restore(vec![root(), child(Some(1), &["other"], 1)], vec![]).is_ok());
        assert_eq!(
            refused(vec![root(), child(Some(1), &["orch"], 5)], vec![]),
            FactoryError::DelegationOutsideGrant
        );
        assert_eq!(
            refused(vec![child(Some(1), &["orch"], 1)], vec![]),
            FactoryError::Invalid("assignment names an unknown parent")
        );
        assert_eq!(
            refused(vec![root(), root()], vec![]),
            FactoryError::Invalid("assignment id already exists")
        );
        let mut cyclic = root();
        cyclic.assignment.parent = Some("child".into());
        cyclic.parent_generation = Some(1);
        assert_eq!(
            refused(vec![cyclic, child(Some(1), &["orch"], 1)], vec![]),
            FactoryError::Invalid("assignment ancestry contains a cycle")
        );
        let mut ahead = link(3);
        ahead.assignment = "root".into();
        assert_eq!(
            refused(vec![root()], vec![ahead]),
            FactoryError::Invalid("link generation is outside its assignment's history")
        );
        assert_eq!(
            refused(vec![root()], vec![link(1)]),
            FactoryError::UnknownAssignment
        );
    }
    #[test]
    fn taking_over_fences_every_earlier_holder() {
        let mut f = Factory::default();
        let identity = || Identity {
            id: "orch".into(),
            target: "rusty_orch run".into(),
        };
        let grant = || Grant {
            projects: vec![],
            delegation_budget: 0,
            call_budget: 3,
            lease: None,
        };
        assert_eq!(f.take_over("goal", identity(), grant()), Ok(1));
        assert_eq!(f.take_over("goal", identity(), grant()), Ok(2));
        assert_eq!(f.authorize("goal", 1), Err(FactoryError::StaleGeneration));
        assert_eq!(f.authorize("goal", 2), Ok(()));
        f.retire("goal", 2).unwrap();
        assert_eq!(
            f.take_over("goal", identity(), grant()),
            Err(FactoryError::Retired)
        );
        f.assign(assignment("root", None, 1, &["orch"], 1, 1))
            .unwrap();
        f.delegate(
            "root",
            1,
            assignment("child", Some("root"), 1, &["orch"], 0, 1),
        )
        .unwrap();
        assert_eq!(
            f.take_over("child", identity(), grant()),
            Err(FactoryError::Invalid(
                "a delegated assignment is re-granted by its parent"
            ))
        );
    }
}
