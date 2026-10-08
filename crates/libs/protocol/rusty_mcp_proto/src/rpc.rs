//! The JSON-RPC 2.0 envelope MCP rides on: requests, notifications,
//! responses and errors. Batches are not part of MCP and are refused.

use crate::codec::{field, object, opt_string, opt_value, string, Obj, Wire};
use crate::{Error, Result};
use rusty_json::Value;

/// The `jsonrpc` member every message carries.
const VERSION: &str = "2.0";

/// A request id: a number or a string, never null.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RequestId {
    /// A numeric id.
    Number(i64),
    /// A string id.
    String(String),
}

impl Wire for RequestId {
    fn to_value(&self) -> Value {
        match self {
            RequestId::Number(n) => Value::from(*n),
            RequestId::String(s) => Value::from(s.as_str()),
        }
    }

    fn from_value(v: &Value) -> Result<Self> {
        match v {
            Value::String(s) => Ok(RequestId::String(s.clone())),
            Value::Number(_) => v
                .as_i64()
                .map(RequestId::Number)
                .ok_or_else(|| Error::decode("request id", "not an integer")),
            _ => Err(Error::decode("request id", "not a number or string")),
        }
    }
}

/// A JSON-RPC error code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ErrorCode(pub i32);

impl ErrorCode {
    /// The server could not parse the JSON.
    pub const PARSE_ERROR: Self = Self(-32700);
    /// The message is not a valid request.
    pub const INVALID_REQUEST: Self = Self(-32600);
    /// No such method.
    pub const METHOD_NOT_FOUND: Self = Self(-32601);
    /// The parameters are wrong.
    pub const INVALID_PARAMS: Self = Self(-32602);
    /// The server failed.
    pub const INTERNAL_ERROR: Self = Self(-32603);
    /// A resource URI names nothing.
    pub const RESOURCE_NOT_FOUND: Self = Self(-32002);
    /// 2026-07-28: an `Mcp-*` header disagrees with the body.
    pub const HEADER_MISMATCH: Self = Self(-32020);
    /// 2026-07-28: the request needs a capability the client did not declare.
    pub const MISSING_REQUIRED_CLIENT_CAPABILITY: Self = Self(-32021);
    /// 2026-07-28: the requested protocol revision is not served.
    pub const UNSUPPORTED_PROTOCOL_VERSION: Self = Self(-32022);
}

/// The `error` member of an error response.
#[derive(Clone, Debug, PartialEq)]
pub struct ErrorData {
    /// What went wrong.
    pub code: ErrorCode,
    /// A short description.
    pub message: String,
    /// Extra detail, kept as raw JSON.
    pub data: Option<Value>,
}

impl ErrorData {
    /// An error without extra data.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }
}

impl Wire for ErrorData {
    fn to_value(&self) -> Value {
        Obj::new()
            .set("code", i64::from(self.code.0))
            .set("message", self.message.as_str())
            .opt_value("data", &self.data)
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "error";
        object(v, W)?;
        let code = field(v, W, "code")?
            .as_i64()
            .and_then(|c| i32::try_from(c).ok())
            .ok_or_else(|| Error::decode(W, "\"code\" is not a 32-bit integer"))?;
        Ok(Self {
            code: ErrorCode(code),
            message: string(v, W, "message")?,
            data: opt_value(v, "data"),
        })
    }
}

/// One JSON-RPC message. Parameters and results stay raw JSON here; decode
/// them with [`params`] or `Wire::from_value` once the method is known.
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    /// A call that expects a response.
    Request {
        /// Correlates the response.
        id: RequestId,
        /// The method name.
        method: String,
        /// The parameters, absent or object or array.
        params: Option<Value>,
    },
    /// A call that expects none.
    Notification {
        /// The method name.
        method: String,
        /// The parameters.
        params: Option<Value>,
    },
    /// A successful answer.
    Response {
        /// The request it answers.
        id: RequestId,
        /// The result.
        result: Value,
    },
    /// A failed answer. `id` is `None` when the request id was unreadable.
    Error {
        /// The request it answers.
        id: Option<RequestId>,
        /// What went wrong.
        error: ErrorData,
    },
}

impl Message {
    /// A request carrying `params`.
    pub fn request(id: RequestId, method: &str, params: &impl Wire) -> Self {
        Message::Request {
            id,
            method: method.to_owned(),
            params: Some(params.to_value()),
        }
    }

    /// A notification carrying `params`.
    pub fn notification(method: &str, params: &impl Wire) -> Self {
        Message::Notification {
            method: method.to_owned(),
            params: Some(params.to_value()),
        }
    }

    /// A successful response carrying `result`.
    pub fn response(id: RequestId, result: &impl Wire) -> Self {
        Message::Response {
            id,
            result: result.to_value(),
        }
    }

    /// An error response.
    pub fn error(id: Option<RequestId>, error: ErrorData) -> Self {
        Message::Error { id, error }
    }
}

/// Decode a message's `params` (absent means no members) as `T`.
///
/// # Errors
/// [`Error::Decode`] when the parameters do not fit `T`.
pub fn params<T: Wire>(params: &Option<Value>) -> Result<T> {
    match params {
        Some(p) => T::from_value(p),
        None => T::from_value(&Value::object()),
    }
}

impl Wire for Message {
    fn to_value(&self) -> Value {
        let base = Obj::new().set("jsonrpc", VERSION);
        match self {
            Message::Request { id, method, params } => base
                .set("id", id.to_value())
                .set("method", method.as_str())
                .opt_value("params", params),
            Message::Notification { method, params } => base
                .set("method", method.as_str())
                .opt_value("params", params),
            Message::Response { id, result } => {
                base.set("id", id.to_value()).set("result", result.clone())
            }
            Message::Error { id, error } => base
                .set("id", id.as_ref().map_or(Value::Null, Wire::to_value))
                .set("error", error.to_value()),
        }
        .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "JSON-RPC message";
        if v.is_array() {
            return Err(Error::decode(W, "batches are not supported"));
        }
        object(v, W)?;
        if opt_string(v, W, "jsonrpc")?.as_deref() != Some(VERSION) {
            return Err(Error::decode(W, "\"jsonrpc\" is not \"2.0\""));
        }
        let id = match v.get("id") {
            None | Some(Value::Null) => None,
            Some(id) => Some(RequestId::from_value(id)?),
        };
        if let Some(method) = opt_string(v, W, "method")? {
            let params = opt_value(v, "params");
            if v.get("id").is_some_and(Value::is_null) {
                return Err(Error::decode(W, "\"id\" is null"));
            }
            return Ok(match id {
                Some(id) => Message::Request { id, method, params },
                None => Message::Notification { method, params },
            });
        }
        if let Some(error) = v.get("error") {
            return Ok(Message::Error {
                id,
                error: ErrorData::from_value(error)?,
            });
        }
        match (id, v.get("result")) {
            (Some(id), Some(result)) => Ok(Message::Response {
                id,
                result: result.clone(),
            }),
            (None, Some(_)) => Err(Error::decode(W, "response without \"id\"")),
            _ => Err(Error::decode(W, "no method, result or error")),
        }
    }
}
