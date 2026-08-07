//! The transport-agnostic core of an A2A server: implements all eleven
//! A2A operations (spec Section 3.1) against an [`AgentExecutor`] and a
//! [`TaskStore`], independent of the JSON-RPC framing used to expose them.

use std::collections::{HashMap, HashSet, VecDeque};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_stream::stream;
use chrono::Utc;
use futures_core::Stream;
use tokio::sync::{broadcast, mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// How many recent events [`subscribe_to_task`](Engine::subscribe_to_task)
/// can replay per task on reconnect - matches the broadcast bus's own
/// buffer size (`start_execution`), since a receiver that lags past this
/// many *live* events already drops some regardless.
const EVENT_LOG_CAPACITY: usize = 256;

/// A task's assigned sequence number alongside its event, per the
/// [`Engine`] docs on `event_logs`/`next_seq`.
type SeqEvent = (u64, StreamResponse);

/// Per-task buses of live [`SeqEvent`]s, keyed by task id.
type Buses = Arc<Mutex<HashMap<String, broadcast::Sender<SeqEvent>>>>;

/// Per-task bounded tails of recent [`SeqEvent`]s, keyed by task id.
type EventLogs = Arc<Mutex<HashMap<String, VecDeque<SeqEvent>>>>;

use crate::error::{A2aError, Result};
use crate::types::{
    AgentCard, CancelTaskRequest, DeleteTaskPushNotificationConfigRequest,
    GetTaskPushNotificationConfigRequest, GetTaskRequest, ListTaskPushNotificationConfigsRequest,
    ListTaskPushNotificationConfigsResponse, ListTasksRequest, ListTasksResponse, Message, Role,
    SecurityRequirement, SendMessageRequest, SendMessageResult, StreamResponse, SubscribeToTaskRequest, Task,
    TaskArtifactUpdateEvent, TaskPushNotificationConfig, TaskState, TaskStatus, TaskStatusUpdateEvent,
};

use super::auth::{authenticate_against, AuthContext, AuthVerifier, Credentials};
use super::executor::{AgentExecutor, EventSink, RequestContext};
use super::push::PushNotifier;
use super::store::TaskStore;

/// Ids assigned to a `SendMessage` invocation before its outcome (task or
/// bare message) is known.
struct Started {
    task_id: String,
    context_id: String,
    tenant: Option<String>,
}

pub struct Engine {
    card: AgentCard,
    extended_card: Option<AgentCard>,
    executor: Arc<dyn AgentExecutor>,
    store: Arc<dyn TaskStore>,
    buses: Buses,
    cancel_tokens: Arc<Mutex<HashMap<String, CancellationToken>>>,
    auth_verifier: Option<Arc<dyn AuthVerifier>>,
    /// The header/metadata key name (if any) an `mtls` security scheme's
    /// credential is read from - see [`AgentServer::with_mtls_header`](super::AgentServer::with_mtls_header).
    mtls_header: Option<String>,
    push_notifier: PushNotifier,
    /// A bounded tail of recent events per task, keyed by task id, so
    /// [`Engine::subscribe_to_task`] can replay what a reconnecting caller
    /// missed instead of only a point-in-time snapshot.
    event_logs: EventLogs,
    /// A single monotonic counter shared by every task's event log, so a
    /// `Last-Event-ID` is always unambiguous to compare regardless of
    /// which task it came from.
    next_seq: Arc<AtomicU64>,
}

impl Engine {
    pub fn new(card: AgentCard, executor: Arc<dyn AgentExecutor>, store: Arc<dyn TaskStore>) -> Self {
        Engine {
            card,
            extended_card: None,
            executor,
            store,
            buses: Arc::new(Mutex::new(HashMap::new())),
            cancel_tokens: Arc::new(Mutex::new(HashMap::new())),
            auth_verifier: None,
            mtls_header: None,
            push_notifier: PushNotifier::new(),
            event_logs: Arc::new(Mutex::new(HashMap::new())),
            next_seq: Arc::new(AtomicU64::new(1)),
        }
    }

    pub(crate) fn set_extended_card(&mut self, card: AgentCard) {
        self.extended_card = Some(card);
    }

    pub(crate) fn set_auth_verifier(&mut self, verifier: Arc<dyn AuthVerifier>) {
        self.auth_verifier = Some(verifier);
    }

    pub(crate) fn set_mtls_header(&mut self, header_name: String) {
        self.mtls_header = Some(header_name);
    }

    pub fn card(&self) -> &AgentCard {
        &self.card
    }

    /// The header/metadata key name `mtls` security scheme credentials are
    /// read from, if [`AgentServer::with_mtls_header`](super::AgentServer::with_mtls_header)
    /// was called.
    pub(crate) fn mtls_header(&self) -> Option<&str> {
        self.mtls_header.as_deref()
    }

    /// Enforces `AgentCard.capabilities.extensions[].required` (spec
    /// Section 3.2.6 / 5.6): rejects a request that doesn't declare
    /// support (via `declared`, the request's parsed `A2A-Extensions`
    /// service parameter) for every extension this agent marks
    /// `required`.
    pub(crate) fn check_required_extensions(&self, declared: &HashSet<String>) -> Result<()> {
        for ext in &self.card.capabilities.extensions {
            if ext.required && !declared.contains(&ext.uri) {
                return Err(A2aError::ExtensionSupportRequired(ext.uri.clone()));
            }
        }
        Ok(())
    }

    /// Enforces `AgentCard.securityRequirements` (spec Section 4.5)
    /// against `credentials` extracted from an incoming request. An empty
    /// requirement list means the agent is public and this always
    /// succeeds with `Ok(None)` - no [`AuthVerifier`] is required or
    /// consulted in that case. A non-empty list without a configured
    /// verifier fails closed: declaring requirements is a statement of
    /// intent, and silently accepting every request would defeat it.
    pub(crate) async fn authenticate(&self, credentials: &Credentials) -> Result<Option<AuthContext>> {
        self.authenticate_requirements(&self.card.security_requirements, credentials)
            .await
    }

    async fn authenticate_requirements(
        &self,
        requirements: &[SecurityRequirement],
        credentials: &Credentials,
    ) -> Result<Option<AuthContext>> {
        if requirements.is_empty() {
            return Ok(None);
        }
        let verifier = self.auth_verifier.as_deref().ok_or_else(|| {
            A2aError::Internal(
                "this agent declares securityRequirements but no AuthVerifier is configured \
                 (see AgentServer::with_auth_verifier)"
                    .to_string(),
            )
        })?;
        authenticate_against(requirements, verifier, credentials)
            .await
            .map(Some)
    }

    fn require_streaming(&self) -> Result<()> {
        if self.card.capabilities.streaming == Some(true) {
            Ok(())
        } else {
            Err(A2aError::UnsupportedOperation(
                "streaming is not supported by this agent".to_string(),
            ))
        }
    }

    fn require_push_notifications(&self) -> Result<()> {
        if self.card.capabilities.push_notifications == Some(true) {
            Ok(())
        } else {
            Err(A2aError::PushNotificationNotSupported)
        }
    }

    /// Rejects a message carrying a `Part.mediaType` this agent never
    /// declared support for (`ContentTypeNotSupportedError`, spec Section
    /// 3.3.2). Checked against `AgentCard.defaultInputModes` only - the
    /// protocol has no way to know which `AgentSkill` (whose `inputModes`
    /// can override the agent-wide defaults) a message is meant to invoke,
    /// so a skill-level override can't be applied here. A part with no
    /// `mediaType` set makes no claim to check. An agent that leaves
    /// `defaultInputModes` empty is treated as accepting anything, rather
    /// than rejecting everything.
    fn check_content_types(&self, message: &Message) -> Result<()> {
        if self.card.default_input_modes.is_empty() {
            return Ok(());
        }
        for part in &message.parts {
            if let Some(media_type) = &part.media_type {
                if !self.card.default_input_modes.iter().any(|m| m == media_type) {
                    return Err(A2aError::ContentTypeNotSupported(media_type.clone()));
                }
            }
        }
        Ok(())
    }

    async fn resolve_ids(
        &self,
        tenant: Option<&str>,
        message: &Message,
    ) -> Result<(String, String, Option<Task>)> {
        if let Some(task_id) = &message.task_id {
            let task = self
                .store
                .get(tenant, task_id)
                .await
                .ok_or_else(|| A2aError::TaskNotFound(task_id.clone()))?;
            if let Some(msg_ctx) = &message.context_id {
                if task.context_id.as_deref() != Some(msg_ctx.as_str()) {
                    return Err(A2aError::InvalidParams(format!(
                        "contextId {msg_ctx} does not match task {task_id}'s contextId"
                    )));
                }
            }
            let context_id = task.context_id.clone().unwrap_or_default();
            Ok((task_id.clone(), context_id, Some(task)))
        } else {
            let task_id = Uuid::new_v4().to_string();
            let context_id = message
                .context_id
                .clone()
                .unwrap_or_else(|| Uuid::new_v4().to_string());
            Ok((task_id, context_id, None))
        }
    }

    /// Kicks off `AgentExecutor::execute` in the background, pumping its
    /// events into both the [`TaskStore`] and a broadcast bus that
    /// blocking, streaming, and subscribing callers all read from.
    async fn start_execution(
        &self,
        req: &SendMessageRequest,
    ) -> Result<(Started, broadcast::Receiver<SeqEvent>)> {
        self.check_content_types(&req.message)?;
        let (task_id, context_id, existing_task) =
            self.resolve_ids(req.tenant.as_deref(), &req.message).await?;

        // Spec Sections 3.1.1/3.1.2: "Messages sent to Tasks that are in a
        // terminal state... cannot accept further messages."
        if let Some(task) = &existing_task {
            if task.status.state.is_terminal() {
                return Err(A2aError::UnsupportedOperation(format!(
                    "task {} is already in a terminal state and cannot accept further messages",
                    task.id
                )));
            }
        }

        // `SendMessageConfiguration.task_push_notification_config` (spec
        // Section 3.1.1) lets a client register push-notification
        // delivery in the same request that creates the task, since it
        // can't yet know the server-assigned task id to make a separate
        // `CreateTaskPushNotificationConfig` call - the proto's own
        // comment says the config's `taskId` "should be empty when
        // sending this configuration in a `SendMessage` request". Only
        // honored on the turn that actually creates the task: applying it
        // again on every continuation turn (e.g. answering
        // `InputRequired`) would register a fresh duplicate config each
        // time, since the client-supplied config has no server-assigned
        // `id` for `put_push_config` to dedupe against.
        if existing_task.is_none() {
            if let Some(mut config) = req
                .configuration
                .as_ref()
                .and_then(|c| c.task_push_notification_config.clone())
            {
                self.require_push_notifications()?;
                config.task_id = Some(task_id.clone());
                config.tenant = req.tenant.clone();
                self.store.put_push_config(req.tenant.as_deref(), config).await;
            }
        }

        // Snapshot of the task as it stands right before this turn's own
        // updates: the existing task's current state for a continuation,
        // or a freshly-`Submitted` task seeded with this message for a
        // brand-new one - mirrors exactly what `apply_event`'s own
        // store-side fallback constructs when no row exists yet. Used
        // below to lead the stream with a `Task` object the moment this
        // turn turns out to be task-shaped (spec Sections 3.1.2/3.1.6).
        let lead_task_snapshot = match &existing_task {
            Some(task) => task.clone(),
            None => {
                let mut task = Task::new(&task_id, &context_id, TaskState::Submitted);
                task.history.push(req.message.clone());
                task
            }
        };

        let (tx, mut rx) = mpsc::unbounded_channel::<StreamResponse>();
        let sink = EventSink::new(task_id.clone(), context_id.clone(), tx);
        let (bus, bus_rx) = broadcast::channel::<SeqEvent>(256);
        let cancellation = CancellationToken::new();

        self.buses.lock().await.insert(task_id.clone(), bus.clone());
        self.cancel_tokens
            .lock()
            .await
            .insert(task_id.clone(), cancellation.clone());

        let ctx = RequestContext {
            message: req.message.clone(),
            task: existing_task,
            task_id: task_id.clone(),
            context_id: context_id.clone(),
            cancellation,
        };

        let executor = self.executor.clone();
        let store = self.store.clone();
        let push = self.push_notifier.clone();
        let event_logs = self.event_logs.clone();
        let next_seq = self.next_seq.clone();
        let buses = self.buses.clone();
        let cancel_tokens = self.cancel_tokens.clone();
        let seed_message = req.message.clone();
        let bg_task_id = task_id.clone();
        let bg_context_id = context_id.clone();
        let bg_tenant = req.tenant.clone();

        tokio::spawn(async move {
            let exec_handle = tokio::spawn(async move { executor.execute(ctx, sink).await });

            let mut saw_closing_event = false;
            let mut first_event = true;
            while let Some(evt) = rx.recv().await {
                lead_with_task_if_needed(&next_seq, &bus, &lead_task_snapshot, first_event, &evt);
                let seq = apply_event(
                    &store,
                    &push,
                    &event_logs,
                    &next_seq,
                    bg_tenant.as_deref(),
                    &bg_task_id,
                    &bg_context_id,
                    &seed_message,
                    first_event,
                    &evt,
                )
                .await;
                first_event = false;
                let closing = evt.closes_stream();
                let _ = bus.send((seq, evt));
                if closing {
                    saw_closing_event = true;
                    break;
                }
            }

            if !saw_closing_event {
                let failure_message = match exec_handle.await {
                    Ok(Ok(())) => None,
                    Ok(Err(e)) => Some(e.to_string()),
                    Err(join_err) => Some(format!("agent executor panicked: {join_err}")),
                };
                if let Some(reason) = failure_message {
                    let failure: StreamResponse = TaskStatusUpdateEvent {
                        task_id: bg_task_id.clone(),
                        context_id: bg_context_id.clone(),
                        status: TaskStatus {
                            state: TaskState::Failed,
                            message: Some(Message::agent_text(reason)),
                            timestamp: Some(Utc::now()),
                        },
                        metadata: None,
                    }
                    .into();
                    lead_with_task_if_needed(&next_seq, &bus, &lead_task_snapshot, first_event, &failure);
                    let seq = apply_event(
                        &store,
                        &push,
                        &event_logs,
                        &next_seq,
                        bg_tenant.as_deref(),
                        &bg_task_id,
                        &bg_context_id,
                        &seed_message,
                        first_event,
                        &failure,
                    )
                    .await;
                    let _ = bus.send((seq, failure));
                } else {
                    tracing::warn!(
                        task_id = %bg_task_id,
                        "AgentExecutor::execute returned without reaching a terminal/interrupted \
                         state or replying with a message"
                    );
                }
            }

            buses.lock().await.remove(&bg_task_id);
            cancel_tokens.lock().await.remove(&bg_task_id);
        });

        Ok((
            Started {
                task_id,
                context_id,
                tenant: req.tenant.clone(),
            },
            bus_rx,
        ))
    }

    async fn snapshot_task(&self, started: &Started) -> Task {
        self.store
            .get(started.tenant.as_deref(), &started.task_id)
            .await
            .unwrap_or_else(|| Task::new(&started.task_id, &started.context_id, TaskState::Submitted))
    }

    /// `SendMessage` (spec Section 3.1.1). Blocks until the task reaches a
    /// terminal/interrupted state unless `configuration.returnImmediately`
    /// is set, in which case it returns as soon as the outcome is known to
    /// be task-shaped (see [`SendMessageConfiguration`] docs on
    /// `returnImmediately` for why a bare `Message` reply is never
    /// affected by this flag).
    ///
    /// [`SendMessageConfiguration`]: crate::types::SendMessageConfiguration
    pub async fn send_message(&self, req: SendMessageRequest) -> Result<SendMessageResult> {
        let wait_for_final = !req
            .configuration
            .as_ref()
            .map(|c| c.return_immediately)
            .unwrap_or(false);
        let history_length = req.configuration.as_ref().and_then(|c| c.history_length);
        let finish = |mut task: Task| {
            apply_history_length(&mut task, history_length);
            Ok(SendMessageResult::Task { task })
        };
        let (started, mut rx) = self.start_execution(&req).await?;
        loop {
            match rx.recv().await {
                Ok((_, StreamResponse::Message { message })) => {
                    return Ok(SendMessageResult::Message { message })
                }
                Ok((_, StreamResponse::Task { task })) => {
                    if !wait_for_final {
                        return finish(task);
                    }
                }
                Ok((_, StreamResponse::StatusUpdate { status_update })) => {
                    if status_update.status.state.is_final_for_blocking_send() {
                        // The task is done; the background pump won't write
                        // to the store again, so a fresh read is safe and
                        // gives the caller the full task (history,
                        // artifacts, ...).
                        return finish(self.snapshot_task(&started).await);
                    }
                    if !wait_for_final {
                        // Non-blocking: return *this* snapshot rather than
                        // re-reading the store, which the still-running
                        // background pump may have already advanced past
                        // (e.g. straight through to Completed).
                        let mut task =
                            Task::new(&started.task_id, &started.context_id, status_update.status.state);
                        task.status = status_update.status;
                        return finish(task);
                    }
                }
                Ok((_, StreamResponse::ArtifactUpdate { artifact_update })) => {
                    if !wait_for_final {
                        let mut task = Task::new(&started.task_id, &started.context_id, TaskState::Working);
                        task.artifacts.push(artifact_update.artifact);
                        return finish(task);
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => {
                    return match self.store.get(started.tenant.as_deref(), &started.task_id).await {
                        Some(task) => finish(task),
                        None => Err(A2aError::InvalidAgentResponse(
                            "agent finished without producing a message or task update".to_string(),
                        )),
                    };
                }
            }
        }
    }

    /// `SendStreamingMessage` (spec Section 3.1.2).
    pub async fn send_streaming_message(
        &self,
        req: SendMessageRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = StreamResponse> + Send>>> {
        self.require_streaming()?;
        let (_, rx) = self.start_execution(&req).await?;
        Ok(Box::pin(stream_through_close(rx)))
    }

    /// `SubscribeToTask` (spec Section 3.1.6): attaches to an in-flight
    /// execution's event stream, or - if the task is idle but not
    /// terminal - synthesizes a single current-snapshot event. In either
    /// case, replays whatever this task's event log still has past
    /// `since_seq` first (see the [`Engine`] docs on `event_logs`), so a
    /// caller reconnecting mid-stream (typically via SSE's `Last-Event-ID`,
    /// read by the bindings' handlers) catches up on what it missed
    /// instead of only seeing where things stand *now*. `since_seq: None`
    /// both replays everything still buffered *and* - per spec Section
    /// 3.1.6's explicit requirement that the operation "MUST return a Task
    /// object as the first event in the stream, representing the current
    /// state of the task at the time of subscription" - leads with a
    /// `Task` snapshot; a reconnect (`since_seq: Some`, this crate's own
    /// extension beyond the spec) skips that lead, since re-announcing
    /// state the caller already has would defeat the point of replaying
    /// only what was missed.
    pub async fn subscribe_to_task(
        &self,
        req: SubscribeToTaskRequest,
        since_seq: Option<u64>,
    ) -> Result<Pin<Box<dyn Stream<Item = (u64, StreamResponse)> + Send>>> {
        self.require_streaming()?;
        let task = self
            .store
            .get(req.tenant.as_deref(), &req.id)
            .await
            .ok_or_else(|| A2aError::TaskNotFound(req.id.clone()))?;
        if task.status.state.is_terminal() {
            return Err(A2aError::UnsupportedOperation(format!(
                "task {} is already in a terminal state",
                req.id
            )));
        }

        // Subscribe *before* reading the log snapshot: any event applied
        // after this point is guaranteed to arrive live, so pairing it
        // with a snapshot taken afterward (and deduping by seq) can never
        // leave a gap, regardless of how the two race.
        let bus_rx = self.buses.lock().await.get(&req.id).map(|b| b.subscribe());
        let replay: Vec<SeqEvent> = {
            let logs = self.event_logs.lock().await;
            logs.get(&req.id)
                .map(|log| {
                    log.iter()
                        .filter(|(seq, _)| since_seq.is_none_or(|since| *seq > since))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default()
        };
        let max_replayed = replay.last().map(|(seq, _)| *seq);
        let lead: Option<SeqEvent> = if since_seq.is_none() {
            let seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
            Some((seq, StreamResponse::Task { task: task.clone() }))
        } else {
            None
        };

        match bus_rx {
            Some(rx) => Ok(Box::pin(replay_then_live(lead, replay, rx, max_replayed))),
            None if lead.is_some() => {
                // Fresh subscribe to an idle task: the lead snapshot above
                // already satisfies "current state at time of
                // subscription" on its own - nothing else to add.
                Ok(Box::pin(async_stream::stream! {
                    yield lead.expect("lead is Some in this branch");
                }))
            }
            None => {
                // Idle but not terminal (e.g. InputRequired/AuthRequired),
                // reconnecting via `since_seq`: no live tail to attach to,
                // so replay whatever's buffered, then a final
                // current-state snapshot.
                let snapshot_seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
                Ok(Box::pin(async_stream::stream! {
                    for item in replay {
                        yield item;
                    }
                    yield (snapshot_seq, StreamResponse::Task { task });
                }))
            }
        }
    }

    /// `GetTask` (spec Section 3.1.3).
    pub async fn get_task(&self, req: GetTaskRequest) -> Result<Task> {
        let mut task = self
            .store
            .get(req.tenant.as_deref(), &req.id)
            .await
            .ok_or_else(|| A2aError::TaskNotFound(req.id.clone()))?;
        apply_history_length(&mut task, req.history_length);
        Ok(task)
    }

    /// `ListTasks` (spec Section 3.1.4).
    pub async fn list_tasks(&self, req: ListTasksRequest) -> Result<ListTasksResponse> {
        let (mut tasks, next_page_token, total) = self.store.list(req.tenant.as_deref(), &req).await;
        for t in &mut tasks {
            apply_history_length(t, req.history_length);
        }
        let page_size = req.page_size.unwrap_or(50).clamp(1, 100);
        Ok(ListTasksResponse {
            tasks,
            next_page_token,
            page_size,
            total_size: total as i32,
        })
    }

    /// `CancelTask` (spec Section 3.1.5).
    pub async fn cancel_task(&self, req: CancelTaskRequest) -> Result<Task> {
        let task = self
            .store
            .get(req.tenant.as_deref(), &req.id)
            .await
            .ok_or_else(|| A2aError::TaskNotFound(req.id.clone()))?;
        if task.status.state.is_terminal() {
            return Err(A2aError::TaskNotCancelable(req.id.clone()));
        }
        let context_id = task.context_id.clone().unwrap_or_default();

        if let Some(token) = self.cancel_tokens.lock().await.get(&req.id).cloned() {
            token.cancel();
        }

        let (tx, mut rx) = mpsc::unbounded_channel::<StreamResponse>();
        let sink = EventSink::new(req.id.clone(), context_id.clone(), tx);
        let seed = task
            .history
            .last()
            .cloned()
            .unwrap_or_else(|| Message::new(Role::User, Vec::new()));
        let ctx = RequestContext {
            message: seed.clone(),
            task: Some(task.clone()),
            task_id: req.id.clone(),
            context_id: context_id.clone(),
            cancellation: CancellationToken::new(),
        };

        self.executor.cancel(ctx, sink).await?;

        let bus = self.buses.lock().await.get(&req.id).cloned();
        while let Ok(evt) = rx.try_recv() {
            let seq = apply_event(
                &self.store,
                &self.push_notifier,
                &self.event_logs,
                &self.next_seq,
                req.tenant.as_deref(),
                &req.id,
                &context_id,
                &seed,
                false,
                &evt,
            )
            .await;
            if let Some(b) = &bus {
                let _ = b.send((seq, evt));
            }
        }

        // Guarantee a canceled/terminal outcome even if the executor's
        // `cancel` override didn't explicitly emit one.
        let mut updated = self
            .store
            .get(req.tenant.as_deref(), &req.id)
            .await
            .unwrap_or(task);
        if !updated.status.state.is_terminal() {
            updated.status = TaskStatus::new(TaskState::Canceled);
            let evt: StreamResponse = TaskStatusUpdateEvent {
                task_id: req.id.clone(),
                context_id: context_id.clone(),
                status: updated.status.clone(),
                metadata: None,
            }
            .into();
            let seq = apply_event(
                &self.store,
                &self.push_notifier,
                &self.event_logs,
                &self.next_seq,
                req.tenant.as_deref(),
                &req.id,
                &context_id,
                &seed,
                false,
                &evt,
            )
            .await;
            if let Some(b) = &bus {
                let _ = b.send((seq, evt));
            }
            updated = self
                .store
                .get(req.tenant.as_deref(), &req.id)
                .await
                .unwrap_or(updated);
        }
        Ok(updated)
    }

    /// `CreateTaskPushNotificationConfig` (spec Section 3.1.7).
    pub async fn create_push_notification_config(
        &self,
        config: TaskPushNotificationConfig,
    ) -> Result<TaskPushNotificationConfig> {
        self.require_push_notifications()?;
        let task_id = config
            .task_id
            .clone()
            .ok_or_else(|| A2aError::InvalidParams("taskId is required".to_string()))?;
        let tenant = config.tenant.clone();
        self.store
            .get(tenant.as_deref(), &task_id)
            .await
            .ok_or_else(|| A2aError::TaskNotFound(task_id.clone()))?;
        Ok(self.store.put_push_config(tenant.as_deref(), config).await)
    }

    /// `GetTaskPushNotificationConfig` (spec Section 3.1.8).
    pub async fn get_push_notification_config(
        &self,
        req: GetTaskPushNotificationConfigRequest,
    ) -> Result<TaskPushNotificationConfig> {
        self.require_push_notifications()?;
        self.store
            .get_push_config(req.tenant.as_deref(), &req.task_id, &req.id)
            .await
            .ok_or_else(|| A2aError::TaskNotFound(format!("push notification config {}", req.id)))
    }

    /// `ListTaskPushNotificationConfigs` (spec Section 3.1.9). Paginates
    /// in memory over [`TaskStore::list_push_configs`]'s full result
    /// (rather than pushing pagination down into the store, the way
    /// [`Engine::list_tasks`] does via [`TaskStore::list`]) since
    /// `list_push_configs` already exists as an unpaginated "every config
    /// for this task" query for [`notify_push_configs`]'s webhook fan-out,
    /// and the number of configs on one task is expected to be small.
    pub async fn list_push_notification_configs(
        &self,
        req: ListTaskPushNotificationConfigsRequest,
    ) -> Result<ListTaskPushNotificationConfigsResponse> {
        self.require_push_notifications()?;
        let mut configs = self
            .store
            .list_push_configs(req.tenant.as_deref(), &req.task_id)
            .await;
        configs.sort_by(|a, b| a.id.cmp(&b.id));

        let page_size = req.page_size.unwrap_or(50).clamp(1, 100) as usize;
        let start = req
            .page_token
            .as_ref()
            .and_then(|token| {
                configs
                    .iter()
                    .position(|c| c.id.as_deref() == Some(token.as_str()))
            })
            .unwrap_or(0);
        let end = (start + page_size).min(configs.len());
        let next_page_token = if end < configs.len() {
            configs[end].id.clone().unwrap_or_default()
        } else {
            String::new()
        };

        Ok(ListTaskPushNotificationConfigsResponse {
            configs: configs[start..end].to_vec(),
            next_page_token,
        })
    }

    /// `DeleteTaskPushNotificationConfig` (spec Section 3.1.10).
    pub async fn delete_push_notification_config(
        &self,
        req: DeleteTaskPushNotificationConfigRequest,
    ) -> Result<()> {
        self.require_push_notifications()?;
        if self
            .store
            .delete_push_config(req.tenant.as_deref(), &req.task_id, &req.id)
            .await
        {
            Ok(())
        } else {
            Err(A2aError::TaskNotFound(format!(
                "push notification config {}",
                req.id
            )))
        }
    }

    /// `GetExtendedAgentCard` (spec Section 3.1.11): unlike every other
    /// operation, this one is authenticated even when
    /// `AgentCard.securityRequirements` is empty, since the spec
    /// describes it as being "for the authenticated agent" - if an
    /// [`AuthVerifier`] is configured, this uses `securityRequirements`
    /// when declared, or else falls back to treating each entry in
    /// `securitySchemes` as its own single-scheme alternative (any one
    /// successfully verified scheme is sufficient). With no verifier
    /// configured, or no schemes declared at all to fall back to, this
    /// method can't enforce anything and just checks the capability flag,
    /// same as before this crate had any auth enforcement.
    pub async fn get_extended_agent_card(&self, credentials: &Credentials) -> Result<AgentCard> {
        match self.card.capabilities.extended_agent_card {
            Some(true) => {}
            _ => {
                return Err(A2aError::UnsupportedOperation(
                    "extended agent card is not supported by this agent".to_string(),
                ))
            }
        }
        if let Some(verifier) = self.auth_verifier.as_deref() {
            let requirements = self.extended_card_security_requirements();
            if !requirements.is_empty() {
                authenticate_against(&requirements, verifier, credentials).await?;
            }
        }
        self.extended_card
            .clone()
            .ok_or(A2aError::ExtendedAgentCardNotConfigured)
    }

    /// `AgentCard.securityRequirements` if declared, else one
    /// single-scheme alternative per entry in `securitySchemes` - see
    /// [`Engine::get_extended_agent_card`].
    fn extended_card_security_requirements(&self) -> Vec<SecurityRequirement> {
        if !self.card.security_requirements.is_empty() {
            return self.card.security_requirements.clone();
        }
        self.card
            .security_schemes
            .keys()
            .map(|name| SecurityRequirement {
                schemes: HashMap::from([(name.clone(), crate::types::StringList { list: Vec::new() })]),
            })
            .collect()
    }
}

/// Parses the `A2A-Extensions` service parameter (spec Section 3.2.6): a
/// comma-separated list of extension URIs the caller declares support
/// for. `None`/empty input yields an empty set.
pub(crate) fn parse_extensions_header(value: Option<&str>) -> HashSet<String> {
    value
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn apply_history_length(task: &mut Task, history_length: Option<i32>) {
    if let Some(n) = history_length {
        if n <= 0 {
            task.history.clear();
        } else {
            let n = n as usize;
            if task.history.len() > n {
                let start = task.history.len() - n;
                task.history.drain(0..start);
            }
        }
    }
}

/// If this is the first event of the turn and it's task-shaping (anything
/// but a bare `Message`), sends `lead_task` as a `StreamResponse::Task`
/// ahead of it on the bus - spec Sections 3.1.2/3.1.6 require a
/// task-shaped `SendStreamingMessage` stream, and every `SubscribeToTask`
/// stream, to begin with the `Task` object itself. Sent directly to the
/// bus without going through the shared `event_logs`/`apply_event`
/// machinery: it carries no new information (it's a snapshot of state
/// [`Engine::subscribe_to_task`] can just as easily reconstruct itself
/// from the store for its own leading `Task`), so logging it would only
/// risk a duplicate for a subscriber that attaches mid-turn.
fn lead_with_task_if_needed(
    next_seq: &Arc<AtomicU64>,
    bus: &broadcast::Sender<SeqEvent>,
    lead_task: &Task,
    first_event: bool,
    evt: &StreamResponse,
) {
    if first_event && !matches!(evt, StreamResponse::Message { .. }) {
        let seq = next_seq.fetch_add(1, Ordering::Relaxed);
        let _ = bus.send((
            seq,
            StreamResponse::Task {
                task: lead_task.clone(),
            },
        ));
    }
}

/// Applies `evt` to the store (as before), fires push notifications, and
/// appends it to `task_id`'s bounded event log under a freshly assigned,
/// engine-wide monotonic sequence number - which it returns, so the
/// caller can pair the exact same number onto the broadcast bus send
/// (see [`Engine::subscribe_to_task`] on why that pairing has to be
/// exact for replay to be race-free).
/// `is_first_event_of_turn` marks the first event of one `start_execution`
/// call (a "turn"): a brand-new task is always seeded with `seed_message`
/// on creation, but a *continuation* turn (the client resumed an existing
/// task by sending a new message with `task_id` set - e.g. answering
/// `InputRequired`) finds the task already in the store from its earlier
/// turn(s), so without this flag `seed_message` - this turn's actual
/// inbound message - would never be recorded into `task.history` at all.
/// `cancel_task`'s synthetic events aren't a new turn, so it always passes
/// `false`.
#[allow(clippy::too_many_arguments)]
async fn apply_event(
    store: &Arc<dyn TaskStore>,
    push: &PushNotifier,
    event_logs: &EventLogs,
    next_seq: &Arc<AtomicU64>,
    tenant: Option<&str>,
    task_id: &str,
    context_id: &str,
    seed_message: &Message,
    is_first_event_of_turn: bool,
    evt: &StreamResponse,
) -> u64 {
    let seq = next_seq.fetch_add(1, Ordering::Relaxed);
    {
        let mut logs = event_logs.lock().await;
        let log = logs.entry(task_id.to_string()).or_default();
        log.push_back((seq, evt.clone()));
        if log.len() > EVENT_LOG_CAPACITY {
            log.pop_front();
        }
    }

    match evt {
        StreamResponse::StatusUpdate { status_update } => {
            let mut task = match store.get(tenant, task_id).await {
                Some(mut t) => {
                    if is_first_event_of_turn {
                        t.history.push(seed_message.clone());
                    }
                    t
                }
                None => {
                    let mut t = Task::new(task_id, context_id, TaskState::Submitted);
                    t.history.push(seed_message.clone());
                    t
                }
            };
            task.status = status_update.status.clone();
            if let Some(msg) = &status_update.status.message {
                task.history.push(msg.clone());
            }
            store.put(tenant, task.clone()).await;
            notify_push_configs(store, push, tenant, &task).await;
        }
        StreamResponse::ArtifactUpdate { artifact_update } => {
            let mut task = match store.get(tenant, task_id).await {
                Some(mut t) => {
                    if is_first_event_of_turn {
                        t.history.push(seed_message.clone());
                    }
                    t
                }
                None => {
                    let mut t = Task::new(task_id, context_id, TaskState::Working);
                    t.history.push(seed_message.clone());
                    t
                }
            };
            merge_artifact(&mut task, artifact_update);
            store.put(tenant, task.clone()).await;
            notify_push_configs(store, push, tenant, &task).await;
        }
        StreamResponse::Task { .. } | StreamResponse::Message { .. } => {}
    }

    seq
}

/// Fires off push notification delivery (spec Section 4.3) to every
/// config registered for `task.id`, one background task per config so a
/// slow/unreachable webhook can never delay task processing.
async fn notify_push_configs(
    store: &Arc<dyn TaskStore>,
    push: &PushNotifier,
    tenant: Option<&str>,
    task: &Task,
) {
    for config in store.list_push_configs(tenant, &task.id).await {
        let push = push.clone();
        let task = task.clone();
        tokio::spawn(async move { push.notify(&config, &task).await });
    }
}

fn merge_artifact(task: &mut Task, update: &TaskArtifactUpdateEvent) {
    if update.append {
        if let Some(existing) = task
            .artifacts
            .iter_mut()
            .find(|a| a.artifact_id == update.artifact.artifact_id)
        {
            existing.parts.extend(update.artifact.parts.clone());
            return;
        }
    } else {
        task.artifacts
            .retain(|a| a.artifact_id != update.artifact.artifact_id);
    }
    task.artifacts.push(update.artifact.clone());
}

/// Wraps a broadcast receiver as a `Stream`, yielding events (skipping
/// over any missed due to lag) until - and including - the first event
/// that closes the stream (spec Section 11.7: a terminal/interrupted
/// status, or a bare message).
fn stream_through_close(mut rx: broadcast::Receiver<SeqEvent>) -> impl Stream<Item = StreamResponse> {
    stream! {
        loop {
            match rx.recv().await {
                Ok((_, evt)) => {
                    let closing = evt.closes_stream();
                    yield evt;
                    if closing {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }
}

/// Yields `lead` (a fresh subscriber's required leading `Task` snapshot,
/// see [`Engine::subscribe_to_task`]), then `replay` (already-buffered
/// events a reconnecting subscriber missed), then the live tail of `rx`,
/// until - and including - the first event that closes the stream. Skips
/// any live event whose sequence number is `<= max_replayed`: since
/// [`Engine::subscribe_to_task`] subscribes to the bus *before* taking the
/// `replay` snapshot, every event either lands in `replay` or arrives
/// live (never both missed, but sometimes both) - this filter is what
/// makes "both" safe instead of a duplicate.
fn replay_then_live(
    lead: Option<SeqEvent>,
    replay: Vec<SeqEvent>,
    mut rx: broadcast::Receiver<SeqEvent>,
    max_replayed: Option<u64>,
) -> impl Stream<Item = (u64, StreamResponse)> {
    stream! {
        if let Some((seq, evt)) = lead {
            let closing = evt.closes_stream();
            yield (seq, evt);
            if closing {
                return;
            }
        }
        for (seq, evt) in &replay {
            let closing = evt.closes_stream();
            yield (*seq, evt.clone());
            if closing {
                return;
            }
        }
        loop {
            match rx.recv().await {
                Ok((seq, evt)) => {
                    if max_replayed.is_some_and(|max| seq <= max) {
                        continue;
                    }
                    let closing = evt.closes_stream();
                    yield (seq, evt);
                    if closing {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }
}
