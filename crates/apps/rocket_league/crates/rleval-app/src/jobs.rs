//! Asynchronous analysis jobs: an explicit lifecycle instead of one long request.
//!
//! `queued → running → done | failed`, held in memory per account and bounded: at most
//! [`MAX_ACTIVE`] unfinished jobs, and the oldest finished ones are dropped past
//! [`KEEP`]. Status is read by long-polling ([`Jobs::wait`]): the caller passes the state
//! it already knows and the call returns when the job moves past it (or after a timeout),
//! so a client learns of a change at once without polling in a loop.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::store::AccountId;

/// Unfinished jobs allowed at once (a new submission past this is refused).
pub const MAX_ACTIVE: usize = 4;
/// Jobs remembered, finished ones evicted oldest first.
pub const KEEP: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Done,
    Failed,
}

impl JobState {
    pub fn finished(self) -> bool {
        matches!(self, Self::Done | Self::Failed)
    }

    /// The state named by a client (`?since=running`).
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "queued" => Some(Self::Queued),
            "running" => Some(Self::Running),
            "done" => Some(Self::Done),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

/// What a client sees of a job.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Status {
    pub job: String,
    pub state: JobState,
    /// Why it failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

struct Job {
    owner: String,
    state: JobState,
    error: Option<String>,
    result: Option<Arc<Vec<u8>>>,
}

#[derive(Default)]
struct Inner {
    jobs: HashMap<String, Job>,
    order: VecDeque<String>,
    counter: u64,
}

#[derive(Default)]
pub struct Jobs {
    inner: Mutex<Inner>,
    changed: Condvar,
}

/// The registry is unusable only if a worker panicked while holding it; recover the data.
fn lock(m: &Mutex<Inner>) -> MutexGuard<'_, Inner> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Jobs {
    /// Register a job for `owner`, or `None` when [`MAX_ACTIVE`] are already unfinished.
    pub fn submit(&self, owner: &AccountId) -> Option<String> {
        let mut g = lock(&self.inner);
        if g.jobs.values().filter(|j| !j.state.finished()).count() >= MAX_ACTIVE {
            return None;
        }
        g.counter += 1;
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        let id = format!(
            "{:016x}",
            nanos ^ g.counter.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        );
        g.jobs.insert(
            id.clone(),
            Job {
                owner: owner.as_str().into(),
                state: JobState::Queued,
                error: None,
                result: None,
            },
        );
        g.order.push_back(id.clone());
        while g.order.len() > KEEP {
            let Some(old) = g
                .order
                .iter()
                .position(|k| g.jobs.get(k).is_some_and(|j| j.state.finished()))
            else {
                break;
            };
            if let Some(k) = g.order.remove(old) {
                g.jobs.remove(&k);
            }
        }
        Some(id)
    }

    pub fn start(&self, id: &str) {
        self.set(id, JobState::Running, None, None);
    }

    /// Finish `id` with its result JSON, or the reason it failed.
    pub fn finish(&self, id: &str, outcome: Result<Vec<u8>, String>) {
        match outcome {
            Ok(json) => self.set(id, JobState::Done, None, Some(Arc::new(json))),
            Err(e) => self.set(id, JobState::Failed, Some(e), None),
        }
    }

    fn set(&self, id: &str, state: JobState, error: Option<String>, result: Option<Arc<Vec<u8>>>) {
        if let Some(j) = lock(&self.inner).jobs.get_mut(id) {
            (j.state, j.error, j.result) = (state, error, result);
        }
        self.changed.notify_all();
    }

    fn status_of(g: &Inner, owner: &AccountId, id: &str) -> Option<Status> {
        let j = g.jobs.get(id).filter(|j| j.owner == owner.as_str())?;
        Some(Status {
            job: id.into(),
            state: j.state,
            error: j.error.clone(),
        })
    }

    /// The job's status; `None` if it does not exist or is another account's.
    pub fn status(&self, owner: &AccountId, id: &str) -> Option<Status> {
        Self::status_of(&lock(&self.inner), owner, id)
    }

