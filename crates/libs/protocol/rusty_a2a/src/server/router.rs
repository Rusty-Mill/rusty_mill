//! Exposes an [`Engine`] over HTTP using the A2A JSON-RPC 2.0 protocol
//! binding (spec Section 9) plus Agent Card discovery (spec Section 8.2).
//!
//! All JSON-RPC requests, streaming or not, are POSTed to `/`. Per spec
//! Section 9.2, errors are conveyed at the JSON-RPC layer (the `error`
//! field of the response envelope); this implementation therefore always
//! answers a syntactically valid JSON-RPC request with HTTP `200 OK`,
//! reserving other HTTP status codes for transport-level failures (e.g.
//! `400` for a body that isn't valid JSON at all).

use std::pin::Pin;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_core::Stream;
use futures_util::StreamExt;
use serde_json::{json, Value};

use crate::error::A2aError;
use crate::types::jsonrpc::{methods, JsonRpcErrorObject, JsonRpcRequest, JsonRpcResponse, RequestId};
use crate::types::{
    CancelTaskRequest, DeleteTaskPushNotificationConfigRequest, GetTaskPushNotificationConfigRequest,
    GetTaskRequest, ListTaskPushNotificationConfigsRequest, ListTasksRequest, SendMessageRequest,
    StreamResponse, SubscribeToTaskRequest, TaskPushNotificationConfig,
};

use super::auth::extract_credentials;
use super::engine::{check_version, parse_extensions_header, Engine};

pub(crate) fn build_router(engine: Arc<Engine>) -> Router {
    Router::new()
        .route(crate::AGENT_CARD_WELL_KNOWN_PATH, get(agent_card_handler))
        .route("/", post(jsonrpc_handler))
        .with_state(engine)
}

