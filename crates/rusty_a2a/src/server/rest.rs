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
//! Every route is registered twice: once as-is, and once again nested
//! under a `/{tenant}` path prefix (the proto's `additional_bindings`,
//! e.g. `POST /{tenant}/message:send`). A `tenant` value can also still be
//! sent via the `tenant` field on the JSON body for operations that have
//! one, or a `?tenant=` query parameter otherwise (`GET`s, and the
//! body-less `:cancel`/`:subscribe`/`DELETE` actions) - the same scope the
//! JSON-RPC binding has; the path segment wins when both are present.
//! Every handler honors it: [`super::store::TaskStore`] treats each
//! tenant (including the absent one) as a fully isolated namespace, so a
//! task or push notification config created under one tenant is
//! invisible - not just inaccessible - to a request that omits `tenant`
//! or names a different one.
//!
//! Each pair of routes (bare and `/{tenant}`-prefixed) shares one `_impl`
//! function taking an explicit `tenant: Option<String>` - the two thin
//! `axum` handlers differ only in how many path segments they capture and
//! whether they pass `Some(tenant)` from the path.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{FromRequest, FromRequestParts, Path, Query, Request, State};
use axum::http::request::Parts;
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

use super::auth::{extract_credentials, AuthContext};
use super::engine::{check_version, parse_extensions_header, Engine};

const A2A_JSON: &str = "application/a2a+json";

pub(crate) fn build_rest_router(engine: Arc<Engine>) -> Router {
    Router::new()
        // "/message:send" and "/message:stream" both capture as `:action`
        // (a single top-level dynamic segment, disambiguated by value
        // inside `message_action`); see the module docs for why they
        // can't be written as literal patterns.
        .route("/:action", post(message_action))
        .route("/tasks", get(list_tasks))
        .route("/tasks/:id", get(get_task_or_subscribe).post(task_action))
        .route(
            "/tasks/:id/pushNotificationConfigs",
            post(create_push_notification_config).get(list_push_notification_configs),
        )
        .route(
            "/tasks/:id/pushNotificationConfigs/:config_id",
            get(get_push_notification_config).delete(delete_push_notification_config),
        )
        .route("/extendedAgentCard", get(get_extended_agent_card))
        // The proto's `additional_bindings`: every route above, again
        // nested under a `/{tenant}` path prefix.
        .route("/:tenant/:action", post(message_action_tenant))
        .route("/:tenant/tasks", get(list_tasks_tenant))
        .route(
            "/:tenant/tasks/:id",
            get(get_task_or_subscribe_tenant).post(task_action_tenant),
        )
        .route(
            "/:tenant/tasks/:id/pushNotificationConfigs",
            post(create_push_notification_config_tenant).get(list_push_notification_configs_tenant),
        )
        .route(
            "/:tenant/tasks/:id/pushNotificationConfigs/:config_id",
            get(get_push_notification_config_tenant).delete(delete_push_notification_config_tenant),
        )
        .route("/:tenant/extendedAgentCard", get(get_extended_agent_card_tenant))
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

const CACHE_CONTROL_VALUE: &str = "public, max-age=300";

/// Like [`rest_ok`], but with `ETag`/`Cache-Control` headers (spec Section
/// 13.3, SHOULD), and honoring a matching `If-None-Match` with a bare
/// `304` - see [`super::router`]'s `agent_card_handler`, which does the
/// same for the base Agent Card.
fn rest_ok_cached<T: serde::Serialize>(value: &T, etag: &str, if_none_match: Option<&str>) -> Response {
    if if_none_match == Some(etag) {
        return (
            StatusCode::NOT_MODIFIED,
            [
                (axum::http::header::ETAG, etag.to_string()),
                (axum::http::header::CACHE_CONTROL, CACHE_CONTROL_VALUE.to_string()),
            ],
        )
            .into_response();
    }
    let body = serde_json::to_vec(value).unwrap_or_default();
    (
        StatusCode::OK,
        [
            (axum::http::header::CONTENT_TYPE, A2A_JSON.to_string()),
            (axum::http::header::ETAG, etag.to_string()),
            (axum::http::header::CACHE_CONTROL, CACHE_CONTROL_VALUE.to_string()),
        ],
        body,
    )
        .into_response()
}

/// Validates the `A2A-Version` header (spec Section 3.2.6 / 3.6.2), then
/// enforces `AgentCard.capabilities.extensions[].required` (spec Section
/// 3.2.6 / 5.6) from the `A2A-Extensions` header, then extracts
/// credentials for `AgentCard.securitySchemes` from `headers` (and
/// `query`, where the route has a meaningful query string) and enforces
/// `AgentCard.securityRequirements` (spec Section 4.5) against them.
/// Returns an error [`Response`] (boxed: `clippy::result_large_err` --
/// `axum::http::Response<axum::body::Body>` is well past the lint's size
/// threshold) ready to `return *err` on failure.
async fn require_auth(
    engine: &Engine,
    headers: &HeaderMap,
    query: &HashMap<String, String>,
) -> Result<Option<AuthContext>, Box<Response>> {
    check_version(headers.get("A2A-Version").and_then(|v| v.to_str().ok()))
        .map_err(|e| Box::new(rest_error(e)))?;

    let declared_extensions =
        parse_extensions_header(headers.get("A2A-Extensions").and_then(|v| v.to_str().ok()));
    engine
        .check_required_extensions(&declared_extensions)
        .map_err(|e| Box::new(rest_error(e)))?;

    let credentials = extract_credentials(
        &engine.card().security_schemes,
        |name| {
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        },
        Some(query),
        engine.mtls_header(),
    );
    engine
        .authenticate(&credentials)
        .await
        .map_err(|e| Box::new(rest_error(e)))
}

/// Like [`axum::Json`], but a rejection (malformed JSON, or JSON that
/// doesn't match `T`'s shape) is reported through [`rest_error`] - the
/// `google.rpc.Status` JSON shape (spec Section 11.6) - instead of
/// `axum`'s own plain-text rejection response, which every other error
/// this binding can produce is deliberately *not* shaped like.
struct A2aJson<T>(T);

#[async_trait::async_trait]
impl<T, S> FromRequest<S> for A2aJson<T>
where
    T: serde::de::DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(req: Request, state: &S) -> std::result::Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(value)) => Ok(A2aJson(value)),
            Err(rejection) => Err(rest_error(json_rejection_to_a2a(rejection))),
        }
    }
}

