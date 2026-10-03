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

    /// Rebuild a ledger from counts an earlier process saved, so a resumed
    /// goal keeps honouring what it already spent. Zero task counts are
    /// dropped; `goal_calls` is taken as given.
    pub fn from_counts(
        goal_calls: u32,
        task_calls: impl IntoIterator<Item = (TaskId, u32)>,
    ) -> Self {
        Self {
            goal_calls,
            task_calls: task_calls.into_iter().filter(|(_, n)| *n > 0).collect(),
        }
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

    /// Undo exactly the most recent charge for `task`.
    pub(crate) fn rollback(&mut self, task: TaskId) {
        self.goal_calls -= 1;
        if let Some(calls) = self.task_calls.get_mut(&task) {
            *calls -= 1;
            if *calls == 0 {
                self.task_calls.remove(&task);
            }
        }
    }
}

fn ceiling(ceiling: Ceiling, task: TaskId, limit: NonZeroU32) -> DispatchError {
    DispatchError::CeilingReached {
        ceiling,
        task,
        limit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_counts_round_trips_and_drops_zero_rows() {
        let t1 = TaskId::from_raw(1);
        let t2 = TaskId::from_raw(2);
        let ledger = Ledger::from_counts(3, [(t1, 2), (t2, 0)]);
        assert_eq!(ledger.calls(), 3);
        assert_eq!(ledger.task_calls(t1), 2);
        assert_eq!(ledger.task_calls(t2), 0);
        assert_eq!(ledger, Ledger::from_counts(3, [(t1, 2)]));
    }
}
