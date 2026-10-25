//! The atomic bit-state backing [`super::Task`]. See the module docs on
//! `task` for why this exists instead of a plain channel-of-`Arc`
//! design.

use std::sync::atomic::{AtomicU8, Ordering};

const QUEUED: u8 = 1 << 0;
const RUNNING: u8 = 1 << 1;
const NOTIFIED: u8 = 1 << 2;
const COMPLETE: u8 = 1 << 3;
const ABORTED: u8 = 1 << 4;

pub(super) struct State(AtomicU8);

#[derive(Debug, PartialEq, Eq)]
pub(super) enum StateSnapshot {
    ShouldSchedule,
    NoOp,
}

impl State {
    pub(super) fn new() -> Self {
        // A freshly spawned task is handed straight to the scheduler by
        // its creator, so it starts life already QUEUED.
        State(AtomicU8::new(QUEUED))
    }

    pub(super) fn is_aborted(&self) -> bool {
        self.0.load(Ordering::Acquire) & ABORTED != 0
    }

    /// Transition QUEUED -> RUNNING. Returns `false` if the task was
    /// aborted before this poll. The caller still owns RUNNING and must
    /// finish cancellation with `end_poll(true)`, even on `false`.
    pub(super) fn begin_poll(&self) -> bool {
        self.begin_poll_from(self.0.load(Ordering::Acquire))
    }

    // Keep the initial observation separate so tests can deterministically
    // insert an abort between the load and the first CAS.
    fn begin_poll_from(&self, mut state: u8) -> bool {
        loop {
            debug_assert!(state & QUEUED != 0 && state & RUNNING == 0);
            let new = (state & !(QUEUED | NOTIFIED)) | RUNNING;
            // Only the dequeuer can claim RUNNING, but request_abort can
            // add ABORTED while the task is still QUEUED. Retry with the
            // actual bits so cancellation is preserved, not overwritten.
            match self
                .0
                .compare_exchange(state, new, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return state & ABORTED == 0,
                Err(actual) => state = actual,
            }
        }
    }

    /// Called after a poll returns. `ready` is true for completion
    /// (including a panic, which is treated as completion). Returns
    /// `true` if the caller must re-schedule the task itself because a
    /// wake arrived while it was running.
    pub(super) fn end_poll(&self, ready: bool) -> bool {
        if ready {
            self.0.fetch_or(COMPLETE, Ordering::AcqRel);
            self.0.fetch_and(!RUNNING, Ordering::AcqRel);
            return false;
        }

        let mut state = self.0.load(Ordering::Acquire);
        loop {
            if state & NOTIFIED != 0 {
                let new = (state & !(RUNNING | NOTIFIED)) | QUEUED;
                match self
                    .0
                    .compare_exchange(state, new, Ordering::AcqRel, Ordering::Acquire)
                {
                    Ok(_) => return true,
                    Err(actual) => {
                        state = actual;
                        continue;
                    }
                }
            } else {
                let new = state & !RUNNING;
                match self
                    .0
                    .compare_exchange(state, new, Ordering::AcqRel, Ordering::Acquire)
                {
                    Ok(_) => return false,
                    Err(actual) => {
                        state = actual;
                        continue;
                    }
                }
            }
        }
    }

    /// A waker fired. Returns whether the caller now owns the
    /// responsibility of pushing the task onto a run queue.
    pub(super) fn wake(&self) -> StateSnapshot {
        let mut state = self.0.load(Ordering::Acquire);
        loop {
            if state & COMPLETE != 0 {
                return StateSnapshot::NoOp;
            }
            if state & RUNNING != 0 {
                if state & NOTIFIED != 0 {
                    return StateSnapshot::NoOp;
                }
                match self.0.compare_exchange(
                    state,
                    state | NOTIFIED,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => return StateSnapshot::NoOp,
                    Err(actual) => {
                        state = actual;
                        continue;
                    }
                }
            }
            if state & QUEUED != 0 {
                return StateSnapshot::NoOp;
            }
            match self.0.compare_exchange(
                state,
                state | QUEUED,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return StateSnapshot::ShouldSchedule,
                Err(actual) => {
                    state = actual;
                    continue;
                }
            }
        }
    }