fn json_rejection_to_a2a(rejection: JsonRejection) -> A2aError {
    A2aError::InvalidParams(rejection.body_text())
}

/// Like [`axum::extract::Query`], but a rejection (a query parameter that
/// doesn't parse into `T`'s shape, e.g. `pageSize=notanumber`) is reported
/// through [`rest_error`] instead of `axum`'s own plain-text rejection
/// response - see [`A2aJson`].
struct A2aQuery<T>(T);

#[async_trait::async_trait]
impl<T, S> FromRequestParts<S> for A2aQuery<T>
where
    T: serde::de::DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> std::result::Result<Self, Self::Rejection> {
        match Query::<T>::from_request_parts(parts, state).await {
            Ok(Query(value)) => Ok(A2aQuery(value)),
            Err(rejection) => Err(rest_error(query_rejection_to_a2a(rejection))),
        }
    }
}

fn query_rejection_to_a2a(rejection: QueryRejection) -> A2aError {
    A2aError::InvalidParams(rejection.body_text())
}

/// SSE for the REST binding carries the raw `StreamResponse` object
/// directly in `data:` (no JSON-RPC envelope), per spec Section 11.7.
fn sse_response(stream: Pin<Box<dyn Stream<Item = StreamResponse> + Send>>) -> Response {
    let sse_stream = stream.map(|item| Event::default().json_data(item));
    Sse::new(sse_stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// Parses the standard SSE `Last-Event-ID` reconnect header (sent
/// automatically by browser `EventSource` implementations, and settable
/// manually by any other client) into the sequence number
/// [`Engine::subscribe_to_task`] should replay events after.
fn parse_last_event_id(headers: &HeaderMap) -> Option<u64> {
    headers
        .get("Last-Event-ID")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
}

/// Like [`sse_response`], but for the `:subscribe` action: each event's
/// SSE `id:` field is set to its sequence number, so a client (or a
/// spec-compliant `EventSource`) that reconnects sends it back as
/// `Last-Event-ID` and [`parse_last_event_id`] can resume the replay from
/// exactly where it left off.
fn sse_subscribe_response(stream: Pin<Box<dyn Stream<Item = (u64, StreamResponse)> + Send>>) -> Response {
    let sse_stream = stream.map(|(seq, item)| Event::default().id(seq.to_string()).json_data(item));
    Sse::new(sse_stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// `POST /message:send` and `POST /message:stream` (spec Section 11.3.1).
async fn message_action(
    State(engine): State<Arc<Engine>>,
    Path(action): Path<String>,
    headers: HeaderMap,
    A2aJson(req): A2aJson<SendMessageRequest>,
) -> Response {
    message_action_impl(engine, None, action, headers, req).await
}

/// Like [`message_action`], but nested under `/{tenant}` (spec's
/// `additional_bindings`): `POST /{tenant}/message:send` /
/// `POST /{tenant}/message:stream`.
async fn message_action_tenant(
    State(engine): State<Arc<Engine>>,
    Path((tenant, action)): Path<(String, String)>,
    headers: HeaderMap,
    A2aJson(req): A2aJson<SendMessageRequest>,
) -> Response {
    message_action_impl(engine, Some(tenant), action, headers, req).await
}

async fn message_action_impl(
    engine: Arc<Engine>,
    tenant: Option<String>,
    action: String,
    headers: HeaderMap,
    mut req: SendMessageRequest,
) -> Response {
    let auth = match require_auth(&engine, &headers, &HashMap::new()).await {
        Ok(auth) => auth,
        Err(resp) => return *resp,
    };
    if tenant.is_some() {
        req.tenant = tenant;
    }
    match action.as_str() {
        "message:send" => match engine.send_message(req, auth.as_ref()).await {
            Ok(result) => rest_ok(&result),
            Err(e) => rest_error(e),
        },
        "message:stream" => match engine.send_streaming_message(req, auth.as_ref()).await {
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

/// Handles both `GET /tasks/{id}` and the spec-literal `GET
/// /tasks/{id}:subscribe` (spec Sections 3.1.6 / 11.3.2), dispatching on an
/// optional `:subscribe` suffix in the path segment the same way
/// [`task_action`] dispatches `POST /tasks/{id}:cancel` -
/// `POST /tasks/{id}:subscribe` (this crate's original, non-spec-literal
/// wiring, kept for backward compatibility) still works too.
async fn get_task_or_subscribe(
    State(engine): State<Arc<Engine>>,
    Path(id_and_action): Path<String>,
    headers: HeaderMap,
    A2aQuery(query): A2aQuery<GetTaskQuery>,
    Query(raw_query): Query<HashMap<String, String>>,
) -> Response {
    let tenant = raw_query.get("tenant").cloned();
    get_task_or_subscribe_impl(engine, tenant, id_and_action, headers, query, raw_query).await
}

/// Like [`get_task_or_subscribe`], but nested under `/{tenant}`:
/// `GET /{tenant}/tasks/{id}` / `GET /{tenant}/tasks/{id}:subscribe`.
async fn get_task_or_subscribe_tenant(
    State(engine): State<Arc<Engine>>,
    Path((tenant, id_and_action)): Path<(String, String)>,
    headers: HeaderMap,
    A2aQuery(query): A2aQuery<GetTaskQuery>,
    Query(raw_query): Query<HashMap<String, String>>,
) -> Response {
    get_task_or_subscribe_impl(engine, Some(tenant), id_and_action, headers, query, raw_query).await
}

async fn get_task_or_subscribe_impl(
    engine: Arc<Engine>,
    tenant: Option<String>,
    id_and_action: String,
    headers: HeaderMap,
    query: GetTaskQuery,
    raw_query: HashMap<String, String>,
) -> Response {
    let auth = match require_auth(&engine, &headers, &raw_query).await {
        Ok(auth) => auth,
        Err(resp) => return *resp,
    };
    match id_and_action.rsplit_once(':') {
        Some((id, "subscribe")) => {
            let req = SubscribeToTaskRequest {
                tenant,
                id: id.to_string(),
            };
            let since_seq = parse_last_event_id(&headers);
            match engine.subscribe_to_task(req, since_seq, auth.as_ref()).await {
                Ok(stream) => sse_subscribe_response(stream),
                Err(e) => rest_error(e),
            }
        }
        Some((_, other)) => rest_error(A2aError::InvalidRequest(format!(
            "unknown task action \"{other}\""
        ))),
        None => {
            let req = GetTaskRequest {
                tenant,
                id: id_and_action,
                history_length: query.history_length,
            };
            match engine.get_task(req, auth.as_ref()).await {
                Ok(task) => rest_ok(&task),
                Err(e) => rest_error(e),
            }
        }
    }
}

/// `GET /tasks` (spec Section 11.3.2).
async fn list_tasks(
    State(engine): State<Arc<Engine>>,
    headers: HeaderMap,
    A2aQuery(req): A2aQuery<ListTasksRequest>,
    Query(raw_query): Query<HashMap<String, String>>,
) -> Response {
    list_tasks_impl(engine, None, headers, req, raw_query).await
}

/// Like [`list_tasks`], but nested under `/{tenant}`: `GET /{tenant}/tasks`.
async fn list_tasks_tenant(
    State(engine): State<Arc<Engine>>,
    Path(tenant): Path<String>,
    headers: HeaderMap,
    A2aQuery(req): A2aQuery<ListTasksRequest>,
    Query(raw_query): Query<HashMap<String, String>>,
) -> Response {
    list_tasks_impl(engine, Some(tenant), headers, req, raw_query).await
}

async fn list_tasks_impl(
    engine: Arc<Engine>,
    tenant: Option<String>,
    headers: HeaderMap,
    mut req: ListTasksRequest,
    raw_query: HashMap<String, String>,
) -> Response {
    let auth = match require_auth(&engine, &headers, &raw_query).await {
        Ok(auth) => auth,
        Err(resp) => return *resp,
    };
    if tenant.is_some() {
        req.tenant = tenant;
    }
    match engine.list_tasks(req, auth.as_ref()).await {
        Ok(res) => rest_ok(&res),
        Err(e) => rest_error(e),
    }
}

/// Handles `POST /tasks/{id}:cancel` (spec Section 11.3.2, the only
/// literal binding `CancelTask` has) and, for backward compatibility,
/// `POST /tasks/{id}:subscribe` too - the spec-literal binding for
/// `SubscribeToTask` is `GET`, handled by
/// [`get_task_or_subscribe`] instead, but this crate wired subscribe as
/// `POST` before that was added, so both still work. Dispatches on the
/// suffix after the last `:` in the path segment.
async fn task_action(
    State(engine): State<Arc<Engine>>,
    Path(id_and_action): Path<String>,
    headers: HeaderMap,
    Query(raw_query): Query<HashMap<String, String>>,
) -> Response {
    let tenant = raw_query.get("tenant").cloned();
    task_action_impl(engine, tenant, id_and_action, headers, raw_query).await
}

/// Like [`task_action`], but nested under `/{tenant}`:
/// `POST /{tenant}/tasks/{id}:cancel` / `POST /{tenant}/tasks/{id}:subscribe`.
async fn task_action_tenant(
    State(engine): State<Arc<Engine>>,
    Path((tenant, id_and_action)): Path<(String, String)>,
    headers: HeaderMap,
    Query(raw_query): Query<HashMap<String, String>>,
) -> Response {
    task_action_impl(engine, Some(tenant), id_and_action, headers, raw_query).await
}

async fn task_action_impl(
    engine: Arc<Engine>,
    tenant: Option<String>,
    id_and_action: String,
    headers: HeaderMap,
    raw_query: HashMap<String, String>,
) -> Response {
    let auth = match require_auth(&engine, &headers, &raw_query).await {
        Ok(auth) => auth,
        Err(resp) => return *resp,
    };
    let Some((id, action)) = id_and_action.rsplit_once(':') else {
        return rest_error(A2aError::InvalidRequest(format!(
            "expected \"{{id}}:cancel\" or \"{{id}}:subscribe\", got \"{id_and_action}\""
        )));
    };
    match action {
        "cancel" => {
            let req = CancelTaskRequest {
                tenant,
                id: id.to_string(),
                metadata: None,
            };
            match engine.cancel_task(req, auth.as_ref()).await {
                Ok(task) => rest_ok(&task),
                Err(e) => rest_error(e),
            }
        }
        "subscribe" => {
            let req = SubscribeToTaskRequest {
                tenant,
                id: id.to_string(),
            };
            let since_seq = parse_last_event_id(&headers);
            match engine.subscribe_to_task(req, since_seq, auth.as_ref()).await {
                Ok(stream) => sse_subscribe_response(stream),
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
    A2aJson(config): A2aJson<TaskPushNotificationConfig>,
) -> Response {
    create_push_notification_config_impl(engine, None, task_id, headers, config).await
}

/// Like [`create_push_notification_config`], but nested under `/{tenant}`:
/// `POST /{tenant}/tasks/{id}/pushNotificationConfigs`.
async fn create_push_notification_config_tenant(
    State(engine): State<Arc<Engine>>,
    Path((tenant, task_id)): Path<(String, String)>,
    headers: HeaderMap,
    A2aJson(config): A2aJson<TaskPushNotificationConfig>,
) -> Response {
    create_push_notification_config_impl(engine, Some(tenant), task_id, headers, config).await
}

async fn create_push_notification_config_impl(
    engine: Arc<Engine>,
    tenant: Option<String>,
    task_id: String,
    headers: HeaderMap,
    mut config: TaskPushNotificationConfig,
) -> Response {
    let auth = match require_auth(&engine, &headers, &HashMap::new()).await {
        Ok(auth) => auth,
        Err(resp) => return *resp,
    };
    config.task_id = Some(task_id);
    if tenant.is_some() {
        config.tenant = tenant;
    }
    match engine
        .create_push_notification_config(config, auth.as_ref())
        .await
    {
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
    let tenant = raw_query.get("tenant").cloned();
    get_push_notification_config_impl(engine, tenant, task_id, config_id, headers, raw_query).await
}

/// Like [`get_push_notification_config`], but nested under `/{tenant}`:
/// `GET /{tenant}/tasks/{id}/pushNotificationConfigs/{configId}`.
async fn get_push_notification_config_tenant(
    State(engine): State<Arc<Engine>>,
    Path((tenant, task_id, config_id)): Path<(String, String, String)>,
    headers: HeaderMap,
    Query(raw_query): Query<HashMap<String, String>>,
) -> Response {
    get_push_notification_config_impl(engine, Some(tenant), task_id, config_id, headers, raw_query).await
}

async fn get_push_notification_config_impl(
    engine: Arc<Engine>,
    tenant: Option<String>,
    task_id: String,
    config_id: String,
    headers: HeaderMap,
    raw_query: HashMap<String, String>,
) -> Response {
    let auth = match require_auth(&engine, &headers, &raw_query).await {
        Ok(auth) => auth,
        Err(resp) => return *resp,
    };
    let req = GetTaskPushNotificationConfigRequest {
        tenant,
        task_id,
        id: config_id,
    };
    match engine.get_push_notification_config(req, auth.as_ref()).await {
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
    A2aQuery(query): A2aQuery<PageQuery>,
    Query(raw_query): Query<HashMap<String, String>>,
) -> Response {
    let tenant = raw_query.get("tenant").cloned();
    list_push_notification_configs_impl(engine, tenant, task_id, headers, query, raw_query).await
}

/// Like [`list_push_notification_configs`], but nested under `/{tenant}`:
/// `GET /{tenant}/tasks/{id}/pushNotificationConfigs`.
async fn list_push_notification_configs_tenant(
    State(engine): State<Arc<Engine>>,
    Path((tenant, task_id)): Path<(String, String)>,
    headers: HeaderMap,
    A2aQuery(query): A2aQuery<PageQuery>,
    Query(raw_query): Query<HashMap<String, String>>,
) -> Response {
    list_push_notification_configs_impl(engine, Some(tenant), task_id, headers, query, raw_query).await
}

async fn list_push_notification_configs_impl(
    engine: Arc<Engine>,
    tenant: Option<String>,
    task_id: String,
    headers: HeaderMap,
    query: PageQuery,
    raw_query: HashMap<String, String>,
) -> Response {
    let auth = match require_auth(&engine, &headers, &raw_query).await {
        Ok(auth) => auth,
        Err(resp) => return *resp,
    };
    let req = ListTaskPushNotificationConfigsRequest {
        tenant,
        task_id,
        page_size: query.page_size,
        page_token: query.page_token,
    };
    match engine.list_push_notification_configs(req, auth.as_ref()).await {
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
    Query(raw_query): Query<HashMap<String, String>>,
) -> Response {
    let tenant = raw_query.get("tenant").cloned();
    delete_push_notification_config_impl(engine, tenant, task_id, config_id, headers, raw_query).await
}

/// Like [`delete_push_notification_config`], but nested under
/// `/{tenant}`: `DELETE /{tenant}/tasks/{id}/pushNotificationConfigs/{configId}`.
async fn delete_push_notification_config_tenant(
    State(engine): State<Arc<Engine>>,
    Path((tenant, task_id, config_id)): Path<(String, String, String)>,
    headers: HeaderMap,
    Query(raw_query): Query<HashMap<String, String>>,
) -> Response {
    delete_push_notification_config_impl(engine, Some(tenant), task_id, config_id, headers, raw_query).await
}

async fn delete_push_notification_config_impl(
    engine: Arc<Engine>,
    tenant: Option<String>,
    task_id: String,
    config_id: String,
    headers: HeaderMap,
    raw_query: HashMap<String, String>,
) -> Response {
    let auth = match require_auth(&engine, &headers, &raw_query).await {
        Ok(auth) => auth,
        Err(resp) => return *resp,
    };
    let req = DeleteTaskPushNotificationConfigRequest {
        tenant,
        task_id,
        id: config_id,
    };
    match engine.delete_push_notification_config(req, auth.as_ref()).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => rest_error(e),
    }
}

/// `GET /extendedAgentCard` (spec Section 11.3.4).
async fn get_extended_agent_card(State(engine): State<Arc<Engine>>, headers: HeaderMap) -> Response {
    get_extended_agent_card_impl(engine, headers).await
}

/// Like [`get_extended_agent_card`], but nested under `/{tenant}`:
/// `GET /{tenant}/extendedAgentCard`.
///
/// The tenant path segment is accepted for symmetry with every other
/// route but otherwise unused: `GetExtendedAgentCard` returns this
/// agent's card (spec Section 3.1.11), which isn't itself tenant-scoped
/// data, so there's nothing for a tenant value to select between.
async fn get_extended_agent_card_tenant(
    State(engine): State<Arc<Engine>>,
    Path(_tenant): Path<String>,
    headers: HeaderMap,
) -> Response {
    get_extended_agent_card_impl(engine, headers).await
}

async fn get_extended_agent_card_impl(engine: Arc<Engine>, headers: HeaderMap) -> Response {
    if let Err(e) = check_version(headers.get("A2A-Version").and_then(|v| v.to_str().ok())) {
        return rest_error(e);
    }
    let declared_extensions =
        parse_extensions_header(headers.get("A2A-Extensions").and_then(|v| v.to_str().ok()));
    if let Err(e) = engine.check_required_extensions(&declared_extensions) {
        return rest_error(e);
    }
    let credentials = extract_credentials(
        &engine.card().security_schemes,
        |name| {
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        },
        None,
        engine.mtls_header(),
    );
    match engine.get_extended_agent_card(&credentials).await {
        Ok(card) => match engine.extended_card_etag() {
            Some(etag) => {
                let if_none_match = headers
                    .get(axum::http::header::IF_NONE_MATCH)
                    .and_then(|v| v.to_str().ok());
                rest_ok_cached(&card, etag, if_none_match)
            }
            None => rest_ok(&card),
        },
        Err(e) => rest_error(e),
    }
}
