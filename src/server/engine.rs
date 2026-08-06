//! The transport-agnostic core of an A2A server: implements all eleven
//! A2A operations (spec Section 3.1) against an [`AgentExecutor`] and a
//! [`TaskStore`], independent of the JSON-RPC framing used to expose them.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use async_stream::stream;
use chrono::Utc;
use futures_core::Stream;
use tokio::sync::{broadcast, mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

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
}

pub struct Engine {
    card: AgentCard,
    extended_card: Option<AgentCard>,
    executor: Arc<dyn AgentExecutor>,
    store: Arc<dyn TaskStore>,
    buses: Arc<Mutex<HashMap<String, broadcast::Sender<StreamResponse>>>>,
    cancel_tokens: Arc<Mutex<HashMap<String, CancellationToken>>>,
    auth_verifier: Option<Arc<dyn AuthVerifier>>,
    push_notifier: PushNotifier,
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
            push_notifier: PushNotifier::new(),
        }
    }

    pub(crate) fn set_extended_card(&mut self, card: AgentCard) {
        self.extended_card = Some(card);
    }

    pub(crate) fn set_auth_verifier(&mut self, verifier: Arc<dyn AuthVerifier>) {
        self.auth_verifier = Some(verifier);
    }

    pub fn card(&self) -> &AgentCard {
        &self.card
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

    async fn resolve_ids(&self, message: &Message) -> Result<(String, String, Option<Task>)> {
        if let Some(task_id) = &message.task_id {
            let task = self
                .store
                .get(task_id)
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
    ) -> Result<(Started, broadcast::Receiver<StreamResponse>)> {
        let (task_id, context_id, existing_task) = self.resolve_ids(&req.message).await?;

        let (tx, mut rx) = mpsc::unbounded_channel::<StreamResponse>();
        let sink = EventSink::new(task_id.clone(), context_id.clone(), tx);
        let (bus, bus_rx) = broadcast::channel::<StreamResponse>(256);
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
        let buses = self.buses.clone();
        let cancel_tokens = self.cancel_tokens.clone();
        let seed_message = req.message.clone();
        let bg_task_id = task_id.clone();
        let bg_context_id = context_id.clone();

        tokio::spawn(async move {
            let exec_handle = tokio::spawn(async move { executor.execute(ctx, sink).await });

            let mut saw_closing_event = false;
            while let Some(evt) = rx.recv().await {
                apply_event(&store, &push, &bg_task_id, &bg_context_id, &seed_message, &evt).await;
                let closing = evt.closes_stream();
                let _ = bus.send(evt);
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
                    apply_event(
                        &store,
                        &push,
                        &bg_task_id,
                        &bg_context_id,
                        &seed_message,
                        &failure,
                    )
                    .await;
                    let _ = bus.send(failure);
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

        Ok((Started { task_id, context_id }, bus_rx))
    }

    async fn snapshot_task(&self, started: &Started) -> Task {
        self.store
            .get(&started.task_id)
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
        let (started, mut rx) = self.start_execution(&req).await?;
        loop {
            match rx.recv().await {
                Ok(StreamResponse::Message { message }) => return Ok(SendMessageResult::Message { message }),
                Ok(StreamResponse::Task { task }) => {
                    if !wait_for_final {
                        return Ok(SendMessageResult::Task { task });
                    }
                }
                Ok(StreamResponse::StatusUpdate { status_update }) => {
                    if status_update.status.state.is_final_for_blocking_send() {
                        // The task is done; the background pump won't write
                        // to the store again, so a fresh read is safe and
                        // gives the caller the full task (history,
                        // artifacts, ...).
                        return Ok(SendMessageResult::Task {
                            task: self.snapshot_task(&started).await,
                        });
                    }
                    if !wait_for_final {
                        // Non-blocking: return *this* snapshot rather than
                        // re-reading the store, which the still-running
                        // background pump may have already advanced past
                        // (e.g. straight through to Completed).
                        let mut task =
                            Task::new(&started.task_id, &started.context_id, status_update.status.state);
                        task.status = status_update.status;
                        return Ok(SendMessageResult::Task { task });
                    }
                }
                Ok(StreamResponse::ArtifactUpdate { artifact_update }) => {
                    if !wait_for_final {
                        let mut task = Task::new(&started.task_id, &started.context_id, TaskState::Working);
                        task.artifacts.push(artifact_update.artifact);
                        return Ok(SendMessageResult::Task { task });
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => {
                    return match self.store.get(&started.task_id).await {
                        Some(task) => Ok(SendMessageResult::Task { task }),
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
    /// terminal - synthesizes a single current-snapshot event.
    pub async fn subscribe_to_task(
        &self,
        req: SubscribeToTaskRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = StreamResponse> + Send>>> {
        self.require_streaming()?;
        let task = self
            .store
            .get(&req.id)
            .await
            .ok_or_else(|| A2aError::TaskNotFound(req.id.clone()))?;
        if task.status.state.is_terminal() {
            return Err(A2aError::UnsupportedOperation(format!(
                "task {} is already in a terminal state",
                req.id
            )));
        }
        let bus_rx = self.buses.lock().await.get(&req.id).map(|b| b.subscribe());
        match bus_rx {
            Some(rx) => Ok(Box::pin(stream_through_close(rx))),
            None => Ok(Box::pin(
                async_stream::stream! { yield StreamResponse::Task { task }; },
            )),
        }
    }

    /// `GetTask` (spec Section 3.1.3).
    pub async fn get_task(&self, req: GetTaskRequest) -> Result<Task> {
        let mut task = self
            .store
            .get(&req.id)
            .await
            .ok_or_else(|| A2aError::TaskNotFound(req.id.clone()))?;
        apply_history_length(&mut task, req.history_length);
        Ok(task)
    }

    /// `ListTasks` (spec Section 3.1.4).
    pub async fn list_tasks(&self, req: ListTasksRequest) -> Result<ListTasksResponse> {
        let (mut tasks, next_page_token, total) = self.store.list(&req).await;
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
            .get(&req.id)
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
            apply_event(
                &self.store,
                &self.push_notifier,
                &req.id,
                &context_id,
                &seed,
                &evt,
            )
            .await;
            if let Some(b) = &bus {
                let _ = b.send(evt);
            }
        }

        // Guarantee a canceled/terminal outcome even if the executor's
        // `cancel` override didn't explicitly emit one.
        let mut updated = self.store.get(&req.id).await.unwrap_or(task);
        if !updated.status.state.is_terminal() {
            updated.status = TaskStatus::new(TaskState::Canceled);
            self.store.put(updated.clone()).await;
            notify_push_configs(&self.store, &self.push_notifier, &updated).await;
            if let Some(b) = &bus {
                let _ = b.send(
                    TaskStatusUpdateEvent {
                        task_id: req.id.clone(),
                        context_id: context_id.clone(),
                        status: updated.status.clone(),
                        metadata: None,
                    }
                    .into(),
                );
            }
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
        self.store
            .get(&task_id)
            .await
            .ok_or_else(|| A2aError::TaskNotFound(task_id.clone()))?;
        Ok(self.store.put_push_config(config).await)
    }

    /// `GetTaskPushNotificationConfig` (spec Section 3.1.8).
    pub async fn get_push_notification_config(
        &self,
        req: GetTaskPushNotificationConfigRequest,
    ) -> Result<TaskPushNotificationConfig> {
        self.require_push_notifications()?;
        self.store
            .get_push_config(&req.task_id, &req.id)
            .await
            .ok_or_else(|| A2aError::TaskNotFound(format!("push notification config {}", req.id)))
    }

    /// `ListTaskPushNotificationConfigs` (spec Section 3.1.9).
    pub async fn list_push_notification_configs(
        &self,
        req: ListTaskPushNotificationConfigsRequest,
    ) -> Result<ListTaskPushNotificationConfigsResponse> {
        self.require_push_notifications()?;
        let configs = self.store.list_push_configs(&req.task_id).await;
        Ok(ListTaskPushNotificationConfigsResponse {
            configs,
            next_page_token: String::new(),
        })
    }

    /// `DeleteTaskPushNotificationConfig` (spec Section 3.1.10).
    pub async fn delete_push_notification_config(
        &self,
        req: DeleteTaskPushNotificationConfigRequest,
    ) -> Result<()> {
        self.require_push_notifications()?;
        if self.store.delete_push_config(&req.task_id, &req.id).await {
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

async fn apply_event(
    store: &Arc<dyn TaskStore>,
    push: &PushNotifier,
    task_id: &str,
    context_id: &str,
    seed_message: &Message,
    evt: &StreamResponse,
) {
    match evt {
        StreamResponse::StatusUpdate { status_update } => {
            let mut task = match store.get(task_id).await {
                Some(t) => t,
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
            store.put(task.clone()).await;
            notify_push_configs(store, push, &task).await;
        }
        StreamResponse::ArtifactUpdate { artifact_update } => {
            let mut task = match store.get(task_id).await {
                Some(t) => t,
                None => {
                    let mut t = Task::new(task_id, context_id, TaskState::Working);
                    t.history.push(seed_message.clone());
                    t
                }
            };
            merge_artifact(&mut task, artifact_update);
            store.put(task.clone()).await;
            notify_push_configs(store, push, &task).await;
        }
        StreamResponse::Task { .. } | StreamResponse::Message { .. } => {}
    }
}

/// Fires off push notification delivery (spec Section 4.3) to every
/// config registered for `task.id`, one background task per config so a
/// slow/unreachable webhook can never delay task processing.
async fn notify_push_configs(store: &Arc<dyn TaskStore>, push: &PushNotifier, task: &Task) {
    for config in store.list_push_configs(&task.id).await {
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
fn stream_through_close(mut rx: broadcast::Receiver<StreamResponse>) -> impl Stream<Item = StreamResponse> {
    stream! {
        loop {
            match rx.recv().await {
                Ok(evt) => {
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