/// Spec Section 8.6.1 (SHOULD): the Agent Card endpoint sends `Cache-Control`
/// and an `ETag` (derived from `version`), and honors a conditional-GET
/// `If-None-Match` with a bare `304`.
async fn agent_card_handler(State(engine): State<Arc<Engine>>, headers: HeaderMap) -> Response {
    let etag = engine.agent_card_etag();
    let cache_control = "public, max-age=300";
    if headers
        .get(axum::http::header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        == Some(etag)
    {
        return (
            StatusCode::NOT_MODIFIED,
            [
                (axum::http::header::ETAG, etag),
                (axum::http::header::CACHE_CONTROL, cache_control),
            ],
        )
            .into_response();
    }
    (
        StatusCode::OK,
        [
            (axum::http::header::ETAG, etag),
            (axum::http::header::CACHE_CONTROL, cache_control),
        ],
        Json(engine.card().clone()),
    )
        .into_response()
}

async fn jsonrpc_handler(State(engine): State<Arc<Engine>>, headers: HeaderMap, body: Bytes) -> Response {
    // Spec Section 9.5 distinguishes these: `-32700` ("The server received
    // invalid JSON") is for input that isn't valid JSON at all; `-32600`
    // ("The JSON sent is not a valid Request object") is for
    // syntactically valid JSON that just doesn't have the shape of a
    // JSON-RPC request (wrong/missing `jsonrpc`/`method`, ...) - parsing
    // in two steps is what lets these be told apart, rather than
    // collapsing both into `-32700`.
    let value: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => {
            let err = A2aError::ParseError;
            let body = json!({
                "jsonrpc": "2.0",
                "id": Value::Null,
                "error": JsonRpcErrorObject::from(&err),
            });
            return (StatusCode::BAD_REQUEST, Json(body)).into_response();
        }
    };
    let envelope: JsonRpcRequest = match serde_json::from_value(value) {
        Ok(r) => r,
        Err(e) => {
            let err = A2aError::InvalidRequest(e.to_string());
            let body = json!({
                "jsonrpc": "2.0",
                "id": Value::Null,
                "error": JsonRpcErrorObject::from(&err),
            });
            return (StatusCode::BAD_REQUEST, Json(body)).into_response();
        }
    };

    let version_header = headers.get("A2A-Version").and_then(|v| v.to_str().ok());
    if let Err(version_err) = check_version(version_header) {
        return jsonrpc_error_response(envelope.id, version_err);
    }

    let declared_extensions =
        parse_extensions_header(headers.get("A2A-Extensions").and_then(|v| v.to_str().ok()));
    if let Err(ext_err) = engine.check_required_extensions(&declared_extensions) {
        return jsonrpc_error_response(envelope.id, ext_err);
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
    let mut auth_context = None;
    if envelope.method.as_str() != methods::GET_EXTENDED_AGENT_CARD {
        match engine.authenticate(&credentials).await {
            Ok(ctx) => auth_context = ctx,
            Err(auth_err) => return jsonrpc_error_response(envelope.id, auth_err),
        }
    }

    let params = envelope.params.unwrap_or(Value::Null);

    macro_rules! parse_params {
        () => {
            match serde_json::from_value(params) {
                Ok(p) => p,
                Err(e) => return jsonrpc_error_response(envelope.id, A2aError::InvalidParams(e.to_string())),
            }
        };
    }

    match envelope.method.as_str() {
        methods::SEND_MESSAGE => {
            let req: SendMessageRequest = parse_params!();
            match engine.send_message(req, auth_context.as_ref()).await {
                Ok(result) => jsonrpc_ok(envelope.id, &result),
                Err(e) => jsonrpc_error_response(envelope.id, e),
            }
        }
        methods::SEND_STREAMING_MESSAGE => {
            let req: SendMessageRequest = parse_params!();
            match engine.send_streaming_message(req, auth_context.as_ref()).await {
                Ok(stream) => sse_response(envelope.id, stream),
                Err(e) => jsonrpc_error_response(envelope.id, e),
            }
        }
        methods::GET_TASK => {
            let req: GetTaskRequest = parse_params!();
            match engine.get_task(req, auth_context.as_ref()).await {
                Ok(task) => jsonrpc_ok(envelope.id, &task),
                Err(e) => jsonrpc_error_response(envelope.id, e),
            }
        }
        methods::LIST_TASKS => {
            let req: ListTasksRequest = parse_params!();
            match engine.list_tasks(req, auth_context.as_ref()).await {
                Ok(res) => jsonrpc_ok(envelope.id, &res),
                Err(e) => jsonrpc_error_response(envelope.id, e),
            }
        }
        methods::CANCEL_TASK => {
            let req: CancelTaskRequest = parse_params!();
            match engine.cancel_task(req, auth_context.as_ref()).await {
                Ok(task) => jsonrpc_ok(envelope.id, &task),
                Err(e) => jsonrpc_error_response(envelope.id, e),
            }
        }
        methods::SUBSCRIBE_TO_TASK => {
            let req: SubscribeToTaskRequest = parse_params!();
            let since_seq = parse_last_event_id(&headers);
            match engine
                .subscribe_to_task(req, since_seq, auth_context.as_ref())
                .await
            {
                Ok(stream) => sse_subscribe_response(envelope.id, stream),
                Err(e) => jsonrpc_error_response(envelope.id, e),
            }
        }
        methods::CREATE_TASK_PUSH_NOTIFICATION_CONFIG => {
            let req: TaskPushNotificationConfig = parse_params!();
            match engine
                .create_push_notification_config(req, auth_context.as_ref())
                .await
            {
                Ok(cfg) => jsonrpc_ok(envelope.id, &cfg),
                Err(e) => jsonrpc_error_response(envelope.id, e),
            }
        }
        methods::GET_TASK_PUSH_NOTIFICATION_CONFIG => {
            let req: GetTaskPushNotificationConfigRequest = parse_params!();
            match engine
                .get_push_notification_config(req, auth_context.as_ref())
                .await
            {
                Ok(cfg) => jsonrpc_ok(envelope.id, &cfg),
                Err(e) => jsonrpc_error_response(envelope.id, e),
            }
        }
        methods::LIST_TASK_PUSH_NOTIFICATION_CONFIGS => {
            let req: ListTaskPushNotificationConfigsRequest = parse_params!();
            match engine
                .list_push_notification_configs(req, auth_context.as_ref())
                .await
            {
                Ok(res) => jsonrpc_ok(envelope.id, &res),
                Err(e) => jsonrpc_error_response(envelope.id, e),
            }
        }
        methods::DELETE_TASK_PUSH_NOTIFICATION_CONFIG => {
            let req: DeleteTaskPushNotificationConfigRequest = parse_params!();
            match engine
                .delete_push_notification_config(req, auth_context.as_ref())
                .await
            {
                Ok(()) => jsonrpc_ok(envelope.id, &Value::Null),
                Err(e) => jsonrpc_error_response(envelope.id, e),
            }
        }
        methods::GET_EXTENDED_AGENT_CARD => match engine.get_extended_agent_card(&credentials).await {
            Ok(card) => jsonrpc_ok(envelope.id, &card),
            Err(e) => jsonrpc_error_response(envelope.id, e),
        },
        other => jsonrpc_error_response(envelope.id, A2aError::MethodNotFound(other.to_string())),
    }
}

fn jsonrpc_ok<T: serde::Serialize>(id: RequestId, value: &T) -> Response {
    let result = serde_json::to_value(value).unwrap_or(Value::Null);
    (StatusCode::OK, Json(JsonRpcResponse::success(id, result))).into_response()
}

fn jsonrpc_error_response(id: RequestId, err: A2aError) -> Response {
    (StatusCode::OK, Json(JsonRpcResponse::failure(id, &err))).into_response()
}

fn sse_response(id: RequestId, stream: Pin<Box<dyn Stream<Item = StreamResponse> + Send>>) -> Response {
    let sse_stream = stream.map(move |item| {
        let result = serde_json::to_value(&item).unwrap_or(Value::Null);
        let payload = JsonRpcResponse::success(id.clone(), result);
        Event::default().json_data(payload)
    });
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

/// Like [`sse_response`], but for `SubscribeToTask`: each event's SSE
/// `id:` field is set to its sequence number, so a client (or a
/// spec-compliant `EventSource`) that reconnects sends it back as
/// `Last-Event-ID` and [`parse_last_event_id`] can resume the replay from
/// exactly where it left off.
fn sse_subscribe_response(
    id: RequestId,
    stream: Pin<Box<dyn Stream<Item = (u64, StreamResponse)> + Send>>,
) -> Response {
    let sse_stream = stream.map(move |(seq, item)| {
        let result = serde_json::to_value(&item).unwrap_or(Value::Null);
        let payload = JsonRpcResponse::success(id.clone(), result);
        Event::default().id(seq.to_string()).json_data(payload)
    });
    Sse::new(sse_stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}
