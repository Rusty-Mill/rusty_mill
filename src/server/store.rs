//! In-memory storage for runs and sessions.
//!
//! The store owns the authoritative [`Run`] snapshot, its event log, and the
//! channels used to stream events, deliver resume payloads and request
//! cancellation.

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, RwLock},
};

use chrono::Utc;
use tokio::sync::{broadcast, mpsc, watch};
use tokio_util::sync::CancellationToken;

use crate::types::{
    AwaitRequest, AwaitResume, Error, Event, Message, MessagePart, Role, Run, RunId, RunStatus,
    Session, SessionId,
};

/// Capacity of the per-run event broadcast channel.
const EVENT_CHANNEL_CAPACITY: usize = 256;

/// Default number of runs retained before terminal ones are evicted.
pub const DEFAULT_MAX_RUNS: usize = 1024;

#[derive(Debug)]
struct RunState {
    run: Run,
    events: Vec<Event>,
    /// Parts accumulated since the last `message.created`.
    pending: Option<Message>,
}

/// The live state of one run: its snapshot, event log and control channels.
#[derive(Debug)]
pub struct RunHandle {
    state: Mutex<RunState>,
    events_tx: broadcast::Sender<Event>,
    status_tx: watch::Sender<RunStatus>,
    resume_tx: mpsc::Sender<AwaitResume>,
    cancel: CancellationToken,
}

impl RunHandle {
    fn new(run: Run) -> (Arc<Self>, mpsc::Receiver<AwaitResume>) {
        let (events_tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        let (status_tx, _) = watch::channel(run.status);
        let (resume_tx, resume_rx) = mpsc::channel(1);
        let handle = Arc::new(Self {
            state: Mutex::new(RunState { run, events: Vec::new(), pending: None }),
            events_tx,
            status_tx,
            resume_tx,
            cancel: CancellationToken::new(),
        });
        (handle, resume_rx)
    }

    /// A snapshot of the current run.
    pub fn snapshot(&self) -> Run {
        self.state.lock().expect("run state poisoned").run.clone()
    }

    /// The run's current status.
    pub fn status(&self) -> RunStatus {
        *self.status_tx.borrow()
    }

    /// The run's identifier.
    pub fn run_id(&self) -> RunId {
        self.state.lock().expect("run state poisoned").run.run_id
    }

    /// Every event emitted so far, in order.
    pub fn events(&self) -> Vec<Event> {
        self.state.lock().expect("run state poisoned").events.clone()
    }

    /// Subscribe to events emitted from now on.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events_tx.subscribe()
    }

    /// Watch the run's status. The receiver starts at the current value.
    pub fn watch_status(&self) -> watch::Receiver<RunStatus> {
        self.status_tx.subscribe()
    }

    /// The token signalled when cancellation is requested.
    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    /// Deliver a resume payload to an awaiting agent.
    pub async fn send_resume(&self, resume: AwaitResume) -> Result<(), Error> {
        self.resume_tx
            .send(resume)
            .await
            .map_err(|_| Error::server_error("run is no longer accepting a resume payload"))
    }

    /// Record and broadcast an event, applying its effect on the run snapshot.
    ///
    /// Broadcast failure (no live subscribers) is not an error: the event is
    /// still appended to the log served by `GET /runs/{run_id}/events`.
    pub fn emit(&self, event: Event) {
        {
            let mut state = self.state.lock().expect("run state poisoned");
            match &event {
                Event::MessageCreated { message } => {
                    state.pending = Some(message.clone());
                }
                Event::MessagePart { part } => match state.pending.as_mut() {
                    Some(message) => message.parts.push(part.clone()),
                    None => {
                        let mut message = Message {
                            role: Role::agent(state.run.agent_name.as_str()),
                            parts: Vec::new(),
                            created_at: Some(Utc::now()),
                            completed_at: None,
                        };
                        message.parts.push(part.clone());
                        state.pending = Some(message);
                    }
                },
                Event::MessageCompleted { message } => {
                    state.pending = None;
                    state.run.output.push(message.clone());
                }
                _ => {}
            }
            state.events.push(event.clone());
        }
        let _ = self.events_tx.send(event);
    }

