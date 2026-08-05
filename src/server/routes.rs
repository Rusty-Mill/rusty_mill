//! The axum handlers implementing the ACP HTTP surface.

use std::{convert::Infallible, sync::Arc};

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        sse::{Event as SseEvent, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{get, post},
    Json, Router,
};
use futures_util::stream::{self, Stream, StreamExt};
use serde::Deserialize;
use tokio::sync::{broadcast::error::RecvError, watch};

use crate::{
    server::{store::RunHandle, AcpServer},
    types::{
        AgentManifest, AgentName, AgentsListResponse, Error, ErrorCode, Event, Message, Run,
        RunCreateRequest, RunEventsListResponse, RunId, RunMode, RunResumeRequest, RunStatus,
        Session, SessionId,
    },
};

/// An [`Error`] rendered as an HTTP response with the conventional status code.
pub(crate) struct ApiError(pub Error);

impl From<Error> for ApiError {
    fn from(value: Error) -> Self {
        ApiError(value)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.0.code.http_status())
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (status, Json(self.0)).into_response()
    }
}

type ApiResult<T> = Result<T, ApiError>;

/// Build the router serving every ACP endpoint.
pub(crate) fn router(server: Arc<AcpServer>) -> Router {
    Router::new()
        .route("/ping", get(ping))
        .route("/agents", get(list_agents))
        .route("/agents/{name}", get(get_agent))
        .route("/runs", post(create_run))
        .route("/runs/{run_id}", get(get_run).post(resume_run))
        .route("/runs/{run_id}/cancel", post(cancel_run))
        .route("/runs/{run_id}/events", get(list_run_events))
        .route("/session/{session_id}", get(get_session))
        .route("/session/{session_id}/messages/{index}", get(get_session_message))
        .with_state(server)
}

async fn ping() -> Json<serde_json::Value> {
    Json(serde_json::json!({}))
}

/// Pagination for `GET /agents`.
#[derive(Debug, Deserialize)]
struct ListAgentsQuery {
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    offset: usize,
}

fn default_limit() -> usize {
    10
}

async fn list_agents(
    State(server): State<Arc<AcpServer>>,
    Query(query): Query<ListAgentsQuery>,
) -> ApiResult<Json<AgentsListResponse>> {
    if query.limit == 0 || query.limit > 1000 {
        return Err(Error::invalid_input("`limit` must be between 1 and 1000").into());
    }
    let agents =
        server.manifests().into_iter().skip(query.offset).take(query.limit).collect::<Vec<_>>();
    Ok(Json(AgentsListResponse { agents }))
}

async fn get_agent(
    State(server): State<Arc<AcpServer>>,
    Path(name): Path<String>,
) -> ApiResult<Json<AgentManifest>> {
    let name = AgentName::new(name).map_err(ApiError::from)?;
    let manifest = server
        .manifest(&name)
        .ok_or_else(|| Error::not_found(format!("agent {name} not found")))?;
    Ok(Json(manifest))
}

/// Either a JSON body or an SSE stream, depending on the requested [`RunMode`].
enum RunResponse {
    Json(StatusCode, Box<Run>),
    Stream(Response),
}

impl IntoResponse for RunResponse {
    fn into_response(self) -> Response {
        match self {
            RunResponse::Json(status, run) => (status, Json(*run)).into_response(),
            RunResponse::Stream(response) => response,
        }
    }
}

type EventStream = std::pin::Pin<Box<dyn Stream<Item = Result<SseEvent, Infallible>> + Send>>;

async fn create_run(
    State(server): State<Arc<AcpServer>>,
    headers: HeaderMap,
    Json(request): Json<RunCreateRequest>,
) -> ApiResult<RunResponse> {
    request.validate().map_err(ApiError::from)?;
    let base_url = server.resolve_base_url(&headers);
    let mode = request.mode();

    let (handle, ready) = server.start_run(request, &base_url).await.map_err(ApiError::from)?;
    Ok(deliver(handle, ready, mode, StatusCode::ACCEPTED).await)
}

async fn resume_run(
    State(server): State<Arc<AcpServer>>,
    Path(run_id): Path<String>,
    Json(request): Json<RunResumeRequest>,
) -> ApiResult<RunResponse> {
    let run_id: RunId = run_id.parse().map_err(ApiError::from)?;
    if request.run_id != run_id {
        return Err(Error::invalid_input(
            "`run_id` in the request body does not match the run id in the path",
        )
        .into());
    }

    let handle = server.store().require_run(run_id).map_err(ApiError::from)?;
    if handle.status() != RunStatus::Awaiting {
        return Err(Error::invalid_input(format!(
            "run {run_id} is `{}`; only an `awaiting` run can be resumed",
            handle.status()
        ))
        .into());
    }

    // Subscribe before delivering the payload so no event or status change is
    // missed between resuming and observing the outcome.
    let events = handle.subscribe();
    let mut status = handle.watch_status();
    status.borrow_and_update();

    handle.send_resume(request.await_resume).await.map_err(ApiError::from)?;

    let ready = Ready { events, status };
    Ok(deliver(handle, ready, request.mode, StatusCode::ACCEPTED).await)
}

