//! `$/setTrace` and `$/logTrace`: the client asking the server to adjust how
//! verbose its trace logging is, and the server relaying trace messages back.
//!
//! Distinct from [`super::window::LogMessageParams`] (`window/logMessage`,
//! ordinary user-facing logging shown in the client's output channel):
//! `$/logTrace` is specifically for protocol-level tracing, gated by the
//! verbosity most recently set via `$/setTrace`, and typically rendered in a
//! separate "trace" output channel.

use serde::{Deserialize, Serialize};

/// How verbose trace logging should be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TraceValue {
    /// No `$/logTrace` notifications.
    Off,
    /// Just the message, no `verbose` detail.
    Messages,
    /// The message plus any `verbose` detail.
    Verbose,
}

/// Parameters of the `$/setTrace` notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetTraceParams {
    /// The trace verbosity the client wants from now on.
    pub value: TraceValue,
}

/// Parameters of the `$/logTrace` notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogTraceParams {
    /// The trace message.
    pub message: String,
    /// Additional detail, only sent when the client's trace value is
    /// [`TraceValue::Verbose`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verbose: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn trace_value_encodes_as_lowercase_string() {
        assert_eq!(serde_json::to_value(TraceValue::Off).unwrap(), json!("off"));
        assert_eq!(
            serde_json::to_value(TraceValue::Messages).unwrap(),
            json!("messages")
        );
        assert_eq!(
            serde_json::to_value(TraceValue::Verbose).unwrap(),
            json!("verbose")
        );
    }

    #[test]
    fn set_trace_params_round_trips() {
        let params = SetTraceParams {
            value: TraceValue::Verbose,
        };
        let value = serde_json::to_value(params).unwrap();
        assert_eq!(value, json!({"value": "verbose"}));
        let back: SetTraceParams = serde_json::from_value(value).unwrap();
        assert_eq!(back, params);
    }

    #[test]
    fn log_trace_params_omits_absent_verbose() {
        let params = LogTraceParams {
            message: "received request".to_owned(),
            verbose: None,
        };
        let value = serde_json::to_value(&params).unwrap();
        assert_eq!(value, json!({"message": "received request"}));
    }
}
