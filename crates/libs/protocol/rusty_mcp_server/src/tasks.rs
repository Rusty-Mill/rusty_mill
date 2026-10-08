//! Tasks (extension `io.modelcontextprotocol/tasks`, 2026-07-28): a tool
//! registered with `task_tool` answers a call at once with a task, runs on a
//! thread of its own, and the client polls `tasks/get`, answers its questions
//! with `tasks/update` and may `tasks/cancel`.
//!
//! The store is in memory and shared by every connection of a [`Server`], so
//! a stateless HTTP client may poll from any POST. It does not survive a
//! restart and does not span server instances. A task id is 128 random bits
//! and is the only credential: whoever holds it can read, answer and cancel
//! the task.

use crate::connection::CancelToken;
use rusty_json::Value;
use rusty_mcp_proto::task::{DetailedTask, Task, TaskPayload, TaskStatus};
use rusty_mcp_proto::{CallToolResult, ElicitParams, ElicitResult, ErrorData, InputRequest, Wire};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The most live tasks unless [`ServerBuilder::max_tasks`](crate::ServerBuilder::max_tasks) says otherwise.
pub const DEFAULT_MAX_TASKS: usize = 1000;
/// How long a task is kept, from its creation, unless
/// [`ServerBuilder::task_ttl`](crate::ServerBuilder::task_ttl) says otherwise.
pub const DEFAULT_TASK_TTL: Duration = Duration::from_secs(3600);
/// The polling interval suggested to clients.
const POLL_INTERVAL_MS: u64 = 1000;

/// A task tool's implementation. It runs on its own thread; the call has
/// already been answered with the task.
pub type TaskHandler = dyn Fn(&TaskContext, rusty_mcp_proto::CallToolParams) -> Result<CallToolResult, ErrorData>
    + Send
    + Sync;

/// The task was cancelled (or expired) while the handler waited.
#[derive(Debug, PartialEq, Eq)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the task was cancelled")
    }
}

impl std::error::Error for Cancelled {}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

fn iso(secs: i64) -> String {
    rusty_time::DateTime::from_unix_secs(secs)
        .map(|d| d.to_iso8601())
        .unwrap_or_default()
}

struct State {
    payload: TaskPayload,
    message: Option<String>,
    updated: i64,
    /// Answers delivered by `tasks/update`, by key, not yet taken.
    answers: BTreeMap<String, Value>,
}

/// One task. Shared by the store, the client-facing methods and the handler.
pub(crate) struct Entry {
    id: String,
    created: i64,
    ttl: Duration,
    state: Mutex<State>,
    changed: Condvar,
    cancel: CancelToken,
}

impl Entry {
    fn update(&self, f: impl FnOnce(&mut State)) {
        let mut s = lock(&self.state);
        f(&mut s);
        s.updated = now_secs();
        self.changed.notify_all();
    }

    /// The task as `tasks/get` shows it.
    pub(crate) fn snapshot(&self) -> DetailedTask {
        let s = lock(&self.state);
        let mut task = Task::new(
            self.id.as_str(),
            s.payload.status(),
            iso(self.created),
            iso(s.updated),
        );
        task.status_message = s.message.clone();
        task.ttl_ms = Some(u64::try_from(self.ttl.as_millis()).unwrap_or(u64::MAX));
        task.poll_interval_ms = Some(POLL_INTERVAL_MS);
        DetailedTask::new(task, s.payload.clone())
    }

    /// The task as `tools/call` announces it.
    pub(crate) fn announcement(&self) -> Task {
        self.snapshot().task().clone()
    }

    pub(crate) fn status(&self) -> TaskStatus {
        lock(&self.state).payload.status()
    }

    /// Record how the handler ended, unless the task already ended.
    pub(crate) fn finish(&self, outcome: Result<CallToolResult, ErrorData>) {
        self.update(|s| {
            if s.payload.status().is_terminal() {
                return;
            }
            s.payload = match outcome {
                Ok(result) => TaskPayload::Completed {
                    result: result.to_value(),
                },
                Err(error) => TaskPayload::Failed {
                    error: error.to_value(),
                },
            };
        });
    }

    /// Stop the task: its handler sees the flag and any `elicit` returns.
    pub(crate) fn cancel(&self) {
        self.cancel.cancel();
        self.update(|s| {
            if !s.payload.status().is_terminal() {
                s.payload = TaskPayload::Cancelled;
            }
        });
    }

    /// Hand the handler the answers to its pending questions.
    ///
    /// # Errors
    /// A message if the task is not waiting for input, none of its pending
    /// questions is answered, or an answer is not a well-formed elicitation
    /// result.
    pub(crate) fn answer(&self, responses: &Value) -> Result<(), String> {
        let mut s = lock(&self.state);
        let TaskPayload::InputRequired { input_requests } = &s.payload else {
            return Err("the task is not waiting for input".to_owned());
        };
        let mut taken = Vec::new();
        for key in input_requests.keys() {
            let Some(raw) = responses.get(key) else {
                continue;
            };
            ElicitResult::from_value(raw).map_err(|e| format!("inputResponses[{key:?}]: {e}"))?;
            taken.push((key.clone(), raw.clone()));
        }
        if taken.is_empty() {
            return Err("no answer for a pending question".to_owned());
        }
        s.answers.extend(taken);
        s.updated = now_secs();
        self.changed.notify_all();
        Ok(())
    }

