//! Exposes an [`Engine`] over HTTP using the A2A HTTP+JSON/REST protocol
//! binding (spec Section 11), as an additional binding alongside the
//! JSON-RPC one in [`super::router`]. Both can be mounted on the same
//! `axum::Router` and served from a single port; declare both interfaces
//! in your `AgentCard` (see [`crate::types::AgentInterface::http_json`]).
//!
//! Unlike the JSON-RPC binding, REST responses use real HTTP status codes
//! and the [`google.rpc.Status`](https://github.com/googleapis/googleapis/blob/master/google/rpc/status.proto)
//! JSON shape for errors (spec Section 11.6).
//!
//! `axum`'s router (`matchit`) treats `:` as "start of a named parameter"
//! *anywhere* it appears in a route pattern string, not only at the start
//! of a segment - so `/message:send` cannot be registered as a literal
//! route. The message and per-task-action endpoints below instead capture
//! the whole segment with an ordinary named param and split on `:` inside
//! the handler; a colon has no special meaning when matching an *actual*
//! request path against an already-compiled param.
//!
//! The proto's `additional_bindings` (serving every route again nested
//! under a `/{tenant}` path prefix) aren't implemented; a `tenant` value
//! can still be sent per-request via the `tenant` field already present
//! on the JSON body (`POST`) or query string (`GET`) of every operation -
//! the same scope the JSON-RPC binding has.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_core::Stream;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::json;

use crate::error::A2aError;
use crate::types::{
    CancelTaskRequest, DeleteTaskPushNotificationConfigRequest, GetTaskPushNotificationConfigRequest,
    GetTaskRequest, ListTaskPushNotificationConfigsRequest, ListTasksRequest, SendMessageRequest,
    StreamResponse, SubscribeToTaskRequest, TaskPushNotificationConfig,
};

use super::auth::extract_credentials;
use super::engine::Engine;

const A2A_JSON: &str = "application/a2a+json";

pub(crate) fn build_rest_router(engine: Arc<Engine>) -> Router {
    Router::new()
        // "/message:send" and "/message:stream" both capture as `:action`
        // (a single top-level dynamic segment, disambiguated by value
        // inside `message_action`); see the module docs for why they
        // can't be written as literal patterns.
        .route("/:action", post(message_action))
        .route("/tasks", get(list_tasks))
        .route("/tasks/:id", get(get_task).post(task_action))
        .route(
            "/tasks/:id/pushNotificationConfigs",
            post(create_push_notification_config).get(list_push_notification_configs),
        )
        .route(
            "/tasks/:id/pushNotificationConfigs/:config_id",
            get(get_push_notification_config).delete(delete_push_notification_config),
        )
        .route("/extendedAgentCard", get(get_extended_agent_card))
        .with_state(engine)
}

fn a2a_json<T: serde::Serialize>(status: StatusCode, value: &T) -> Response {
    let body = serde_json::to_vec(value).unwrap_or_default();
    (status, [(axum::http::header::CONTENT_TYPE, A2A_JSON)], body).into_response()
}

/// Maps an [`A2aError`] to the `google.rpc.Status` JSON shape (spec
/// Section 11.6), with the real HTTP status code on the response.
fn rest_error(err: A2aError) -> Response {
    let status = StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let reason = err.reason().unwrap_or_else(|| err.grpc_status_name());
    let body = json!({
        "error": {
            "code": status.as_u16(),
            "status": err.grpc_status_name(),
            "message": err.standard_message(),
            "details": [{
                "@type": "type.googleapis.com/google.rpc.ErrorInfo",
                "reason": reason,
                "domain": "a2a-protocol.org",
            }],
        }
    });
    a2a_json(status, &body)
}

fn rest_ok<T: serde::Serialize>(value: &T) -> Response {
    a2a_json(StatusCode::OK, value)
}

