//! The local handle for a run this replica is executing.
//!
//! A [`RunHandle`] is deliberately *not* stored in the [`Store`]: it holds the
//! things that only make sense in the process running the agent — the
//! cancellation token tied to the agent's future, and the channel that hands a
//! resume payload to it. Everything durable goes through the store, so other
//! replicas can serve reads and route control signals in.

use std::sync::{Arc, Mutex};

use chrono::Utc;
use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    server::store::{Notification, Store},
    types::{
        AwaitRequest, AwaitResume, Error, Event, Message, MessagePart, Role, Run, RunId, RunStatus,
    },
};

#[derive(Debug)]
struct LocalState {
    run: Run,
    /// Parts accumulated since the last `message.created`.
    pending: Option<Message>,
}

/// Tracks one run being executed by this replica.
///
/// Every mutation writes through to the [`Store`] and publishes a
/// [`Notification`], so a client attached to any replica sees the same thing.
#[derive(Debug)]
pub struct RunHandle {
    store: Arc<dyn Store>,
    state: Mutex<LocalState>,
    cancel: CancellationToken,
    resume_tx: mpsc::Sender<AwaitResume>,
}

impl RunHandle {
    /// Create a handle for `run` and the receiver the agent awaits on.
    pub(crate) fn new(store: Arc<dyn Store>, run: Run) -> (Arc<Self>, mpsc::Receiver<AwaitResume>) {
        let (resume_tx, resume_rx) = mpsc::channel(1);
        let handle = Arc::new(Self {
            store,
            state: Mutex::new(LocalState { run, pending: None }),
            cancel: CancellationToken::new(),
            resume_tx,
        });
        (handle, resume_rx)
    }

    /// A snapshot of the run as this replica sees it.
    pub fn snapshot(&self) -> Run {
        self.state.lock().expect("run state poisoned").run.clone()
    }

    /// The run's identifier.
    pub fn run_id(&self) -> RunId {
        self.state.lock().expect("run state poisoned").run.run_id
    }

    /// The run's current status.
    pub fn status(&self) -> RunStatus {
        self.state.lock().expect("run state poisoned").run.status
    }

    /// The token signalled when cancellation is requested.
    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    /// The store backing this run.
    pub fn store(&self) -> &Arc<dyn Store> {
        &self.store
    }

    /// Listen for control signals aimed at this run and apply them locally.
    ///
    /// This is what makes cancel and resume work across replicas: a client can
    /// hit any replica, which publishes the signal; this task runs on the
    /// replica actually executing the run and turns the signal back into a
    /// local cancellation or resume delivery. Events are ignored — this
    /// replica is the one that published them.
    pub(crate) fn spawn_control_listener(
        self: &Arc<Self>,
        mut notifications: crate::server::store::NotificationStream,
    ) {
        let handle = Arc::clone(self);
        tokio::spawn(async move {
            while let Some(notification) = notifications.next().await {
                match notification {
                    Notification::Cancel => {
                        // Records `cancelling` and signals the token; the
                        // executor observes it and writes the terminal state.
                        if let Err(error) = handle.set_cancelling().await {
                            tracing::warn!(%error, "failed to record cancellation");
                            handle.cancel.cancel();
                        }
                    }
                    Notification::Resume(payload) => {
                        // A full channel means a resume is already in flight;
                        // dropping the duplicate is correct.
                        let _ = handle.resume_tx.try_send(payload);
                    }
                    Notification::Event(_) => {}
                }
                if handle.status().is_terminal() {
                    break;
                }
            }
        });
    }

    /// Close off the run's output and return it, flushing a message the agent
    /// left open by returning mid-message.
    ///
    /// Split out of the terminal transition rather than left inside it because
    /// the caller has work to do *between* the output being final and the run
    /// being marked terminal — recording it in the session. The terminal event
    /// is what releases a `sync` caller, so anything that must be visible to
    /// that caller has to happen before it, and that in turn needs the output
    /// finalised first.
    ///
    /// Returns an empty vec for an already-terminal run, whose output was
    /// recorded when it got there.
    pub(crate) async fn finalize_output(&self) -> Result<Vec<Message>, Error> {
        let (flushed, output) = {
            let mut state = self.state.lock().expect("run state poisoned");
            if state.run.status.is_terminal() {
                return Ok(Vec::new());
            }
            let mut flushed = None;
            if let Some(mut pending) = state.pending.take() {
                if !pending.parts.is_empty() {
                    pending.complete();
                    state.run.output.push(pending.clone());
                    flushed = Some(pending);
                }
            }
            (flushed, state.run.output.clone())
        };

        if let Some(message) = flushed {
            let run_id = self.run_id();
            let event = Event::MessageCompleted { message };
            self.store.append_event(run_id, &event).await?;
            self.store.publish(run_id, Notification::Event(event)).await?;
        }
        Ok(output)
    }

