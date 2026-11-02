//! The task system: a heap-allocated, reference-counted future plus a
//! small lock-free state machine that decides, on every wake, whether
//! the task needs to be (re-)enqueued on the scheduler or is already
//! spoken for.
//!
//! Naively you could push `Arc<Task>` onto a channel every time a waker
//! fires and have workers pop-lock-poll-unlock. That's what most "build
//! your own executor" blog posts do, and it has a real bug under actual
//! multi-threaded concurrency: if a wake happens *while* a task is being
//! polled on another thread, the future is temporarily absent from its
//! slot (taken out for polling), so the duplicate wakeup finds nothing
//! to do and is silently dropped -- a lost wakeup, which means a task
//! can go to sleep forever even though something tried to wake it.
//!
//! The fix is the same one tokio and `async-task` use: an explicit
//! `RUNNING` / `NOTIFIED` pair of state bits. A wake that arrives while
//! the task is running doesn't try to touch the future at all -- it
//! just leaves a note (`NOTIFIED`) for the poller to see when it's done,
//! and the poller reschedules itself if that note is present.

mod block_in_place;
mod builder;
pub mod coop;
mod id;
mod join;
mod join_set;
mod local;
mod local_key;
mod owned;
mod state;
pub(crate) mod trace;
mod yield_now;

/// tokio's `task::spawn_blocking` path for [`crate::spawn_blocking`] (#897).
pub use crate::spawn_blocking;
pub use block_in_place::block_in_place;
pub use builder::Builder;
pub use id::{try_id, try_name, TaskId};
pub use join::{AbortHandle, JoinError, JoinHandle};
pub use join_set::JoinSet;
pub use local::{spawn_local, LocalEnterGuard, LocalSet};
pub use local_key::{AccessError, LocalKey, TaskLocalFuture};
pub use yield_now::{yield_now, YieldNow};

pub(crate) use owned::OwnedTasks;

use crate::runtime::Shared;
use join::{AbnormalHook, JoinInner, Outcome};
use state::{Begin, State, StateSnapshot};
use std::future::Future;
use std::panic::{self, AssertUnwindSafe};
use std::pin::Pin;
use std::sync::{Arc, Mutex, Weak};

type BoxFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

/// A scheduled unit of work. Type-erased to `Future<Output = ()>` --
/// the actual output value (if any) is threaded out separately through
/// a [`JoinInner`], so `Task` itself never needs to be generic.
pub(crate) struct Task {
    id: TaskId,
    name: Option<Arc<str>>,
    state: State,
    future: Mutex<Option<BoxFuture>>,
    scheduler: Weak<Shared>,
    /// Fires exactly once if the task ends abnormally (aborted before
    /// completion, or panicked) so the `JoinHandle` doesn't hang
    /// forever waiting for a completion that will never come through
    /// the future's own body.
    abnormal_hook: Mutex<Option<AbnormalHook>>,
}

impl Task {
    fn schedule(self: &Arc<Self>) {
        if let Some(shared) = self.scheduler.upgrade() {
            shared.schedule(self.clone());
        }
    }

    /// Called by a worker immediately after popping this task off a run
    /// queue. Runs at most one poll, handling completion, panics, and
    /// abort requests, and re-schedules itself if it was woken again
    /// while it was running.
    pub(crate) fn run(self: Arc<Self>) {
        match self.state.begin_poll() {
            Begin::Poll => {}
            Begin::Aborted => return self.cancel(),
            // Runtime shutdown already finished this task.
            Begin::Stale => return,
        }

        if self.state.is_aborted() {
            return self.cancel();
        }

        let mut slot = self.future.lock().unwrap();
        let Some(mut future) = slot.take() else {
            // Nothing left to poll (defensive -- shouldn't happen given
            // the state machine's invariants, but a stray extra wakeup
            // must never panic a worker thread, and the joiner must
            // never be left hanging because of it).
            drop(slot);
            return self.cancel();
        };
        drop(slot);

        let waker = std::task::Waker::from(self.clone());
        let mut cx = std::task::Context::from_waker(&waker);

        let poll_result = {
            // Scoped tightly around just this poll call -- see
            // `id::EnterGuard`'s docs for why that's the only span
            // "the task currently running on this thread" is
            // well-defined for.
            let _id_guard = id::enter(id::CurrentTask {
                id: self.id,
                name: self.name.clone(),
            });
            crate::coop::budget(|| {
                panic::catch_unwind(AssertUnwindSafe(|| future.as_mut().poll(&mut cx)))
            })
        };

        match poll_result {
            Ok(std::task::Poll::Ready(())) => {
                self.state.end_poll(true);
                // The future's own body already notified its JoinInner
                // on the normal-completion path; drop the hook so it
                // doesn't hold that Arc alive for no reason.
                self.abnormal_hook.lock().unwrap().take();
                self.mark_finished();
            }
            Ok(std::task::Poll::Pending) => {
                *self.future.lock().unwrap() = Some(future);
                if self.state.end_poll(false) {
                    if self.state.is_aborted() {
                        // Aborted mid-poll, perhaps by runtime shutdown,
                        // whose run queues nothing will drain: cancel
                        // here rather than through the scheduler.
                        return self.run();
                    }
                    // Woken again while we were polling: go around once
                    // more instead of waiting for an external wake.
                    self.schedule();
                }
            }
            Err(payload) => {
                *self.future.lock().unwrap() = None;
                self.state.end_poll(true);
                self.fire_abnormal_hook(Outcome::Panicked(payload));
                self.mark_finished();
            }
        }
    }

