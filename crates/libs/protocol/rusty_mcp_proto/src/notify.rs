//! Notifications both sides send: cancellation and progress.

use crate::codec::{field, object, opt_string, opt_value, Obj, Wire};
use crate::rpc::RequestId;
use crate::{Error, Result};
use rusty_json::Value;

/// Method names for these notifications.
pub mod method {
    /// Abandon an in-flight request.
    pub const CANCELLED: &str = "notifications/cancelled";
    /// Report progress of an in-flight request.
    pub const PROGRESS: &str = "notifications/progress";
}

/// The token a requester puts in `_meta.progressToken` to ask for progress.
/// It is a number or a string, like a request id.
pub type ProgressToken = RequestId;

/// `notifications/cancelled`: the sender no longer wants the result.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CancelledParams {
    /// The request being cancelled; absent for task-based cancellation.
    pub request_id: Option<RequestId>,
    /// Why, for logs.
    pub reason: Option<String>,
    /// `_meta`, raw JSON.
    pub meta: Option<Value>,
}

impl Wire for CancelledParams {
    fn to_value(&self) -> Value {
        Obj::new()
            .opt_value("requestId", &self.request_id.as_ref().map(Wire::to_value))
            .opt("reason", self.reason.clone())
            .opt_value("_meta", &self.meta)
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "CancelledParams";
        object(v, W)?;
        Ok(Self {
            request_id: opt_value(v, "requestId")
                .as_ref()
                .map(RequestId::from_value)
                .transpose()?,
            reason: opt_string(v, W, "reason")?,
            meta: opt_value(v, "_meta"),
        })
    }
}

/// `notifications/progress`: how far an in-flight request has got.
#[derive(Clone, Debug, PartialEq)]
pub struct ProgressParams {
    /// The token the request carried.
    pub progress_token: ProgressToken,
    /// Work done so far; it must increase with every notification.
    pub progress: f64,
    /// Total work, when known.
    pub total: Option<f64>,
    /// A status line.
    pub message: Option<String>,
    /// `_meta`, raw JSON.
    pub meta: Option<Value>,
}

impl Wire for ProgressParams {
    fn to_value(&self) -> Value {
        Obj::new()
            .set("progressToken", self.progress_token.to_value())
            .set("progress", self.progress)
            .opt("total", self.total)
            .opt("message", self.message.clone())
            .opt_value("_meta", &self.meta)
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "ProgressParams";
        object(v, W)?;
        let number = |name: &'static str| -> Result<Option<f64>> {
            match v.get(name) {
                None | Some(Value::Null) => Ok(None),
                Some(n) => n
                    .as_f64()
                    .map(Some)
                    .ok_or_else(|| Error::decode(W, format!("{name:?} is not a number"))),
            }
        };
        Ok(Self {
            progress_token: ProgressToken::from_value(field(v, W, "progressToken")?)?,
            progress: number("progress")?
                .ok_or_else(|| Error::decode(W, "missing \"progress\""))?,
            total: number("total")?,
            message: opt_string(v, W, "message")?,
            meta: opt_value(v, "_meta"),
        })
    }
}