/// Subscriptions taken before a run starts or resumes, so the response can
/// observe everything that follows.
pub(crate) struct Ready {
    pub events: tokio::sync::broadcast::Receiver<Event>,
    pub status: watch::Receiver<RunStatus>,
}

/// Turn a started run into the response the requested mode calls for.
async fn deliver(
    handle: Arc<RunHandle>,
    ready: Ready,
    mode: RunMode,
    async_status: StatusCode,
) -> RunResponse {
    match mode {
        RunMode::Async => RunResponse::Json(async_status, Box::new(handle.snapshot())),
        RunMode::Sync => {
            wait_until_settled(ready.status).await;
            RunResponse::Json(StatusCode::OK, Box::new(handle.snapshot()))
        }
        RunMode::Stream => RunResponse::Stream(
            Sse::new(event_stream(ready.events)).keep_alive(KeepAlive::default()).into_response(),
        ),
    }
}

/// Wait until the run reaches a terminal state or pauses awaiting input.
///
/// The receiver must already have its current value marked as seen.
async fn wait_until_settled(mut status: watch::Receiver<RunStatus>) {
    loop {
        if status.changed().await.is_err() {
            return;
        }
        let current = *status.borrow_and_update();
        if current.is_terminal() || current.is_awaiting() {
            return;
        }
    }
}

/// Adapt the run's event broadcast into an SSE stream that ends after the
/// terminal event.
fn event_stream(events: tokio::sync::broadcast::Receiver<Event>) -> EventStream {
    let stream = stream::unfold((events, false), |(mut events, done)| async move {
        if done {
            return None;
        }
        match events.recv().await {
            Ok(event) => {
                let done = event.is_terminal();
                Some((sse_event(&event), (events, done)))
            }
            Err(RecvError::Closed) => None,
            // Report the gap and keep streaming; the full log stays available
            // from `GET /runs/{run_id}/events`.
            Err(RecvError::Lagged(skipped)) => {
                let event = Event::Error {
                    error: Error::server_error(format!(
                        "event stream lagged; {skipped} events were dropped. \
                         Fetch GET /runs/{{run_id}}/events for the full log."
                    )),
                };
                Some((sse_event(&event), (events, false)))
            }
        }
    });
    Box::pin(stream.map(Ok))
}

fn sse_event(event: &Event) -> SseEvent {
    match SseEvent::default().event(event.event_type()).json_data(event) {
        Ok(sse) => sse,
        Err(err) => SseEvent::default().event("error").data(
            serde_json::to_string(&Event::Error {
                error: Error::server_error(format!("failed to serialize event: {err}")),
            })
            .unwrap_or_else(|_| {
                r#"{"type":"error","error":{"code":"server_error","message":"failed to serialize event"}}"#
                    .to_string()
            }),
        ),
    }
}

async fn get_run(
    State(server): State<Arc<AcpServer>>,
    Path(run_id): Path<String>,
) -> ApiResult<Json<Run>> {
    let run_id: RunId = run_id.parse().map_err(ApiError::from)?;
    let handle = server.store().require_run(run_id).map_err(ApiError::from)?;
    Ok(Json(handle.snapshot()))
}

async fn cancel_run(
    State(server): State<Arc<AcpServer>>,
    Path(run_id): Path<String>,
) -> ApiResult<(StatusCode, Json<Run>)> {
    let run_id: RunId = run_id.parse().map_err(ApiError::from)?;
    let handle = server.store().require_run(run_id).map_err(ApiError::from)?;
    handle.request_cancel();
    Ok((StatusCode::ACCEPTED, Json(handle.snapshot())))
}

async fn list_run_events(
    State(server): State<Arc<AcpServer>>,
    Path(run_id): Path<String>,
) -> ApiResult<Json<RunEventsListResponse>> {
    let run_id: RunId = run_id.parse().map_err(ApiError::from)?;
    let handle = server.store().require_run(run_id).map_err(ApiError::from)?;
    Ok(Json(RunEventsListResponse { events: handle.events() }))
}

async fn get_session(
    State(server): State<Arc<AcpServer>>,
    Path(session_id): Path<String>,
) -> ApiResult<Json<Session>> {
    let session_id: SessionId = session_id.parse().map_err(ApiError::from)?;
    let record = server.store().require_session(session_id).map_err(ApiError::from)?;
    Ok(Json(record.session))
}

/// Resource endpoint backing the message URLs in a [`Session`]'s history.
async fn get_session_message(
    State(server): State<Arc<AcpServer>>,
    Path((session_id, index)): Path<(String, usize)>,
) -> ApiResult<Json<Message>> {
    let session_id: SessionId = session_id.parse().map_err(ApiError::from)?;
    let record = server.store().require_session(session_id).map_err(ApiError::from)?;
    let message = record.messages.get(index).cloned().ok_or_else(|| {
        Error::new(
            ErrorCode::NotFound,
            format!("session {session_id} has no message at index {index}"),
        )
    })?;
    Ok(Json(message))
}
