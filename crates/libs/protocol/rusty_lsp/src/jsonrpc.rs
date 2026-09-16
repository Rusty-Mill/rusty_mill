//! JSON-RPC 2.0 message model used by the LSP wire protocol.
//!
//! Three message shapes travel over the connection:
//! - [`Request`] — has an `id` and expects a [`Response`].
//! - [`Notification`] — fire-and-forget, no `id`.
//! - [`Response`] — carries the `id` of the request it answers.
//!
//! [`Message`] is the tagged union read off the wire. Its [`serde`]
//! implementation classifies an incoming object by the presence of the
//! `method` and `id` fields, which is more robust than `#[serde(untagged)]`
//! for this protocol.

use crate::error::{Error, ResponseError, Result};
use serde::de::{self, Deserializer};
use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The JSON-RPC protocol version string carried by every message.
pub const JSONRPC_VERSION: &str = "2.0";

/// A request identifier: either an integer or a string, per JSON-RPC.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    /// Numeric id (the common case for LSP clients).
    Number(i64),
    /// String id.
    String(String),
}

impl std::fmt::Display for RequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequestId::Number(n) => write!(f, "{n}"),
            RequestId::String(s) => write!(f, "{s}"),
        }
    }
}

impl TryFrom<Value> for RequestId {
    type Error = Error;

    fn try_from(value: Value) -> Result<Self> {
        match value {
            Value::Number(n) => n
                .as_i64()
                .map(RequestId::Number)
                .ok_or_else(|| Error::protocol("request id is not an integer")),
            Value::String(s) => Ok(RequestId::String(s)),
            other => Err(Error::protocol(format!("invalid request id: {other}"))),
        }
    }
}

/// A request: a method invocation that expects a matching [`Response`].
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    /// Correlation id echoed back in the response.
    pub id: RequestId,
    /// The method to invoke (e.g. `"textDocument/hover"`).
    pub method: String,
    /// Method parameters, if any.
    pub params: Option<Value>,
}

/// A notification: a method invocation with no response.
#[derive(Debug, Clone, PartialEq)]
pub struct Notification {
    /// The method being notified (e.g. `"textDocument/didOpen"`).
    pub method: String,
    /// Method parameters, if any.
    pub params: Option<Value>,
}

/// A response to a [`Request`]. Exactly one of `result` / `error` is set.
#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    /// The id of the request being answered. `None` only when the server could
    /// not determine the id of a malformed request.
    pub id: Option<RequestId>,
    /// The success payload (may be JSON `null`).
    pub result: Option<Value>,
    /// The failure payload.
    pub error: Option<ResponseError>,
}

impl Response {
    /// Build a success response for `id` carrying `result`.
    pub fn success(id: RequestId, result: Value) -> Self {
        Response {
            id: Some(id),
            result: Some(result),
            error: None,
        }
    }

    /// Build an error response for `id`.
    pub fn error(id: Option<RequestId>, error: ResponseError) -> Self {
        Response {
            id,
            result: None,
            error: Some(error),
        }
    }
}

/// Any message that can appear on the connection.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// A request expecting a response.
    Request(Request),
    /// A response to an earlier request.
    Response(Response),
    /// A fire-and-forget notification.
    Notification(Notification),
}

impl From<Request> for Message {
    fn from(r: Request) -> Self {
        Message::Request(r)
    }
}

impl From<Response> for Message {
    fn from(r: Response) -> Self {
        Message::Response(r)
    }
}

impl From<Notification> for Message {
    fn from(n: Notification) -> Self {
        Message::Notification(n)
    }
}

impl Serialize for Request {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("jsonrpc", JSONRPC_VERSION)?;
        map.serialize_entry("id", &self.id)?;
        map.serialize_entry("method", &self.method)?;
        if let Some(params) = &self.params {
            map.serialize_entry("params", params)?;
        }
        map.end()
    }
}

impl Serialize for Notification {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("jsonrpc", JSONRPC_VERSION)?;
        map.serialize_entry("method", &self.method)?;
        if let Some(params) = &self.params {
            map.serialize_entry("params", params)?;
        }
        map.end()
    }
}