    /// The message currently being composed, if any.
    pub fn pending_message(&self) -> Option<Message> {
        self.state.lock().expect("run state poisoned").pending.clone()
    }

    /// Emit a `message.part` for the message currently being composed.
    pub fn emit_part(&self, part: MessagePart) {
        self.emit(Event::MessagePart { part });
    }

    /// Transition the run, emitting the matching `run.*` event.
    ///
    /// Terminal transitions are applied once; later attempts are ignored so a
    /// cancellation racing a completion cannot rewrite the outcome.
    fn transition(
        &self,
        status: RunStatus,
        await_request: Option<AwaitRequest>,
        error: Option<Error>,
    ) -> bool {
        let (snapshot, flushed) = {
            let mut state = self.state.lock().expect("run state poisoned");
            if state.run.status.is_terminal() {
                return false;
            }
            // Flush a message left open by an agent that returned mid-message.
            let mut flushed = None;
            if status.is_terminal() {
                if let Some(mut pending) = state.pending.take() {
                    if !pending.parts.is_empty() {
                        pending.complete();
                        let event = Event::MessageCompleted { message: pending.clone() };
                        state.events.push(event.clone());
                        state.run.output.push(pending);
                        flushed = Some(event);
                    }
                }
            }
            state.run.status = status;
            state.run.await_request = await_request;
            if error.is_some() {
                state.run.error = error;
            }
            if status.is_terminal() {
                state.run.finished_at = Some(Utc::now());
            }
            (state.run.clone(), flushed)
        };

        if let Some(event) = flushed {
            let _ = self.events_tx.send(event);
        }
        let _ = self.status_tx.send(status);
        if let Some(event) = Event::for_run(snapshot) {
            let event_for_log = event.clone();
            {
                let mut state = self.state.lock().expect("run state poisoned");
                state.events.push(event_for_log);
            }
            let _ = self.events_tx.send(event);
        }
        true
    }

    pub(crate) fn set_created(&self) {
        self.transition(RunStatus::Created, None, None);
    }

    pub(crate) fn set_in_progress(&self) {
        self.transition(RunStatus::InProgress, None, None);
    }

    pub(crate) fn set_awaiting(&self, request: AwaitRequest) {
        self.transition(RunStatus::Awaiting, Some(request), None);
    }

    pub(crate) fn set_completed(&self) {
        self.transition(RunStatus::Completed, None, None);
    }

    pub(crate) fn set_failed(&self, error: Error) {
        self.transition(RunStatus::Failed, None, Some(error));
    }

    pub(crate) fn set_cancelled(&self) {
        self.transition(RunStatus::Cancelled, None, None);
    }

    /// Move the run to `cancelling` and signal the executor.
    ///
    /// Returns `false` if the run had already finished.
    pub fn request_cancel(&self) -> bool {
        let moved = {
            let mut state = self.state.lock().expect("run state poisoned");
            if state.run.status.is_terminal() {
                false
            } else {
                state.run.status = RunStatus::Cancelling;
                true
            }
        };
        if moved {
            let _ = self.status_tx.send(RunStatus::Cancelling);
            self.cancel.cancel();
        }
        moved
    }
}

/// A session together with the messages this server holds for it.
#[derive(Debug, Clone)]
pub struct SessionRecord {
    /// The session as returned by `GET /session/{session_id}`.
    pub session: Session,
    /// Messages stored locally, indexed by their position in this server's
    /// contribution to the session history.
    pub messages: Vec<Message>,
}

/// In-memory store of runs and sessions.
///
/// Runs are retained up to `max_runs`; beyond that the oldest terminal runs are
/// evicted. Active runs are never evicted.
#[derive(Debug)]
pub struct Store {
    runs: RwLock<HashMap<RunId, Arc<RunHandle>>>,
    run_order: Mutex<VecDeque<RunId>>,
    sessions: RwLock<HashMap<SessionId, SessionRecord>>,
    max_runs: usize,
}