    fn expired(&self, now: i64) -> bool {
        let ttl = i64::try_from(self.ttl.as_secs()).unwrap_or(i64::MAX);
        now > self.created.saturating_add(ttl)
    }
}

/// What a task handler can do besides compute.
pub struct TaskContext {
    entry: Arc<Entry>,
}

impl TaskContext {
    pub(crate) fn new(entry: Arc<Entry>) -> Self {
        Self { entry }
    }

    /// Whether the client cancelled the task (or it expired). Poll it in a
    /// long computation.
    pub fn is_cancelled(&self) -> bool {
        self.entry.cancel.is_cancelled()
    }

    /// Set the status line `tasks/get` shows.
    pub fn set_message(&self, message: impl Into<String>) {
        let message = message.into();
        self.entry.update(|s| s.message = Some(message));
    }

    /// Ask the user `params` and wait for the client's `tasks/update` to
    /// answer it under `key`. The task shows as `input_required` meanwhile.
    ///
    /// # Errors
    /// [`Cancelled`] if the task is cancelled or expires while waiting.
    pub fn elicit(&self, key: &str, params: &ElicitParams) -> Result<ElicitResult, Cancelled> {
        let e = &self.entry;
        let mut s = lock(&e.state);
        if s.payload.status().is_terminal() {
            return Err(Cancelled);
        }
        let mut requests = BTreeMap::new();
        requests.insert(key.to_owned(), InputRequest::elicitation(params));
        s.answers.remove(key);
        s.payload = TaskPayload::InputRequired {
            input_requests: requests,
        };
        s.updated = now_secs();
        loop {
            if s.payload.status().is_terminal() {
                return Err(Cancelled);
            }
            if let Some(raw) = s.answers.remove(key) {
                s.payload = TaskPayload::Working;
                s.updated = now_secs();
                return ElicitResult::from_value(&raw).map_err(|_| Cancelled);
            }
            s = e.changed.wait(s).unwrap_or_else(PoisonError::into_inner);
        }
    }
}

/// Every live task of a server.
pub(crate) struct TaskStore {
    tasks: Mutex<HashMap<String, Arc<Entry>>>,
    max: usize,
    ttl: Duration,
}

/// Why a task could not be created.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CreateError {
    Full,
    NoRandom,
}

impl TaskStore {
    pub(crate) fn new(max: usize, ttl: Duration) -> Self {
        Self {
            tasks: Mutex::new(HashMap::new()),
            max,
            ttl,
        }
    }

    /// Drop (and stop) tasks past their lifetime.
    fn sweep(map: &mut HashMap<String, Arc<Entry>>) {
        let now = now_secs();
        map.retain(|_, e| {
            let keep = !e.expired(now);
            if !keep {
                e.cancel();
            }
            keep
        });
    }

    pub(crate) fn create(&self) -> Result<Arc<Entry>, CreateError> {
        let mut map = lock(&self.tasks);
        Self::sweep(&mut map);
        if map.len() >= self.max {
            return Err(CreateError::Full);
        }
        let mut raw = [0u8; 16];
        rusty_rand::fill(&mut raw).map_err(|_| CreateError::NoRandom)?;
        let id = raw.iter().fold(String::with_capacity(32), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        });
        let now = now_secs();
        let entry = Arc::new(Entry {
            id: id.clone(),
            created: now,
            ttl: self.ttl,
            state: Mutex::new(State {
                payload: TaskPayload::Working,
                message: None,
                updated: now,
                answers: BTreeMap::new(),
            }),
            changed: Condvar::new(),
            cancel: CancelToken::default(),
        });
        map.insert(id, Arc::clone(&entry));
        Ok(entry)
    }

    pub(crate) fn get(&self, id: &str) -> Option<Arc<Entry>> {
        let mut map = lock(&self.tasks);
        Self::sweep(&mut map);
        map.get(id).cloned()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn store() -> TaskStore {
        TaskStore::new(2, Duration::from_secs(60))
    }

    #[test]
    fn ids_are_32_hex_digits_and_distinct() {
        let s = store();
        let (a, b) = (s.create().unwrap(), s.create().unwrap());
        assert_eq!(a.id.len(), 32);
        assert!(a.id.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn the_store_is_bounded_and_finished_tasks_still_count_until_they_expire() {
        let s = store();
        let a = s.create().unwrap();
        s.create().unwrap();
        assert_eq!(s.create().err(), Some(CreateError::Full));
        a.finish(Ok(CallToolResult::default()));
        assert_eq!(s.create().err(), Some(CreateError::Full));
    }

    #[test]
    fn an_expired_task_is_cancelled_and_forgotten() {
        let s = TaskStore::new(2, Duration::from_secs(0));
        let e = s.create().unwrap();
        std::thread::sleep(Duration::from_millis(1100));
        assert!(s.get(&e.id).is_none());
        assert!(e.cancel.is_cancelled());
    }

    #[test]
    fn finishing_cannot_undo_a_cancel() {
        let e = store().create().unwrap();
        e.cancel();
        e.finish(Ok(CallToolResult::default()));
        assert_eq!(e.status(), TaskStatus::Cancelled);
    }

    #[test]
    fn a_failure_is_recorded_as_such() {
        let e = store().create().unwrap();
        e.finish(Err(ErrorData::new(
            rusty_mcp_proto::ErrorCode::INTERNAL_ERROR,
            "boom",
        )));
        assert_eq!(e.status(), TaskStatus::Failed);
    }
}