impl Serialize for Response {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("jsonrpc", JSONRPC_VERSION)?;
        // JSON-RPC requires the id member to be present in a response; emit
        // null when the request id was indeterminate.
        match &self.id {
            Some(id) => map.serialize_entry("id", id)?,
            None => map.serialize_entry("id", &Value::Null)?,
        }
        // Exactly one of result/error. Prefer error when both are somehow set.
        if let Some(error) = &self.error {
            map.serialize_entry("error", error)?;
        } else {
            // `result` may legitimately be null (e.g. the shutdown response).
            map.serialize_entry("result", self.result.as_ref().unwrap_or(&Value::Null))?;
        }
        map.end()
    }
}

impl Serialize for Message {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Message::Request(r) => r.serialize(serializer),
            Message::Response(r) => r.serialize(serializer),
            Message::Notification(n) => n.serialize(serializer),
        }
    }
}

/// Flattened view of any incoming message, used to classify it.
#[derive(Deserialize)]
struct RawMessage {
    #[serde(default)]
    id: Option<Value>,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    params: Option<Value>,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<ResponseError>,
}

impl<'de> Deserialize<'de> for Message {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = RawMessage::deserialize(deserializer)?;

        // A `method` field means it is a client->server call: request if it
        // also carries a (non-null) id, otherwise a notification.
        if let Some(method) = raw.method {
            return match raw.id {
                Some(id) if !id.is_null() => {
                    let id = RequestId::try_from(id).map_err(de::Error::custom)?;
                    Ok(Message::Request(Request {
                        id,
                        method,
                        params: raw.params,
                    }))
                }
                _ => Ok(Message::Notification(Notification {
                    method,
                    params: raw.params,
                })),
            };
        }

        // No `method`: it is a response to one of our requests.
        let id = match raw.id {
            Some(id) if !id.is_null() => Some(RequestId::try_from(id).map_err(de::Error::custom)?),
            _ => None,
        };
        Ok(Message::Response(Response {
            id,
            result: raw.result,
            error: raw.error,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(value: serde_json::Value) -> Message {
        serde_json::from_value(value).expect("valid message")
    }

    #[test]
    fn classifies_request() {
        let msg = parse(json!({"jsonrpc": "2.0", "id": 1, "method": "hover", "params": {"x": 1}}));
        let Message::Request(req) = msg else {
            panic!("expected request")
        };
        assert_eq!(req.id, RequestId::Number(1));
        assert_eq!(req.method, "hover");
        assert_eq!(req.params, Some(json!({"x": 1})));
    }

    #[test]
    fn classifies_notification_by_absent_id() {
        let msg = parse(json!({"jsonrpc": "2.0", "method": "didOpen", "params": {}}));
        let Message::Notification(note) = msg else {
            panic!("expected notification")
        };
        assert_eq!(note.method, "didOpen");
    }

    #[test]
    fn classifies_response_by_absent_method() {
        let msg = parse(json!({"jsonrpc": "2.0", "id": "abc", "result": 42}));
        let Message::Response(resp) = msg else {
            panic!("expected response")
        };
        assert_eq!(resp.id, Some(RequestId::String("abc".to_owned())));
        assert_eq!(resp.result, Some(json!(42)));
        assert!(resp.error.is_none());
    }

    #[test]
    fn response_with_null_id_has_none() {
        let msg =
            parse(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "x"}}));
        let Message::Response(resp) = msg else {
            panic!("expected response")
        };
        assert_eq!(resp.id, None);
        assert_eq!(resp.error.unwrap().code, -32700);
    }

    #[test]
    fn serializes_success_response() {
        let value = serde_json::to_value(Message::Response(Response::success(
            RequestId::Number(7),
            json!({"ok": true}),
        )))
        .unwrap();
        assert_eq!(
            value,
            json!({"jsonrpc": "2.0", "id": 7, "result": {"ok": true}})
        );
    }

    #[test]
    fn serializes_null_result() {
        // The shutdown response carries a null result, which must be emitted.
        let value = serde_json::to_value(Message::Response(Response::success(
            RequestId::Number(1),
            Value::Null,
        )))
        .unwrap();
        assert_eq!(value, json!({"jsonrpc": "2.0", "id": 1, "result": null}));
    }

    #[test]
    fn serializes_notification_without_id() {
        let value = serde_json::to_value(Message::Notification(Notification {
            method: "window/logMessage".to_owned(),
            params: Some(json!({"type": 3, "message": "hi"})),
        }))
        .unwrap();
        assert_eq!(
            value,
            json!({"jsonrpc": "2.0", "method": "window/logMessage", "params": {"type": 3, "message": "hi"}})
        );
        assert!(value.get("id").is_none());
    }
}
