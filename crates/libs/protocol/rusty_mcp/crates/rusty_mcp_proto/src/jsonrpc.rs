//! JSON-RPC 2.0 messages as MCP uses them.
//!
//! A [`Message`] is a request (has `method` and `id`), a notification (has
//! `method`, no `id`), a success response (`result`) or an error response
//! (`error`, with a possibly-null `id`). Batches are not decoded here.

use crate::util::{expect_object, field, opt_value, string, Obj};
use crate::{Error, Result};
use rusty_json::Value;

/// JSON-RPC and MCP error codes.
pub mod code {
    /// Invalid JSON was received.
    pub const PARSE_ERROR: i64 = -32700;
    /// The JSON is not a valid request object.
    pub const INVALID_REQUEST: i64 = -32600;
    /// The method does not exist.
    pub const METHOD_NOT_FOUND: i64 = -32601;
    /// Invalid method parameters.
    pub const INVALID_PARAMS: i64 = -32602;
    /// Internal error.
    pub const INTERNAL_ERROR: i64 = -32603;
    /// MCP: the requested resource does not exist.
    pub const RESOURCE_NOT_FOUND: i64 = -32002;
}

/// A request id: a number or a string.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RequestId {
    /// A numeric id.
    Number(i64),
    /// A string id.
    String(String),
}

impl RequestId {
    /// This id as a JSON value.
    pub fn to_value(&self) -> Value {
        match self {
            RequestId::Number(n) => Value::from(*n),
            RequestId::String(s) => Value::from(s.as_str()),
        }
    }

    /// Decode an id from a JSON value (a string or an integer).
    pub fn from_value(v: &Value) -> Result<Self> {
        if let Some(s) = v.as_str() {
            return Ok(RequestId::String(s.to_string()));
        }
        v.as_i64()
            .map(RequestId::Number)
            .ok_or_else(|| Error::decode("id", "not a string or integer"))
    }
}

/// The `error` member of an error response.
#[derive(Clone, Debug, PartialEq)]
pub struct ErrorObject {
    /// The error code; see [`code`].
    pub code: i64,
    /// A short description.
    pub message: String,
    /// Extra information, if any.
    pub data: Option<Value>,
}

impl ErrorObject {
    /// An error with no `data`.
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        ErrorObject {
            code,
            message: message.into(),
            data: None,
        }
    }

    fn to_value(&self) -> Value {
        Obj::new()
            .set("code", self.code)
            .set("message", self.message.as_str())
            .opt("data", self.data.clone())
            .finish()
    }

    fn from_value(v: &Value) -> Result<Self> {
        expect_object(v, "error")?;
        let code = field(v, "error", "code")?
            .as_i64()
            .ok_or_else(|| Error::decode("error", "\"code\" is not an integer"))?;
        Ok(ErrorObject {
            code,
            message: string(v, "error", "message")?,
            data: opt_value(v, "data"),
        })
    }
}

/// One JSON-RPC message.
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    /// A call that expects a response.
    Request {
        /// The id the response must carry.
        id: RequestId,
        /// The method name, for example `tools/call`.
        method: String,
        /// The parameters, if any.
        params: Option<Value>,
    },
    /// A one-way message.
    Notification {
        /// The method name, for example `notifications/initialized`.
        method: String,
        /// The parameters, if any.
        params: Option<Value>,
    },
    /// A success response.
    Response {
        /// The id of the request being answered.
        id: RequestId,
        /// The result.
        result: Value,
    },
    /// An error response. `id` is `None` when the request id could not be read.
    Error {
        /// The id of the request being answered, if known.
        id: Option<RequestId>,
        /// What went wrong.
        error: ErrorObject,
    },
}

impl Message {
    /// This message as a JSON value, with `"jsonrpc": "2.0"`.
    pub fn to_value(&self) -> Value {
        let base = Obj::new().set("jsonrpc", "2.0");
        match self {
            Message::Request { id, method, params } => base
                .set("id", id.to_value())
                .set("method", method.as_str())
                .opt("params", params.clone())
                .finish(),
            Message::Notification { method, params } => base
                .set("method", method.as_str())
                .opt("params", params.clone())
                .finish(),
            Message::Response { id, result } => base
                .set("id", id.to_value())
                .set("result", result.clone())
                .finish(),
            Message::Error { id, error } => base
                .set("id", id.as_ref().map_or(Value::Null, RequestId::to_value))
                .set("error", error.to_value())
                .finish(),
        }
    }

    /// Decode a message. Unknown members are ignored.
    pub fn from_value(v: &Value) -> Result<Self> {
        expect_object(v, "message")?;
        if v.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Err(Error::decode("message", "\"jsonrpc\" is not \"2.0\""));
        }
        let id = match v.get("id") {
            None | Some(Value::Null) => None,
            Some(raw) => Some(RequestId::from_value(raw)?),
        };
        if v.get("method").is_some() {
            let method = string(v, "message", "method")?;
            let params = opt_value(v, "params");
            return Ok(match id {
                Some(id) => Message::Request { id, method, params },
                None => Message::Notification { method, params },
            });
        }
        if let Some(error) = v.get("error") {
            return Ok(Message::Error {
                id,
                error: ErrorObject::from_value(error)?,
            });
        }
        if let Some(result) = v.get("result") {
            let id = id.ok_or_else(|| Error::decode("message", "response without an id"))?;
            return Ok(Message::Response {
                id,
                result: result.clone(),
            });
        }
        Err(Error::decode(
            "message",
            "no method, result or error member",
        ))
    }

    /// This message as compact JSON text.
    pub fn to_json(&self) -> String {
        self.to_value().to_json_string()
    }

    /// Parse JSON text into a message.
    pub fn from_json(text: &str) -> Result<Self> {
        let value = Value::parse(text).map_err(|e| Error::Json(e.to_string()))?;
        Self::from_value(&value)
    }
}
