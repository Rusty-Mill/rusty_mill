//! Shutdowns that outlive a dropped handle but not the process.
//!
//! Closing an own-host pane hangs its console up, which ends the clients
//! still attached 5 s later (#1259). Waiting for that in the handle's Drop
//! would freeze whichever thread drops it, the GUI's event loop included.
//! A [`Reaper`] runs each wait on its own thread instead, and its owner
//! joins them all before the process exits, so the kill still happens.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};

/// A shutdown handed to the reaper.
pub(crate) type Job = Box<dyn FnOnce() + Send>;

/// Runs jobs on threads that [`join`](Self::join) waits for. Once joined,
/// it runs any later job on the caller's thread, so nothing is left behind.
pub(crate) struct Reaper {
    /// The running jobs, or `None` once joined.
    threads: Mutex<Option<Vec<JoinHandle<()>>>>,
}

impl Reaper {
    pub(crate) fn new() -> Self {
        Self {
            threads: Mutex::new(Some(Vec::new())),
        }
    }

    /// Runs `job` on its own thread, or here if the reaper has been joined
    /// or no thread can start.
    pub(crate) fn run(&self, job: Job) {
        let mut guard = self.lock();
        let Some(threads) = guard.as_mut() else {
            drop(guard);
            job();
            return;
        };
        threads.retain(|t| !t.is_finished());
        let slot = Arc::new(Mutex::new(Some(job)));
        let theirs = Arc::clone(&slot);
        let spawned = thread::Builder::new()
            .name("rusty_term-reaper".into())
            .spawn(move || run_taken(&theirs));
        match spawned {
            Ok(thread) => threads.push(thread),
            Err(e) => {
                drop(guard);
                eprintln!("rusty_term: no thread for a console shutdown ({e}); waiting here");
                run_taken(&slot);
            }
        }
    }

    /// Waits for every running job; later jobs run on the caller's thread.
    /// Each job bounds its own wait, so this is bounded too.
    pub(crate) fn join(&self) {
        let threads = self.lock().take().unwrap_or_default();
        for thread in threads {
            if thread.join().is_err() {
                eprintln!("rusty_term: a console shutdown panicked");
            }
        }
    }

    /// The thread list, even if a job panicked while it was held (none
    /// runs under the lock, so the list is still whole).
    fn lock(&self) -> MutexGuard<'_, Option<Vec<JoinHandle<()>>>> {
        self.threads.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Runs the job in `slot`, unless someone has taken it already.
fn run_taken(slot: &Mutex<Option<Job>>) {
    let job = slot.lock().unwrap_or_else(PoisonError::into_inner).take();
    if let Some(job) = job {
        job();
    }
}

#[cfg(test)]
mod tests {
    use super::Reaper;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
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
        let reaper = Reaper::new();
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
    fn join_waits_for_every_job() {
        let reaper = Reaper::new();
        let flags: Vec<_> = (0..3).map(|_| Arc::new(AtomicBool::new(false))).collect();
        for done in &flags {
            reaper.run(slow_job(done));
        }
        reaper.join();
        assert!(flags.iter().all(|d| d.load(Ordering::SeqCst)));
    }

    #[test]
    fn a_job_after_join_runs_on_the_callers_thread() {
        let reaper = Reaper::new();
        reaper.join();
        let done = Arc::new(AtomicBool::new(false));
        reaper.run(slow_job(&done));
        assert!(done.load(Ordering::SeqCst), "it ran before run returned");
    }

    #[test]
    fn a_panicking_job_does_not_stop_join() {
        let reaper = Reaper::new();
        let done = Arc::new(AtomicBool::new(false));
        reaper.run(Box::new(|| panic!("a shutdown failed")));
        reaper.run(slow_job(&done));
        reaper.join();
        assert!(done.load(Ordering::SeqCst));
    }
}
