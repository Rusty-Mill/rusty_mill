//! Shutdowns that outlive a dropped handle but not the process.
//!
//! Closing an own-host pane hangs its console up, which ends the clients
//! still attached 5 s later (#1259). Waiting for that in the handle's Drop
//! would freeze whichever thread drops it, the GUI's event loop included.
//! A [`Reaper`] runs each wait on a small, bounded pool of threads
//! instead, and its owner joins them before the process exits, so the
//! kill still happens.

use std::collections::VecDeque;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;

/// A shutdown handed to the reaper. Each bounds its own wait.
pub(crate) type Job = Box<dyn FnOnce() + Send>;

/// Starts a worker thread running its argument.
type Spawn = fn(Box<dyn FnOnce() + Send>) -> io::Result<()>;

/// The most worker threads at once. A burst of closes queues behind
/// them instead of starting a thread each (review of 0ce71b34).
const MAX_WORKERS: usize = 8;

/// Runs jobs off the caller's thread; [`join`](Self::join) waits for every
/// job it accepted, wherever it runs. Once joined, it runs a later job on
/// the caller's thread, so handles should be dropped before the joiner.
pub(crate) struct Reaper {
    state: Mutex<State>,
    /// Signalled when `in_flight` reaches 0.
    idle: Condvar,
    spawn: Spawn,
}

#[derive(Default)]
struct State {
    queue: VecDeque<Job>,
    /// Jobs accepted and not yet finished: queued or running.
    in_flight: usize,
    /// Workers taking from `queue`, at most [`MAX_WORKERS`].
    workers: usize,
    joined: bool,
}

impl Reaper {
    pub(crate) fn new() -> Self {
        Self::with_spawn(spawn_thread)
    }

    fn with_spawn(spawn: Spawn) -> Self {
        Self {
            state: Mutex::new(State::default()),
            idle: Condvar::new(),
            spawn,
        }
    }

    /// Queues `job` for a worker, starting one if fewer than
    /// [`MAX_WORKERS`] run. If none can start, this thread works the
    /// queue, still counted, so `join` waits for it. Once joined, runs
    /// `job` here.
    pub(crate) fn run(self: &Arc<Self>, job: Job) {
        let mut state = self.lock();
        if state.joined {
            drop(state);
            run_logged(job);
            return;
        }
        state.in_flight += 1;
        state.queue.push_back(job);
        if state.workers >= MAX_WORKERS {
            return;
        }
        state.workers += 1;
        drop(state);
        let me = Arc::clone(self);
        if let Err(e) = (self.spawn)(Box::new(move || me.work())) {
            eprintln!("rusty_term: no thread for a console shutdown ({e}); waiting here");
            self.work();
        }
    }

    /// Waits until every accepted job has finished; later jobs run on the
    /// caller's thread. Bounded, as each job bounds its own wait.
    pub(crate) fn join(&self) {
        let mut state = self.lock();
        state.joined = true;
        while state.in_flight > 0 {
            state = self
                .idle
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// A worker: runs queued jobs until none is left.
    fn work(&self) {
        loop {
            let job = {
                let mut state = self.lock();
                let Some(job) = state.queue.pop_front() else {
                    state.workers -= 1;
                    return;
                };
                job
            };
            run_logged(job);
            let mut state = self.lock();
            state.in_flight -= 1;
            if state.in_flight == 0 {
                self.idle.notify_all();
            }
        }
    }

    /// The state, even if a thread panicked while holding it (none runs
    /// a job under the lock, so it is still whole).
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn spawn_thread(work: Box<dyn FnOnce() + Send>) -> io::Result<()> {
    thread::Builder::new()
        .name("rusty_term-reaper".into())
        .spawn(work)
        .map(drop)
}

/// Runs `job`, logging a panic rather than losing its count.
fn run_logged(job: Job) {
    if catch_unwind(AssertUnwindSafe(job)).is_err() {
        eprintln!("rusty_term: a console shutdown panicked");
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_WORKERS, Reaper};
    use std::io;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    const JOB: Duration = Duration::from_millis(300);

    fn slow_job(done: &Arc<AtomicBool>) -> super::Job {
        let done = Arc::clone(done);
        Box::new(move || {
            std::thread::sleep(JOB);
            done.store(true, Ordering::SeqCst);
        })
    }

    #[test]
    fn a_job_runs_off_the_callers_thread_and_join_waits_for_it() {
        let reaper = Arc::new(Reaper::new());
        let done = Arc::new(AtomicBool::new(false));
        let start = Instant::now();
        reaper.run(slow_job(&done));
        assert!(
            start.elapsed() < Duration::from_millis(100),
            "run returns at once"
        );
        assert!(!done.load(Ordering::SeqCst));
        reaper.join();
        assert!(done.load(Ordering::SeqCst), "join waited for the job");
        assert!(start.elapsed() >= JOB);
    }

    #[test]
    fn a_burst_runs_on_at_most_max_workers_and_join_waits_for_all() {
        let reaper = Arc::new(Reaper::new());
        let running = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let finished = Arc::new(AtomicUsize::new(0));
        let jobs = 3 * MAX_WORKERS;
        let start = Instant::now();
        for _ in 0..jobs {
            let (running, peak, finished) = (
                Arc::clone(&running),
                Arc::clone(&peak),
                Arc::clone(&finished),
            );
            reaper.run(Box::new(move || {
                let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(50));
                running.fetch_sub(1, Ordering::SeqCst);
                finished.fetch_add(1, Ordering::SeqCst);
            }));
        }
        assert!(
            start.elapsed() < Duration::from_millis(100),
            "no close waits"
        );
        reaper.join();
        assert_eq!(finished.load(Ordering::SeqCst), jobs);
        assert!(peak.load(Ordering::SeqCst) <= MAX_WORKERS);
        assert_eq!(reaper.lock().workers, 0, "the workers exit once idle");
    }

    #[test]
    fn a_job_after_join_runs_on_the_callers_thread() {
        let reaper = Arc::new(Reaper::new());
        reaper.join();
        let done = Arc::new(AtomicBool::new(false));
        reaper.run(slow_job(&done));
        assert!(done.load(Ordering::SeqCst), "it ran before run returned");
    }

    /// With no thread to be had, the caller runs the job, and `join` on
    /// another thread still waits for it (review of 0ce71b34).
    #[test]
    fn a_job_run_here_for_want_of_a_thread_is_still_joined() {
        let reaper = Arc::new(Reaper::with_spawn(|_| Err(io::Error::other("no threads"))));
        let done = Arc::new(AtomicBool::new(false));
        let caller = {
            let (reaper, done) = (Arc::clone(&reaper), Arc::clone(&done));
            std::thread::spawn(move || reaper.run(slow_job(&done)))
        };
        while reaper.lock().in_flight == 0 {
            std::thread::yield_now();
        }
        reaper.join();
        assert!(
            done.load(Ordering::SeqCst),
            "join waited for the inline job"
        );
        caller.join().expect("caller");
    }

    #[test]
    fn a_panicking_job_does_not_stop_join() {
        let reaper = Arc::new(Reaper::new());
        let done = Arc::new(AtomicBool::new(false));
        reaper.run(Box::new(|| panic!("a shutdown failed")));
        reaper.run(slow_job(&done));
        reaper.join();
        assert!(done.load(Ordering::SeqCst));
    }
}
