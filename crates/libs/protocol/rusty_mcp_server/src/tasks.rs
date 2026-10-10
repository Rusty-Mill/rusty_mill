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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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

/// Why [`TaskContext::elicit`] got no answer.
#[derive(Debug, PartialEq, Eq)]
pub enum ElicitError {
    /// The task was cancelled (or expired) while the handler waited.
    Cancelled,
    /// The call is not running as a task (the client did not declare the
    /// extension), so there is no `tasks/update` to answer with.
    NotATask,
}

impl std::fmt::Display for ElicitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Cancelled => "the task was cancelled",
            Self::NotATask => "the call is not running as a task",
        })
    }
}

impl std::error::Error for ElicitError {}

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
    /// When the task expires (`None` for a call that is not a task). Kept as
    /// an `Instant` so expiry needs no clock arithmetic and no API traffic: a
    /// handler waiting on this entry wakes at the deadline by itself.
    deadline: Option<Instant>,
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

    /// An entry that no store holds, for a call that is not a task.
    fn detached(cancel: CancelToken) -> Arc<Self> {
        let now = now_secs();
        Arc::new(Self {
            id: String::new(),
            created: now,
            ttl: Duration::ZERO,
            deadline: None,
            state: Mutex::new(State {
                payload: TaskPayload::Working,
                message: None,
                updated: now,
                answers: BTreeMap::new(),
            }),
            changed: Condvar::new(),
            cancel,
        })
    }

    fn expired(&self) -> bool {
        self.deadline.is_some_and(|d| Instant::now() >= d)
    }
}

/// What a task handler can do besides compute.
pub struct TaskContext {
    entry: Arc<Entry>,
    /// Running inside the request of a client without the extension.
    inline: bool,
}

impl TaskContext {
    pub(crate) fn new(entry: Arc<Entry>) -> Self {
        Self {
            entry,
            inline: false,
        }
    }

    /// The context for a task tool called by a client that cannot use tasks:
    /// the handler runs inside the request and cancellation is the request's.
    pub(crate) fn inline(cancel: CancelToken) -> Self {
        Self {
            entry: Entry::detached(cancel),
            inline: true,
        }
    }