    /// Record an event: update local state, append it to the durable log, and
    /// publish it to every subscriber.
    pub async fn emit(&self, event: Event) -> Result<(), Error> {
        let run_id = {
            let mut state = self.state.lock().expect("run state poisoned");
            match &event {
                Event::MessageCreated { message } => {
                    state.pending = Some(message.clone());
                }
                Event::MessagePart { part } => match state.pending.as_mut() {
                    Some(message) => message.parts.push(part.clone()),
                    None => {
                        let role = Role::agent(state.run.agent_name.as_str());
                        state.pending = Some(Message {
                            role,
                            parts: vec![part.clone()],
                            created_at: Some(Utc::now()),
                            completed_at: None,
                        });
                    }
                },
                Event::MessageCompleted { message } => {
                    state.pending = None;
                    state.run.output.push(message.clone());
                }
                _ => {}
            }
            state.run.run_id
        };

        self.store.append_event(run_id, &event).await?;
        self.store.publish(run_id, Notification::Event(event)).await
    }

    /// Emit a `message.part` for the message currently being composed.
    pub async fn emit_part(&self, part: MessagePart) -> Result<(), Error> {
        self.emit(Event::MessagePart { part }).await
    }

    /// The message currently being composed, if any.
    pub fn pending_message(&self) -> Option<Message> {
        self.state.lock().expect("run state poisoned").pending.clone()
    }

    /// Transition the run, persisting the snapshot and emitting the matching
    /// `run.*` event.
    ///
    /// Terminal transitions are applied once; later attempts are ignored, so a
    /// cancellation racing a completion cannot rewrite the outcome.
    async fn transition(
        &self,
        status: RunStatus,
        await_request: Option<AwaitRequest>,
        error: Option<Error>,
    ) -> Result<(), Error> {
        let (snapshot, flushed) = {
            let mut state = self.state.lock().expect("run state poisoned");
            if state.run.status.is_terminal() {
                return Ok(());
            }
            // Flush a message left open by an agent that returned mid-message.
            // `finalize_output` has normally taken it already; this covers any
            // terminal write that did not go through it.
            let mut flushed = None;
            if status.is_terminal() {
                if let Some(mut pending) = state.pending.take() {
                    if !pending.parts.is_empty() {
                        pending.complete();
                        state.run.output.push(pending.clone());
                        flushed = Some(pending);
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

        let run_id = snapshot.run_id;

        if let Some(message) = flushed {
            let event = Event::MessageCompleted { message };
            self.store.append_event(run_id, &event).await?;
            self.store.publish(run_id, Notification::Event(event)).await?;
        }

        // Persist before publishing, so anyone woken by the notification who
        // then reads the run sees the new state rather than the old one.
        self.store.put_run(&snapshot).await?;

        if let Some(event) = Event::for_run(snapshot) {
            self.store.append_event(run_id, &event).await?;
            self.store.publish(run_id, Notification::Event(event)).await?;
        }
        Ok(())
    }

    pub(crate) async fn set_created(&self) -> Result<(), Error> {
        self.transition(RunStatus::Created, None, None).await
    }

    pub(crate) async fn set_in_progress(&self) -> Result<(), Error> {
        self.transition(RunStatus::InProgress, None, None).await
    }

    pub(crate) async fn set_awaiting(&self, request: AwaitRequest) -> Result<(), Error> {
        self.transition(RunStatus::Awaiting, Some(request), None).await
    }

    pub(crate) async fn set_completed(&self) -> Result<(), Error> {
        self.transition(RunStatus::Completed, None, None).await
    }

    pub(crate) async fn set_failed(&self, error: Error) -> Result<(), Error> {
        self.transition(RunStatus::Failed, None, Some(error)).await
    }

    pub(crate) async fn set_cancelled(&self) -> Result<(), Error> {
        self.transition(RunStatus::Cancelled, None, None).await
    }

    /// Mark the run `cancelling` and signal the agent's future.
    ///
    /// Called on the executing replica once a [`Notification::Cancel`] arrives.
    /// The run reaches `cancelled` only when the executor has actually stopped.
    pub(crate) async fn set_cancelling(&self) -> Result<(), Error> {
        let snapshot = {
            let mut state = self.state.lock().expect("run state poisoned");
            if state.run.status.is_terminal() {
                return Ok(());
            }
            state.run.status = RunStatus::Cancelling;
            state.run.clone()
        };
        self.cancel.cancel();
        self.store.put_run(&snapshot).await
    }
}