    /// Like [`Jobs::status`], but if the job is still in `since`, wait up to `timeout` for it to move on.
    pub fn wait(
        &self,
        owner: &AccountId,
        id: &str,
        since: JobState,
        timeout: Duration,
    ) -> Option<Status> {
        let g = lock(&self.inner);
        let (g, _) = self
            .changed
            .wait_timeout_while(g, timeout, |g| {
                Self::status_of(g, owner, id).is_some_and(|s| s.state == since && !since.finished())
            })
            .unwrap_or_else(|e| e.into_inner());
        Self::status_of(&g, owner, id)
    }

    /// The finished job's result JSON.
    pub fn result(&self, owner: &AccountId, id: &str) -> Option<Arc<Vec<u8>>> {
        let g = lock(&self.inner);
        g.jobs
            .get(id)
            .filter(|j| j.owner == owner.as_str())?
            .result
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acct(s: &str) -> AccountId {
        AccountId::new(s).expect("valid account")
    }

    #[test]
    fn a_job_moves_queued_running_done_and_only_its_owner_sees_it() {
        let jobs = Jobs::default();
        let (ann, bob) = (acct("ann"), acct("bob"));
        let id = jobs.submit(&ann).unwrap();
        assert_eq!(jobs.status(&ann, &id).unwrap().state, JobState::Queued);
        jobs.start(&id);
        assert_eq!(jobs.status(&ann, &id).unwrap().state, JobState::Running);
        assert!(jobs.result(&ann, &id).is_none(), "no result while running");
        jobs.finish(&id, Ok(b"{}".to_vec()));
        assert_eq!(jobs.status(&ann, &id).unwrap().state, JobState::Done);
        assert_eq!(
            jobs.result(&ann, &id).as_deref().map(Vec::as_slice),
            Some(&b"{}"[..])
        );
        assert!(jobs.status(&bob, &id).is_none() && jobs.result(&bob, &id).is_none());
    }

    #[test]
    fn a_failure_keeps_its_reason() {
        let jobs = Jobs::default();
        let a = acct("ann");
        let id = jobs.submit(&a).unwrap();
        jobs.finish(&id, Err("could not decode".into()));
        let s = jobs.status(&a, &id).unwrap();
        assert_eq!(
            (s.state, s.error.as_deref()),
            (JobState::Failed, Some("could not decode"))
        );
    }

    #[test]
    fn too_many_unfinished_jobs_are_refused_until_one_finishes() {
        let jobs = Jobs::default();
        let a = acct("ann");
        let ids: Vec<_> = (0..MAX_ACTIVE).map(|_| jobs.submit(&a).unwrap()).collect();
        assert!(jobs.submit(&a).is_none());
        jobs.finish(&ids[0], Ok(vec![]));
        assert!(jobs.submit(&a).is_some());
    }

    #[test]
    fn finished_jobs_are_evicted_oldest_first_past_the_cap() {
        let jobs = Jobs::default();
        let a = acct("ann");
        let first = jobs.submit(&a).unwrap();
        jobs.finish(&first, Ok(vec![]));
        for _ in 0..KEEP {
            let id = jobs.submit(&a).unwrap();
            jobs.finish(&id, Ok(vec![]));
        }
        assert!(
            jobs.status(&a, &first).is_none(),
            "the oldest finished job was dropped"
        );
    }

    #[test]
    fn wait_returns_at_once_on_a_change_and_times_out_otherwise() {
        let jobs = Arc::new(Jobs::default());
        let a = acct("ann");
        let id = jobs.submit(&a).unwrap();
        let t = std::time::Instant::now();
        let s = jobs
            .wait(&a, &id, JobState::Queued, Duration::from_millis(60))
            .unwrap();
        assert_eq!(s.state, JobState::Queued, "timed out unchanged");
        assert!(t.elapsed() >= Duration::from_millis(50));
        let (j2, id2) = (Arc::clone(&jobs), id.clone());
        let h = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            j2.start(&id2);
        });
        let t = std::time::Instant::now();
        let s = jobs
            .wait(&a, &id, JobState::Queued, Duration::from_secs(5))
            .unwrap();
        assert_eq!(s.state, JobState::Running);
        assert!(
            t.elapsed() < Duration::from_secs(2),
            "woken by the change, not the timeout"
        );
        h.join().unwrap();
    }
}
