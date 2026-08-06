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
use super::engine::Engine;

pub(crate) fn build_router(engine: Arc<Engine>) -> Router {
    Router::new()
        .route(crate::AGENT_CARD_WELL_KNOWN_PATH, get(agent_card_handler))
        .route("/", post(jsonrpc_handler))
        .with_state(engine)
}

async fn agent_card_handler(State(engine): State<Arc<Engine>>) -> impl IntoResponse {
    Json(engine.card().clone())
}

/// Validates the `A2A-Version` service parameter (spec Section 3.2.6 /
/// 3.6.2): an absent or empty header is treated as version `0.3`; any
/// value other than exactly [`crate::PROTOCOL_VERSION`] is rejected.
fn check_version(headers: &HeaderMap) -> Result<(), A2aError> {
    let version = headers
        .get("A2A-Version")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty())
        .unwrap_or("0.3");
    if version == crate::PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(A2aError::VersionNotSupported(version.to_string()))
    }
}

async fn jsonrpc_handler(State(engine): State<Arc<Engine>>, headers: HeaderMap, body: Bytes) -> Response {
    let envelope: JsonRpcRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
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

    if let Err(version_err) = check_version(&headers) {
        return jsonrpc_error_response(envelope.id, version_err);
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
    );
    if envelope.method.as_str() != methods::GET_EXTENDED_AGENT_CARD {
        if let Err(auth_err) = engine.authenticate(&credentials).await {
            return jsonrpc_error_response(envelope.id, auth_err);
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
            match engine.send_message(req).await {
                Ok(result) => jsonrpc_ok(envelope.id, &result),
                Err(e) => jsonrpc_error_response(envelope.id, e),
            }
        }
        methods::SEND_STREAMING_MESSAGE => {
            let req: SendMessageRequest = parse_params!();
            match engine.send_streaming_message(req).await {
                Ok(stream) => sse_response(envelope.id, stream),
                Err(e) => jsonrpc_error_response(envelope.id, e),
            }
        }
        methods::GET_TASK => {
            let req: GetTaskRequest = parse_params!();
            match engine.get_task(req).await {
                Ok(task) => jsonrpc_ok(envelope.id, &task),
                Err(e) => jsonrpc_error_response(envelope.id, e),
            }
        }
        methods::LIST_TASKS => {
            let req: ListTasksRequest = parse_params!();
            match engine.list_tasks(req).await {
                Ok(res) => jsonrpc_ok(envelope.id, &res),
                Err(e) => jsonrpc_error_response(envelope.id, e),
            }
        }
        methods::CANCEL_TASK => {
            let req: CancelTaskRequest = parse_params!();
            match engine.cancel_task(req).await {
                Ok(task) => jsonrpc_ok(envelope.id, &task),
                Err(e) => jsonrpc_error_response(envelope.id, e),
            }
        }
        methods::SUBSCRIBE_TO_TASK => {
            let req: SubscribeToTaskRequest = parse_params!();
            match engine.subscribe_to_task(req).await {
                Ok(stream) => sse_response(envelope.id, stream),
                Err(e) => jsonrpc_error_response(envelope.id, e),
            }
        }
        methods::CREATE_TASK_PUSH_NOTIFICATION_CONFIG => {
            let req: TaskPushNotificationConfig = parse_params!();
            match engine.create_push_notification_config(req).await {
                Ok(cfg) => jsonrpc_ok(envelope.id, &cfg),
                Err(e) => jsonrpc_error_response(envelope.id, e),
            }
        }
        methods::GET_TASK_PUSH_NOTIFICATION_CONFIG => {
            let req: GetTaskPushNotificationConfigRequest = parse_params!();
            match engine.get_push_notification_config(req).await {
                Ok(cfg) => jsonrpc_ok(envelope.id, &cfg),
                Err(e) => jsonrpc_error_response(envelope.id, e),
            }
        }
        methods::LIST_TASK_PUSH_NOTIFICATION_CONFIGS => {
            let req: ListTaskPushNotificationConfigsRequest = parse_params!();
            match engine.list_push_notification_configs(req).await {
                Ok(res) => jsonrpc_ok(envelope.id, &res),
                Err(e) => jsonrpc_error_response(envelope.id, e),
            }
        }
        methods::DELETE_TASK_PUSH_NOTIFICATION_CONFIG => {
            let req: DeleteTaskPushNotificationConfigRequest = parse_params!();
            match engine.delete_push_notification_config(req).await {
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