    /// Finishes a task this thread owns RUNNING for without polling it
    /// again: drops the future, resolves the `JoinHandle` as cancelled
    /// and releases the task.
    fn cancel(&self) {
        let future = self.future.lock().unwrap().take();
        // Dropped after the lock: a future's drop can run arbitrary code.
        drop(future);
        self.state.end_poll(true);
        self.fire_abnormal_hook(Outcome::Aborted);
        self.mark_finished();
    }

    /// Releases the task from the runtime's [`OwnedTasks`] and counts it
    /// out of `num_alive_tasks` (see [`Shared::wait_for_tasks_drain`]).
    /// Called from every terminal path in `run`, never on `Pending`, since
    /// a `Pending` task is still alive and expected to be polled again.
    /// Only the first call for a task counts.
    fn mark_finished(&self) {
        let Some(shared) = self.scheduler.upgrade() else {
            return;
        };
        let released = shared.owned.remove(self.id);
        if released.is_some() {
            shared.task_finished();
        }
    }

    /// Cancels this task as part of runtime shutdown, wherever it is: an
    /// idle or queued task is finished here; a task mid-poll on another
    /// thread is finished by that thread as soon as its poll returns.
    pub(crate) fn shutdown(self: Arc<Self>) {
        if self.state.begin_shutdown() {
            self.cancel();
        }
    }

    fn fire_abnormal_hook(&self, outcome: Outcome) {
        if let Some(hook) = self.abnormal_hook.lock().unwrap().take() {
            hook(outcome);
        }
    }

    pub(crate) fn abort(self: &Arc<Self>) {
        if self.state.request_abort() {
            self.schedule();
        }
    }
}

impl std::task::Wake for Task {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        if self.state.wake() == StateSnapshot::ShouldSchedule {
            self.schedule();
        }
    }
}

/// Spawn `future` onto `shared`'s scheduler, returning a handle that can
/// be awaited for its output (or used to abort it).
#[track_caller]
pub(crate) fn spawn<F>(shared: &Arc<Shared>, future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    spawn_named(shared, future, None)
}

/// Like [`spawn`], but carrying an optional name -- the
/// [`builder::Builder`] spawn path this backs.
#[track_caller]
pub(crate) fn spawn_named<F>(
    shared: &Arc<Shared>,
    future: F,
    name: Option<Arc<str>>,
) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let id = TaskId::next();
    let join_inner = Arc::new(JoinInner::new());
    let handle = JoinHandle::new(join_inner.clone(), id);

    let hook_inner = join_inner.clone();
    let hook: AbnormalHook = Box::new(move |outcome| hook_inner.finish_abnormal(outcome));

    // `()` when the `tracing` feature is off -- see `trace`'s module
    // docs for why call sites don't need to `#[cfg]` around this.
    #[allow(clippy::let_unit_value)]
    let span = trace::spawn_span("task", name.as_deref(), id.as_u64());
    // Only `future` runs inside the span: completing the join after it
    // exits means a subscriber has seen the last poll end by the time the
    // `JoinHandle` resolves. It is instrumented out here, not inside the
    // block, so a task aborted before its first poll still drops `future`
    // inside its span.
    let instrumented = trace::instrument(future, span);
    let wrapped: BoxFuture = Box::pin(async move {
        let output = instrumented.await;
        join_inner.complete(output);
    });

    let task = Arc::new(Task {
        id,
        name,
        state: State::new(),
        future: Mutex::new(Some(wrapped)),
        scheduler: Arc::downgrade(shared),
        abnormal_hook: Mutex::new(Some(hook)),
    });

    shared.task_spawned();
    if shared.owned.bind(&task) {
        shared.schedule(task.clone());
    } else {
        // The runtime has shut down: nothing would ever run this task.
        // Its never-polled future is dropped and the handle resolves as
        // cancelled, as on tokio.
        shared.task_finished();
        task.clone().shutdown();
    }
    handle.with_task(task)
}
