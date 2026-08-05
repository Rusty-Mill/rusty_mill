//! The axum handlers implementing the ACP HTTP surface.
//!
//! Every handler reaches for the [`Store`](crate::server::store::Store) rather
//! than any process-local map, so a request for a run can be served by any
//! replica — including one that is not executing it.

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
use futures_util::stream::{Stream, StreamExt};
use serde::Deserialize;

use crate::{
    server::{
        store::{Notification, NotificationStream},
        AcpServer,
    },
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
    let router = Router::new()
        .route("/ping", get(ping))
        .route("/agents", get(list_agents))
        .route("/agents/{name}", get(get_agent))
        .route("/runs", post(create_run))
        .route("/runs/{run_id}", get(get_run).post(resume_run))
        .route("/runs/{run_id}/cancel", post(cancel_run))
        .route("/runs/{run_id}/events", get(list_run_events))
        .route("/session/{session_id}", get(get_session))
        .route("/session/{session_id}/messages/{index}", get(get_session_message))
        .route("/session/{session_id}/state", get(get_session_state));

    #[cfg(feature = "well-known")]
    let router = router.route("/.well-known/agent.yml", get(well_known_agents));

    router.with_state(server)
}

/// Open discovery: the registered manifests as YAML at the well-known location.
///
/// ACP defines this so a crawler or another agent can find what a domain hosts
/// without knowing the ACP endpoints. The content is built from the same
/// manifests `GET /agents` serves, so the two cannot drift.
#[cfg(feature = "well-known")]
async fn well_known_agents(State(server): State<Arc<AcpServer>>) -> ApiResult<Response> {
    let body = serde_norway::to_string(&AgentsListResponse { agents: server.manifests() })
        .map_err(|err| {
            ApiError(Error::server_error(format!("failed to serialize agent manifests: {err}")))
        })?;
    Ok((
        StatusCode::OK,
        // RFC 9512 registers `application/yaml`.
        [(axum::http::header::CONTENT_TYPE, "application/yaml")],
        body,
    )
        .into_response())
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

async fn create_run(
    State(server): State<Arc<AcpServer>>,
    headers: HeaderMap,
    Json(request): Json<RunCreateRequest>,
) -> ApiResult<RunResponse> {
    request.validate().map_err(ApiError::from)?;
    let base_url = server.resolve_base_url(&headers);
    let mode = request.mode();

    let (run_id, notifications) =
        server.start_run(request, &base_url).await.map_err(ApiError::from)?;
    deliver(&server, run_id, notifications, mode, StatusCode::ACCEPTED).await
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

    let store = server.store();
    let run = store.require_run(run_id).await.map_err(ApiError::from)?;
    if run.status != RunStatus::Awaiting {
        return Err(Error::invalid_input(format!(
            "run {run_id} is `{}`; only an `awaiting` run can be resumed",
            run.status
        ))
        .into());
    }

    // Subscribe before publishing so nothing that the resume triggers is missed.
    let notifications = store.subscribe(run_id).await.map_err(ApiError::from)?;
    store
        .publish(run_id, Notification::Resume(request.await_resume))
        .await
        .map_err(ApiError::from)?;

    deliver(&server, run_id, notifications, request.mode, StatusCode::ACCEPTED).await
}

/// Turn a started or resumed run into the response the requested mode calls for.
async fn deliver(
    server: &Arc<AcpServer>,
    run_id: RunId,
    notifications: NotificationStream,
    mode: RunMode,
    async_status: StatusCode,
) -> ApiResult<RunResponse> {
    match mode {
        RunMode::Async => {
            let run = server.store().require_run(run_id).await.map_err(ApiError::from)?;
            Ok(RunResponse::Json(async_status, Box::new(run)))
        }
        RunMode::Sync => {
            wait_until_settled(server, run_id, notifications).await?;
            let run = server.store().require_run(run_id).await.map_err(ApiError::from)?;
            Ok(RunResponse::Json(StatusCode::OK, Box::new(run)))
        }
        RunMode::Stream => Ok(RunResponse::Stream(
            Sse::new(event_stream(notifications)).keep_alive(KeepAlive::default()).into_response(),
        )),
    }
}

/// Wait until the run reaches a terminal state or pauses awaiting input.
///
/// The caller must have subscribed *before* the action it is waiting on, so
/// nothing that action triggers can be missed.
///
/// The up-front read short-circuits only on a **terminal** status, never on
/// `awaiting`. A resume is issued against a run that is already `awaiting`, and
/// treating that as settled would return the pre-resume snapshot instead of
/// waiting for the agent to act on the payload. A terminal run, by contrast,
/// can never produce another notification, so returning immediately is the only
/// correct answer.
async fn wait_until_settled(
    server: &Arc<AcpServer>,
    run_id: RunId,
    mut notifications: NotificationStream,
) -> ApiResult<()> {
    if let Some(run) = server.store().get_run(run_id).await.map_err(ApiError::from)? {
        if run.status.is_terminal() {
            return Ok(());
        }
    }

    while let Some(notification) = notifications.next().await {
        let Some(event) = notification.event() else {
            continue;
        };
        // `run.awaiting` and the terminal `run.*` events are exactly the set
        // that ends a stream, which is the same set that settles a sync call.
        if event.is_terminal() {
            return Ok(());
        }
    }
    Ok(())
}

type EventStream = std::pin::Pin<Box<dyn Stream<Item = Result<SseEvent, Infallible>> + Send>>;

/// Adapt a run's notifications into an SSE stream that ends after the terminal
/// event, dropping control signals aimed at the executing replica.
fn event_stream(notifications: NotificationStream) -> EventStream {
    let mut done = false;
    let stream = notifications.filter_map(move |notification| {
        let item = if done {
            None
        } else {
            match notification {
                Notification::Event(event) => {
                    done = event.is_terminal();
                    Some(Ok(sse_event(&event)))
                }
                // Resume and Cancel are addressed to the replica running the
                // agent, not to a watching client.
                Notification::Resume(_) | Notification::Cancel => None,
            }
        };
        futures_util::future::ready(item)
    });
    Box::pin(stream)
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
    let run = server.store().require_run(run_id).await.map_err(ApiError::from)?;
    Ok(Json(run))
}

async fn cancel_run(
    State(server): State<Arc<AcpServer>>,
    Path(run_id): Path<String>,
) -> ApiResult<(StatusCode, Json<Run>)> {
    let run_id: RunId = run_id.parse().map_err(ApiError::from)?;
    let store = server.store();
    let run = store.require_run(run_id).await.map_err(ApiError::from)?;

    if !run.status.is_terminal() {
        // The executing replica decides when the run actually stops, so the
        // snapshot returned here may still read `in-progress`. That is what
        // 202 Accepted means.
        store.publish(run_id, Notification::Cancel).await.map_err(ApiError::from)?;
    }

    Ok((StatusCode::ACCEPTED, Json(run)))
}

async fn list_run_events(
    State(server): State<Arc<AcpServer>>,
    Path(run_id): Path<String>,
) -> ApiResult<Json<RunEventsListResponse>> {
    let run_id: RunId = run_id.parse().map_err(ApiError::from)?;
    let store = server.store();
    // Confirm the run exists so an unknown id is a 404 rather than an empty list.
    store.require_run(run_id).await.map_err(ApiError::from)?;
    let events = store.events(run_id).await.map_err(ApiError::from)?;
    Ok(Json(RunEventsListResponse { events }))
}

async fn get_session(
    State(server): State<Arc<AcpServer>>,
    Path(session_id): Path<String>,
) -> ApiResult<Json<Session>> {
    let session_id: SessionId = session_id.parse().map_err(ApiError::from)?;
    let record = server.store().require_session(session_id).await.map_err(ApiError::from)?;
    Ok(Json(record.session))
}

/// Resource endpoint backing the `state` URL on a [`Session`].
///
/// ACP models state as a link, so this is where that link resolves to. The
/// document is whatever the agent stored, verbatim.
async fn get_session_state(
    State(server): State<Arc<AcpServer>>,
    Path(session_id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let session_id: SessionId = session_id.parse().map_err(ApiError::from)?;
    let store = server.store();
    // Distinguish "no such session" from "session with no state yet".
    store.require_session(session_id).await.map_err(ApiError::from)?;
    let state = store.get_session_state(session_id).await.map_err(ApiError::from)?;
    state.map(Json).ok_or_else(|| {
        ApiError(Error::not_found(format!("session {session_id} has no stored state")))
    })
}

/// Resource endpoint backing the message URLs in a [`Session`]'s history.
async fn get_session_message(
    State(server): State<Arc<AcpServer>>,
    Path((session_id, index)): Path<(String, usize)>,
) -> ApiResult<Json<Message>> {
    let session_id: SessionId = session_id.parse().map_err(ApiError::from)?;
    let record = server.store().require_session(session_id).await.map_err(ApiError::from)?;
    let message = record.messages.get(index).cloned().ok_or_else(|| {
        Error::new(
            ErrorCode::NotFound,
            format!("session {session_id} has no message at index {index}"),
        )
    })?;
    Ok(Json(message))
}
