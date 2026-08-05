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
        reap_if_abandoned,
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

/// Read a run, failing it first if its executing replica is gone.
///
/// Every path that reads a run goes through here, so an abandoned run is
/// noticed by whoever next asks about it. That is deliberately lazy: no
/// background sweeper, no extra moving parts, and the check costs one lease
/// lookup on reads that were already hitting the store.
async fn require_live_run(server: &Arc<AcpServer>, run_id: RunId) -> ApiResult<Run> {
    let run = server.store().require_run(run_id).await.map_err(ApiError::from)?;
    reap_if_abandoned(server, run).await.map_err(ApiError::from)
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

/// The event log, in whichever shape the client asked for.
enum RunEventsResponse {
    Json(RunEventsListResponse),
    Stream(Response),
}

impl IntoResponse for RunEventsResponse {
    fn into_response(self) -> Response {
        match self {
            RunEventsResponse::Json(events) => Json(events).into_response(),
            RunEventsResponse::Stream(response) => response,
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

    let run = require_live_run(&server, run_id).await?;
    let store = server.store();
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
            let run = require_live_run(server, run_id).await?;
            Ok(RunResponse::Json(StatusCode::OK, Box::new(run)))
        }
        // A run being started or resumed has nothing to replay: the caller is
        // watching from here on, not catching up.
        RunMode::Stream => Ok(RunResponse::Stream(
            Sse::new(event_stream(0, Vec::new(), notifications))
                .keep_alive(KeepAlive::default())
                .into_response(),
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

    // Between notifications, check that the run still has an owner. Without
    // this a caller would wait out the whole timeout on a run whose replica had
    // already died — the run.failed that unblocks them is only published once
    // somebody looks.
    let lease_ttl = server.lease_ttl();
    let deadline = server.sync_timeout().map(|timeout| tokio::time::Instant::now() + timeout);

    loop {
        let next = notifications.next();
        let tick = tokio::time::sleep(lease_ttl);

        let notification = match deadline {
            Some(deadline) => tokio::select! {
                notification = next => notification,
                _ = tick => None,
                _ = tokio::time::sleep_until(deadline) => {
                    // Not an error: the run is still going, and the caller gets
                    // an honest snapshot of where it got to.
                    tracing::debug!(%run_id, "sync request timed out; returning the run as it stands");
                    return Ok(());
                }
            },
            None => tokio::select! {
                notification = next => notification,
                _ = tick => None,
            },
        };

        match notification {
            // `run.awaiting` and the terminal `run.*` events are exactly the
            // set that ends a stream, which is the same set that settles a
            // sync call.
            Some(notification) => {
                if notification.event().is_some_and(Event::is_terminal) {
                    return Ok(());
                }
            }
            // Either the tick fired or the channel closed. Check for
            // abandonment; if the run is settled either way, we are done.
            None => {
                let run = server.store().get_run(run_id).await.map_err(ApiError::from)?;
                let Some(run) = run else {
                    return Ok(());
                };
                let run = reap_if_abandoned(server, run).await.map_err(ApiError::from)?;
                if run.status.is_terminal() || run.status.is_awaiting() {
                    return Ok(());
                }
            }
        }
    }
}

type EventStream = std::pin::Pin<Box<dyn Stream<Item = Result<SseEvent, Infallible>> + Send>>;

/// How far a stream has got, and whether it is finished.
struct StreamCursor {
    /// The lowest log index not yet sent.
    next_index: u64,
    done: bool,
}

/// Adapt a run's notifications into an SSE stream that ends after the terminal
/// event, dropping control signals aimed at the executing replica.
///
/// `replay` is the slice of the log to send before attaching to the live
/// subscription, with `first_index` the index of its first entry. A fresh
/// stream passes an empty replay starting at zero.
///
/// # Splicing the replay onto the live subscription
///
/// The caller must subscribe **before** reading the log. That ordering is what
/// rules out a gap: anything appended after the read arrives live, and anything
/// before it is in the read, so no event can fall between the two. It does
/// admit an overlap — events appended between subscribing and reading appear in
/// both — and that is what the index is for. A live event whose index was
/// already covered by the replay is dropped, exactly, rather than guessed at
/// from arrival order.
fn event_stream(
    first_index: u64,
    replay: Vec<Event>,
    notifications: NotificationStream,
) -> EventStream {
    let replayed = futures_util::stream::iter(
        replay
            .into_iter()
            .enumerate()
            .map(move |(offset, event)| (Some(first_index + offset as u64), event)),
    );

    let live = notifications.filter_map(|notification| {
        futures_util::future::ready(match notification {
            Notification::Event { event, index } => Some((index, event)),
            // Resume and Cancel are addressed to the replica running the agent,
            // not to a watching client.
            Notification::Resume(_) | Notification::Cancel => None,
        })
    });

    let cursor = StreamCursor { next_index: first_index, done: false };
    let stream = futures_util::stream::unfold(
        (Box::pin(replayed.chain(live)), cursor),
        |(mut inner, mut cursor)| async move {
            // Checked before awaiting, so a stream that has sent its terminal
            // event ends rather than holding the connection open waiting for a
            // notification that can never come.
            if cursor.done {
                return None;
            }
            while let Some((index, event)) = inner.next().await {
                if index.is_some_and(|index| index < cursor.next_index) {
                    continue;
                }
                if let Some(index) = index {
                    cursor.next_index = index + 1;
                }
                cursor.done = event.is_terminal();
                return Some((Ok(sse_event(index, &event)), (inner, cursor)));
            }
            None
        },
    );
    Box::pin(stream)
}

/// Render an event as SSE, tagging it with its log index so a client that
/// drops can ask for everything after it.
///
/// An event with no index is one a backend synthesised rather than one the run
/// emitted. It deliberately carries no `id`, so receiving one does not move the
/// client's resume point past anything real.
fn sse_event(index: Option<u64>, event: &Event) -> SseEvent {
    let sse = match index {
        Some(index) => SseEvent::default().id(index.to_string()),
        None => SseEvent::default(),
    };
    match sse.event(event.event_type()).json_data(event) {
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
    Ok(Json(require_live_run(&server, run_id).await?))
}

async fn cancel_run(
    State(server): State<Arc<AcpServer>>,
    Path(run_id): Path<String>,
) -> ApiResult<(StatusCode, Json<Run>)> {
    let run_id: RunId = run_id.parse().map_err(ApiError::from)?;
    let run = require_live_run(&server, run_id).await?;
    let store = server.store();

    if !run.status.is_terminal() {
        // The executing replica decides when the run actually stops, so the
        // snapshot returned here may still read `in-progress`. That is what
        // 202 Accepted means.
        store.publish(run_id, Notification::Cancel).await.map_err(ApiError::from)?;
    }

    Ok((StatusCode::ACCEPTED, Json(run)))
}

/// The run's event log: a JSON list, or an SSE stream if the client asks for
/// one with `Accept: text/event-stream`.
///
/// The streaming half is how a dropped stream is resumed. It is an extension —
/// the OpenAPI document describes only the list — so the JSON form stays the
/// default and a client that says nothing gets exactly what the spec promises.
async fn list_run_events(
    State(server): State<Arc<AcpServer>>,
    Path(run_id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<RunEventsResponse> {
    let run_id: RunId = run_id.parse().map_err(ApiError::from)?;
    // Confirm the run exists so an unknown id is a 404 rather than an empty
    // list — and reap it first, so the log ends with `run.failed` rather than
    // trailing off mid-run.
    require_live_run(&server, run_id).await?;

    if !wants_event_stream(&headers) {
        let events = server.store().events(run_id).await.map_err(ApiError::from)?;
        return Ok(RunEventsResponse::Json(RunEventsListResponse { events }));
    }

    // Resume after the last event the client acknowledged; with no header, send
    // the log from the beginning.
    let from = last_event_id(&headers).map_or(0, |last| last + 1);

    // Subscribe before reading the log. See `event_stream` for why this
    // ordering is what makes the splice gapless.
    let notifications = server.store().subscribe(run_id).await.map_err(ApiError::from)?;
    let replay = server.store().events_from(run_id, from).await.map_err(ApiError::from)?;

    Ok(RunEventsResponse::Stream(
        Sse::new(event_stream(from, replay, notifications))
            .keep_alive(KeepAlive::default())
            .into_response(),
    ))
}

/// Whether the client asked for an SSE stream rather than the JSON list.
fn wants_event_stream(headers: &HeaderMap) -> bool {
    headers.get(axum::http::header::ACCEPT).and_then(|value| value.to_str().ok()).is_some_and(
        |accept| {
            accept.split(',').any(|entry| {
                entry.split(';').next().is_some_and(|media| media.trim() == "text/event-stream")
            })
        },
    )
}

/// The index a resuming client last saw, from `Last-Event-ID`.
///
/// An unparseable value is treated as absent: replaying the whole log is
/// wasteful but correct, where guessing an offset would not be.
fn last_event_id(headers: &HeaderMap) -> Option<u64> {
    headers.get("last-event-id")?.to_str().ok()?.trim().parse().ok()
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