    /// Whether the client cancelled the task (or it expired). Poll it in a
    /// long computation.
    pub fn is_cancelled(&self) -> bool {
        if self.entry.expired() {
            // Expiry cancels the task here, without waiting for a sweep.
            self.entry.cancel();
        }
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
    /// [`ElicitError::Cancelled`] if the task is cancelled or expires while
    /// waiting; [`ElicitError::NotATask`] when the client did not declare the
    /// extension (use [`Ask`](crate::Ask) in an `interactive_tool` for those).
    pub fn elicit(&self, key: &str, params: &ElicitParams) -> Result<ElicitResult, ElicitError> {
        if self.inline {
            return Err(ElicitError::NotATask);
        }
        let e = &self.entry;
        let mut s = lock(&e.state);
        if s.payload.status().is_terminal() {
            return Err(ElicitError::Cancelled);
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
                return Err(ElicitError::Cancelled);
            }
            if let Some(raw) = s.answers.remove(key) {
                s.payload = TaskPayload::Working;
                s.updated = now_secs();
                return ElicitResult::from_value(&raw).map_err(|_| ElicitError::Cancelled);
            }
            // Wait for an answer, but no longer than the task lives.
            let wait = match e.deadline {
                Some(deadline) => match deadline.checked_duration_since(Instant::now()) {
                    Some(left) if !left.is_zero() => left,
                    _ => {
                        s.payload = TaskPayload::Cancelled;
                        s.updated = now_secs();
                        e.cancel.cancel();
                        e.changed.notify_all();
                        return Err(ElicitError::Cancelled);
                    }
                },
                None => Duration::from_secs(3600),
            };
            s = e
                .changed
                .wait_timeout(s, wait)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

/// Every live task of a server.
pub(crate) struct TaskStore {
    tasks: Arc<Mutex<HashMap<String, Arc<Entry>>>>,
    max: usize,
    ttl: Duration,
    /// Whether the background sweeper is running (started with the first
    /// task, so a server without task tools never has one).
    sweeping: AtomicBool,
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
            tasks: Arc::new(Mutex::new(HashMap::new())),
            max,
            ttl,
            sweeping: AtomicBool::new(false),
        }
    }

    /// Drop (and stop) tasks past their lifetime.
    fn sweep(map: &mut HashMap<String, Arc<Entry>>) {
        map.retain(|_, e| {
            let keep = !e.expired();
            if !keep {
                e.cancel();
            }
            keep
        });
    }

    /// Sweep expired tasks on a timer, so a task nobody asks about again still
    /// expires (and a handler blocked on it is released). One thread per
    /// store, started with its first task; it ends once the store is gone.
    fn start_sweeper(&self) {
        if self.sweeping.swap(true, Ordering::SeqCst) {
            return;
        }
        let tasks: Weak<Mutex<HashMap<String, Arc<Entry>>>> = Arc::downgrade(&self.tasks);
        let every = (self.ttl / 4).clamp(Duration::from_millis(100), Duration::from_secs(30));
        let spawned = std::thread::Builder::new()
            .name("mcp-task-sweeper".to_owned())
            .spawn(move || {
                while let Some(tasks) = tasks.upgrade() {
                    Self::sweep(&mut lock(&tasks));
                    drop(tasks);
                    std::thread::sleep(every);
                }
            });
        if spawned.is_err() {
            // No thread: expiry falls back to the sweeps `create` and `get` do.
            self.sweeping.store(false, Ordering::SeqCst);
        }
    }

    pub(crate) fn create(&self) -> Result<Arc<Entry>, CreateError> {
        self.start_sweeper();
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
            deadline: Instant::now().checked_add(self.ttl),
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

    /// An entry no store (and so no sweeper) knows about: only its own
    /// deadline can end it.
    fn lone_entry(ttl: Duration) -> Arc<Entry> {
        let now = now_secs();
        Arc::new(Entry {
            id: "lone".to_owned(),
            created: now,
            ttl,
            deadline: Instant::now().checked_add(ttl),
            state: Mutex::new(State {
                payload: TaskPayload::Working,
                message: None,
                updated: now,
                answers: BTreeMap::new(),
            }),
            changed: Condvar::new(),
            cancel: CancelToken::default(),
        })
    }

    fn form() -> ElicitParams {
        ElicitParams::Form {
            message: "?".to_owned(),
            requested_schema: Value::object(),
            meta: None,
        }
    }

    /// Run `elicit` on a thread and wait for it for at most `limit`.
    fn blocked_elicit(
        ctx: TaskContext,
        limit: Duration,
    ) -> Option<Result<ElicitResult, ElicitError>> {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(ctx.elicit("k", &form()));
        });
        rx.recv_timeout(limit).ok()
    }

    #[test]
    fn a_worker_blocked_on_a_question_is_released_at_the_deadline_with_no_api_traffic() {
        let entry = lone_entry(Duration::from_secs(1));
        let started = Instant::now();
        // Nothing calls `get`, `create` or `tasks/update`, and no sweeper
        // exists: the wait itself must end at the deadline.
        let ended = blocked_elicit(TaskContext::new(Arc::clone(&entry)), Duration::from_secs(5));
        assert_eq!(
            ended.expect("the worker never woke"),
            Err(ElicitError::Cancelled)
        );
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(entry.status(), TaskStatus::Cancelled);
    }

    #[test]
    fn a_polling_handler_sees_cancellation_at_the_deadline_without_a_sweep() {
        let entry = lone_entry(Duration::from_secs(1));
        let ctx = TaskContext::new(Arc::clone(&entry));
        assert!(!ctx.is_cancelled());
        std::thread::sleep(Duration::from_millis(1200));
        assert!(ctx.is_cancelled(), "still running past its deadline");
        assert_eq!(entry.status(), TaskStatus::Cancelled);
    }

    #[test]
    fn an_abandoned_task_is_swept_without_anyone_asking() {
        let s = TaskStore::new(4, Duration::from_secs(1));
        s.create().unwrap();
        s.create().unwrap();
        assert_eq!(lock(&s.tasks).len(), 2);
        let end = Instant::now() + Duration::from_secs(5);
        while !lock(&s.tasks).is_empty() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            lock(&s.tasks).is_empty(),
            "expired tasks stayed in the store"
        );
    }

    #[test]
    fn the_sweeper_ends_with_its_store() {
        let s = TaskStore::new(2, Duration::from_secs(1));
        s.create().unwrap();
        let weak = Arc::downgrade(&s.tasks);
        drop(s);
        let end = Instant::now() + Duration::from_secs(3);
        while weak.upgrade().is_some() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(weak.upgrade().is_none(), "the store outlived its owner");
    }
}