impl Default for Store {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_RUNS)
    }
}

impl Store {
    /// A store retaining at most `max_runs` runs.
    pub fn new(max_runs: usize) -> Self {
        Self {
            runs: RwLock::new(HashMap::new()),
            run_order: Mutex::new(VecDeque::new()),
            sessions: RwLock::new(HashMap::new()),
            max_runs: max_runs.max(1),
        }
    }

    /// Register a new run, returning its handle and the resume receiver that
    /// the executor hands to the agent.
    pub fn insert_run(&self, run: Run) -> (Arc<RunHandle>, mpsc::Receiver<AwaitResume>) {
        let run_id = run.run_id;
        let (handle, resume_rx) = RunHandle::new(run);
        {
            let mut runs = self.runs.write().expect("run map poisoned");
            runs.insert(run_id, Arc::clone(&handle));
            let mut order = self.run_order.lock().expect("run order poisoned");
            order.push_back(run_id);
            self.evict_locked(&mut runs, &mut order);
        }
        (handle, resume_rx)
    }

    fn evict_locked(&self, runs: &mut HashMap<RunId, Arc<RunHandle>>, order: &mut VecDeque<RunId>) {
        while runs.len() > self.max_runs {
            let Some(position) = order
                .iter()
                .position(|id| runs.get(id).is_some_and(|handle| handle.status().is_terminal()))
            else {
                // Every retained run is still active; keep them all.
                break;
            };
            if let Some(id) = order.remove(position) {
                runs.remove(&id);
            }
        }
    }

    /// Look up a run handle.
    pub fn run(&self, run_id: RunId) -> Option<Arc<RunHandle>> {
        self.runs.read().expect("run map poisoned").get(&run_id).cloned()
    }

    /// Look up a run handle, or produce a `not_found` error.
    pub fn require_run(&self, run_id: RunId) -> Result<Arc<RunHandle>, Error> {
        self.run(run_id).ok_or_else(|| Error::not_found(format!("run {run_id} not found")))
    }

    /// Look up a session record.
    pub fn session(&self, session_id: SessionId) -> Option<SessionRecord> {
        self.sessions.read().expect("session map poisoned").get(&session_id).cloned()
    }

    /// Look up a session, or produce a `not_found` error.
    pub fn require_session(&self, session_id: SessionId) -> Result<SessionRecord, Error> {
        self.session(session_id)
            .ok_or_else(|| Error::not_found(format!("session {session_id} not found")))
    }

    /// Create the session if absent, seeding it from a client-supplied
    /// [`Session`] so history hosted by other servers is preserved.
    pub fn ensure_session(&self, session: Session) -> SessionRecord {
        let mut sessions = self.sessions.write().expect("session map poisoned");
        sessions
            .entry(session.id)
            .or_insert_with(|| SessionRecord { session, messages: Vec::new() })
            .clone()
    }

    /// Append messages to a session, extending its history with URLs that
    /// resolve against `base_url`.
    pub fn append_session_messages(
        &self,
        session_id: SessionId,
        base_url: &str,
        messages: impl IntoIterator<Item = Message>,
    ) {
        let mut sessions = self.sessions.write().expect("session map poisoned");
        let record = sessions.entry(session_id).or_insert_with(|| SessionRecord {
            session: Session::with_id(session_id),
            messages: Vec::new(),
        });
        for message in messages {
            let index = record.messages.len();
            record.messages.push(message);
            record.session.history.push(message_url(base_url, session_id, index));
        }
    }
}

/// Build the resource URL for a stored session message.
pub fn message_url(base_url: &str, session_id: SessionId, index: usize) -> String {
    format!("{}/session/{}/messages/{}", base_url.trim_end_matches('/'), session_id, index)
}
