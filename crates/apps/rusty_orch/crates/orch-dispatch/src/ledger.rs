//! Call accounting, owned by the caller so it outlives any one dispatcher.

use std::collections::BTreeMap;
use std::num::NonZeroU32;

use orch_core::goal::Goal;
use orch_core::task::Task;
use orch_core::TaskId;

use crate::{Ceiling, DispatchError};

/// Calls spent against a goal's budget and each card's own ceiling.
///
/// Passed to [`Dispatcher::run`](crate::Dispatcher::run) like the plan and
/// board, so a fresh dispatcher resuming a goal keeps honouring what earlier
/// runs spent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ledger {
    goal_calls: u32,
    task_calls: BTreeMap<TaskId, u32>,
}

impl Ledger {
    /// Nothing spent.
    pub fn new() -> Self {
        Self::default()
    }

    /// Calls made against the goal budget.
    pub fn calls(&self) -> u32 {
        self.goal_calls
    }

    /// Calls made for one card.
    pub fn task_calls(&self, id: TaskId) -> u32 {
        self.task_calls.get(&id).copied().unwrap_or(0)
    }

    /// Refuse if one more call for `task` would exceed either ceiling.
    pub(crate) fn check(&self, goal: &Goal, task: &Task) -> Result<(), DispatchError> {
        let id = task.id();
        let goal_limit = goal.budget().max_calls();
        if self.goal_calls >= goal_limit.get() {
            return Err(ceiling(Ceiling::Goal, id, goal_limit));
        }
        let task_limit = task.spec().max_calls;
        if self.task_calls(id) >= task_limit.get() {
            return Err(ceiling(Ceiling::Task, id, task_limit));
        }
        Ok(())
    }

    /// Record one call for `task`.
    pub(crate) fn count(&mut self, task: TaskId) {
        self.goal_calls += 1;
        *self.task_calls.entry(task).or_insert(0) += 1;
    }
}

fn ceiling(ceiling: Ceiling, task: TaskId, limit: NonZeroU32) -> DispatchError {
    DispatchError::CeilingReached {
        ceiling,
        task,
        limit,
    }
}