    /// Mark the task as wanting to be dropped instead of polled again,
    /// piggy-backing on the same wake state machine to guarantee it
    /// gets scheduled (if idle) or re-checked (if running/queued).
    pub(super) fn request_abort(&self) -> bool {
        let state = self.0.fetch_or(ABORTED, Ordering::AcqRel);
        if state & COMPLETE != 0 || state & ABORTED != 0 {
            return false;
        }
        self.wake() == StateSnapshot::ShouldSchedule
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_state_is_queued_and_pollable() {
        let s = State::new();
        assert!(s.begin_poll());
    }

    #[test]
    fn wake_while_running_defers_to_notified() {
        let s = State::new();
        assert!(s.begin_poll());
        // Woken mid-poll: not our job to schedule, the poller will.
        assert_eq!(s.wake(), StateSnapshot::NoOp);
        assert!(s.end_poll(false));
    }

    #[test]
    fn wake_while_idle_schedules() {
        let s = State::new();
        assert!(s.begin_poll());
        assert!(!s.end_poll(false));
        assert_eq!(s.wake(), StateSnapshot::ShouldSchedule);
    }

    #[test]
    fn double_wake_only_schedules_once() {
        let s = State::new();
        assert!(s.begin_poll());
        assert!(!s.end_poll(false));
        assert_eq!(s.wake(), StateSnapshot::ShouldSchedule);
        assert_eq!(s.wake(), StateSnapshot::NoOp);
    }

    #[test]
    fn abort_between_load_and_cas() {
        let s = State::new();
        let observed = s.0.load(Ordering::Acquire);
        // Precisely the interleaving in begin_poll: QUEUED was loaded,
        // then another thread's request_abort adds ABORTED before the CAS.
        std::thread::scope(|scope| {
            scope.spawn(|| assert!(!s.request_abort())).join().unwrap();
        });
        assert!(!s.begin_poll_from(observed));
        assert_eq!(s.0.load(Ordering::Acquire), RUNNING | ABORTED);
        assert!(!s.end_poll(true));
        assert_eq!(s.0.load(Ordering::Acquire), COMPLETE | ABORTED);
        assert_eq!(s.wake(), StateSnapshot::NoOp);
        assert!(!s.request_abort());
    }

    #[test]
    fn pre_poll_abort_finishes_task_and_join() {
        use super::super::{join, Task, TaskId};
        use std::future::Future;
        use std::pin::Pin;
        use std::sync::{Arc, Mutex, Weak};
        use std::task::{Context, Poll, Wake, Waker};

        struct JoinWake(std::sync::atomic::AtomicUsize);
        impl Wake for JoinWake {
            fn wake(self: Arc<Self>) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let inner = Arc::new(join::JoinInner::<()>::new());
        let hook_inner = inner.clone();
        let id = TaskId::next();
        let mut handle = join::JoinHandle::new(inner, id);
        let task = Arc::new(Task {
            id,
            name: None,
            state: State::new(),
            future: Mutex::new(Some(Box::pin(std::future::poll_fn(|_| {
                panic!("aborted future must never be polled")
            })))),
            scheduler: Weak::new(),
            abnormal_hook: Mutex::new(Some(Box::new(move |outcome| {
                hook_inner.finish_abnormal(outcome);
            }))),
        });
        let joined = Arc::new(JoinWake(std::sync::atomic::AtomicUsize::new(0)));
        let waker = Waker::from(joined.clone());
        let mut cx = Context::from_waker(&waker);
        assert!(Pin::new(&mut handle).poll(&mut cx).is_pending());
        task.abort();
        task.clone().run();
        assert_eq!(task.state.0.load(Ordering::Acquire), COMPLETE | ABORTED);
        assert!(task.future.lock().unwrap().is_none());
        assert!(task.abnormal_hook.lock().unwrap().is_none());
        assert_eq!(joined.0.load(Ordering::SeqCst), 1);
        let Poll::Ready(Err(error)) = Pin::new(&mut handle).poll(&mut cx) else {
            panic!("aborted join did not complete");
        };
        assert!(error.is_cancelled());
        task.abort();
        Waker::from(task.clone()).wake();
        assert_eq!(task.state.0.load(Ordering::Acquire), COMPLETE | ABORTED);
        assert_eq!(joined.0.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn abort_while_running_is_rescheduled_once() {
        let s = State::new();
        assert!(s.begin_poll());
        assert!(!s.request_abort());
        assert!(!s.request_abort());
        assert_eq!(s.wake(), StateSnapshot::NoOp);
        assert!(s.end_poll(false));
        assert!(!s.begin_poll());
        assert!(!s.end_poll(true));
        assert_eq!(s.wake(), StateSnapshot::NoOp);
        assert!(!s.request_abort());
    }

    #[test]
    fn abort_while_idle_schedules_once() {
        let s = State::new();
        assert!(s.begin_poll());
        assert!(!s.end_poll(false));
        assert!(s.request_abort());
        assert!(!s.request_abort());
        assert_eq!(s.wake(), StateSnapshot::NoOp);
        assert!(!s.begin_poll());
        assert!(!s.end_poll(true));
        assert_eq!(s.wake(), StateSnapshot::NoOp);
    }

    #[test]
    fn abort_before_first_poll() {
        let s = State::new();
        assert!(!s.request_abort()); // already queued -> no extra schedule needed
        assert!(!s.begin_poll());
    }
}
