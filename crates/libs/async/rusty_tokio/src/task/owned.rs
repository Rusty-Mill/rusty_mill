//! [`OwnedTasks`]: the runtime's list of every task it has spawned and
//! not yet finished (card #405). Without it a task was owned only by the
//! wakers and run-queue entries pointing at it, so a task pending with no
//! waker stored anywhere was silently dropped, its `JoinHandle` hung and
//! `num_alive_tasks` never went down. With it the runtime keeps every
//! task alive until the task finishes or the runtime shuts down, and
//! shutdown cancels whatever is left, as tokio does. A [`LocalSet`]
//! keeps its `spawn_local` tasks in one too (card #591), and cancels
//! what is left when it drops.
//!
//! [`LocalSet`]: super::LocalSet

use super::TaskId;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

pub(crate) struct OwnedTasks<T> {
    inner: Mutex<Inner<T>>,
}

struct Inner<T> {
    /// Set once by [`OwnedTasks::close`]; no task is added after that.
    closed: bool,
    tasks: HashMap<TaskId, Arc<T>>,
}

impl<T> OwnedTasks<T> {
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
    pub(super) fn bind(&self, id: TaskId, task: &Arc<T>) -> bool {
        let mut inner = self.lock();
        if inner.closed {
            return false;
        }
        inner.tasks.insert(id, task.clone());
        true
    }

    /// Releases a finished task. `None` if it was never bound or was
    /// already released. The caller drops the returned `Arc` after this
    /// lock is released, since dropping a task can run arbitrary code.
    pub(super) fn remove(&self, id: TaskId) -> Option<Arc<T>> {
        self.lock().tasks.remove(&id)
    }

    /// Refuses all further tasks and returns every task still owned, to
    /// be cancelled by the caller. Each one stays owned until its own
    /// cancellation releases it, so `num_alive_tasks` counts down as
    /// tasks actually finish.
    pub(crate) fn close(&self) -> Vec<Arc<T>> {
        let mut inner = self.lock();
        inner.closed = true;
        inner.tasks.values().cloned().collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner<T>> {
        // Nothing panics while holding this lock, but a poisoned registry
        // must not stop shutdown from cancelling what it holds.
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
