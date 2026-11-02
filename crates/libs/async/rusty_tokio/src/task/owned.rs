//! [`OwnedTasks`]: the runtime's list of every task it has spawned and
//! not yet finished (card #405). Without it a task was owned only by the
//! wakers and run-queue entries pointing at it, so a task pending with no
//! waker stored anywhere was silently dropped, its `JoinHandle` hung and
//! `num_alive_tasks` never went down. With it the runtime keeps every
//! task alive until the task finishes or the runtime shuts down, and
//! shutdown cancels whatever is left, as tokio does.

use super::{Task, TaskId};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

pub(crate) struct OwnedTasks {
    inner: Mutex<Inner>,
}

struct Inner {
    /// Set once by [`OwnedTasks::close`]; no task is added after that.
    closed: bool,
    tasks: HashMap<TaskId, Arc<Task>>,
}

impl OwnedTasks {
    pub(crate) fn new() -> Self {
        OwnedTasks {
            inner: Mutex::new(Inner {
                closed: false,
                tasks: HashMap::new(),
            }),
        }
    }

    /// Takes ownership of a newly spawned task. Returns `false`, and
    /// keeps nothing, once the runtime has shut down: the caller then
    /// cancels the task instead of scheduling it.
    pub(super) fn bind(&self, task: &Arc<Task>) -> bool {
        let mut inner = self.lock();
        if inner.closed {
            return false;
        }
        inner.tasks.insert(task.id, task.clone());
        true
    }

    /// Releases a finished task. `None` if it was never bound or was
    /// already released. The caller drops the returned `Arc` after this
    /// lock is released, since dropping a task can run arbitrary code.
    pub(super) fn remove(&self, id: TaskId) -> Option<Arc<Task>> {
        self.lock().tasks.remove(&id)
    }

    /// Refuses all further tasks and returns every task still owned, to
    /// be cancelled by the caller. Each one stays owned until its own
    /// cancellation releases it, so `num_alive_tasks` counts down as
    /// tasks actually finish.
    pub(crate) fn close(&self) -> Vec<Arc<Task>> {
        let mut inner = self.lock();
        inner.closed = true;
        inner.tasks.values().cloned().collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        // Nothing panics while holding this lock, but a poisoned registry
        // must not stop shutdown from cancelling what it holds.
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