/// Extracts credentials for `AgentCard.securitySchemes` from `headers`
/// (and `query`, where the route has a meaningful query string) and
/// enforces `AgentCard.securityRequirements` (spec Section 4.5) against
/// them, returning an error [`Response`] ready to `return` on failure.
async fn require_auth(
    engine: &Engine,
    headers: &HeaderMap,
    query: &HashMap<String, String>,
) -> Result<(), Response> {
    let credentials = extract_credentials(
        &engine.card().security_schemes,
        |name| {
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        },
        Some(query),
    );
    engine
        .authenticate(&credentials)
        .await
        .map(|_| ())
        .map_err(rest_error)
}

/// SSE for the REST binding carries the raw `StreamResponse` object
/// directly in `data:` (no JSON-RPC envelope), per spec Section 11.7.
fn sse_response(stream: Pin<Box<dyn Stream<Item = StreamResponse> + Send>>) -> Response {
    let sse_stream = stream.map(|item| Event::default().json_data(item));
    Sse::new(sse_stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// `POST /message:send` and `POST /message:stream` (spec Section 11.3.1).
async fn message_action(
    State(engine): State<Arc<Engine>>,
    Path(action): Path<String>,
    headers: HeaderMap,
    Json(req): Json<SendMessageRequest>,
) -> Response {
    if let Err(resp) = require_auth(&engine, &headers, &HashMap::new()).await {
        return resp;
    }
    match action.as_str() {
        "message:send" => match engine.send_message(req).await {
            Ok(result) => rest_ok(&result),
            Err(e) => rest_error(e),
        },
        "message:stream" => match engine.send_streaming_message(req).await {
            Ok(stream) => sse_response(stream),
            Err(e) => rest_error(e),
        },
        other => rest_error(A2aError::MethodNotFound(other.to_string())),
    }
}

#[derive(Debug, Deserialize)]
struct GetTaskQuery {
    #[serde(rename = "historyLength", default)]
    history_length: Option<i32>,
}

/// `GET /tasks/{id}` (spec Section 11.3.2).
async fn get_task(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(query): Query<GetTaskQuery>,
    Query(raw_query): Query<HashMap<String, String>>,
) -> Response {
    if let Err(resp) = require_auth(&engine, &headers, &raw_query).await {
        return resp;
    }
    let req = GetTaskRequest {
        tenant: None,
        id,
        history_length: query.history_length,
    };
    match engine.get_task(req).await {
        Ok(task) => rest_ok(&task),
        Err(e) => rest_error(e),
    }
}

/// `GET /tasks` (spec Section 11.3.2).
async fn list_tasks(
    State(engine): State<Arc<Engine>>,
    headers: HeaderMap,
    Query(req): Query<ListTasksRequest>,
    Query(raw_query): Query<HashMap<String, String>>,
) -> Response {
    if let Err(resp) = require_auth(&engine, &headers, &raw_query).await {
        return resp;
    }
    match engine.list_tasks(req).await {
        Ok(res) => rest_ok(&res),
        Err(e) => rest_error(e),
    }
}

/// Handles both `POST /tasks/{id}:cancel` and `POST /tasks/{id}:subscribe`
/// (spec Section 11.3.2), dispatching on the suffix after the last `:` in
/// the path segment.
async fn task_action(
    State(engine): State<Arc<Engine>>,
    Path(id_and_action): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(resp) = require_auth(&engine, &headers, &HashMap::new()).await {
        return resp;
    }
    let Some((id, action)) = id_and_action.rsplit_once(':') else {
        return rest_error(A2aError::InvalidRequest(format!(
            "expected \"{{id}}:cancel\" or \"{{id}}:subscribe\", got \"{id_and_action}\""
        )));
    };
    match action {
        "cancel" => {
            let req = CancelTaskRequest {
                tenant: None,
                id: id.to_string(),
                metadata: None,
            };
            match engine.cancel_task(req).await {
                Ok(task) => rest_ok(&task),
                Err(e) => rest_error(e),
            }
        }
        "subscribe" => {
            let req = SubscribeToTaskRequest {
                tenant: None,
                id: id.to_string(),
            };
            match engine.subscribe_to_task(req).await {
                Ok(stream) => sse_response(stream),
                Err(e) => rest_error(e),
            }
        }
        other => rest_error(A2aError::InvalidRequest(format!(
            "unknown task action \"{other}\""
        ))),
    }
}

/// `POST /tasks/{id}/pushNotificationConfigs` (spec Section 11.3.3). The
/// path's `id` always wins over whatever `taskId` (if any) is in the body.
async fn create_push_notification_config(
    State(engine): State<Arc<Engine>>,
    Path(task_id): Path<String>,
    headers: HeaderMap,
    Json(mut config): Json<TaskPushNotificationConfig>,
) -> Response {
    if let Err(resp) = require_auth(&engine, &headers, &HashMap::new()).await {
        return resp;
    }
    config.task_id = Some(task_id);
    match engine.create_push_notification_config(config).await {
        Ok(cfg) => rest_ok(&cfg),
        Err(e) => rest_error(e),
    }
}

/// `GET /tasks/{id}/pushNotificationConfigs/{configId}` (spec Section
/// 11.3.3).
async fn get_push_notification_config(
    State(engine): State<Arc<Engine>>,
    Path((task_id, config_id)): Path<(String, String)>,
    headers: HeaderMap,
    Query(raw_query): Query<HashMap<String, String>>,
) -> Response {
    if let Err(resp) = require_auth(&engine, &headers, &raw_query).await {
        return resp;
    }
    let req = GetTaskPushNotificationConfigRequest {
        tenant: None,
        task_id,
        id: config_id,
    };
    match engine.get_push_notification_config(req).await {
        Ok(cfg) => rest_ok(&cfg),
        Err(e) => rest_error(e),
    }
}

#[derive(Debug, Deserialize)]
struct PageQuery {
    #[serde(rename = "pageSize", default)]
    page_size: Option<i32>,
    #[serde(rename = "pageToken", default)]
    page_token: Option<String>,
}

/// `GET /tasks/{id}/pushNotificationConfigs` (spec Section 11.3.3).
async fn list_push_notification_configs(
    State(engine): State<Arc<Engine>>,
    Path(task_id): Path<String>,
    headers: HeaderMap,
    Query(query): Query<PageQuery>,
    Query(raw_query): Query<HashMap<String, String>>,
) -> Response {
    if let Err(resp) = require_auth(&engine, &headers, &raw_query).await {
        return resp;
    }
    let req = ListTaskPushNotificationConfigsRequest {
        tenant: None,
        task_id,
        page_size: query.page_size,
        page_token: query.page_token,
    };
    match engine.list_push_notification_configs(req).await {
        Ok(res) => rest_ok(&res),
        Err(e) => rest_error(e),
    }
}

/// `DELETE /tasks/{id}/pushNotificationConfigs/{configId}` (spec Section
/// 11.3.3).
async fn delete_push_notification_config(
    State(engine): State<Arc<Engine>>,
    Path((task_id, config_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    if let Err(resp) = require_auth(&engine, &headers, &HashMap::new()).await {
        return resp;
    }
    let req = DeleteTaskPushNotificationConfigRequest {
        tenant: None,
        task_id,
        id: config_id,
    };
    match engine.delete_push_notification_config(req).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => rest_error(e),
    }
}

/// `GET /extendedAgentCard` (spec Section 11.3.4).
async fn get_extended_agent_card(State(engine): State<Arc<Engine>>, headers: HeaderMap) -> Response {
    let credentials = extract_credentials(
        &engine.card().security_schemes,
        |name| {
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        },
        None,
    );
    match engine.get_extended_agent_card(&credentials).await {
        Ok(card) => rest_ok(&card),
        Err(e) => rest_error(e),
    }
}
