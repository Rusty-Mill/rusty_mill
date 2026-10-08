//! Multi-round-trip input (2026-07-28): a handler that needs the user's
//! input answers [`ToolOutcome::Ask`] instead of a result; the client
//! gathers the input and calls again with `inputResponses` (and the state the
//! handler left, if any), so the server keeps nothing between the rounds.

use rusty_mcp_proto::{
    CallToolParams, CallToolResult, ElicitParams, ElicitResult, ErrorCode, ErrorData, InputRequest,
    InputRequests, Wire,
};

/// What a tool handler decided: it is done, or it needs input first.
#[derive(Clone, Debug)]
pub enum ToolOutcome {
    /// The finished result.
    Done(CallToolResult),
    /// Ask the client for input, then be called again.
    Ask(Ask),
}

impl From<CallToolResult> for ToolOutcome {
    fn from(result: CallToolResult) -> Self {
        Self::Done(result)
    }
}

/// A request for input, built with [`Ask::elicit`].
#[derive(Clone, Debug, Default)]
pub struct Ask {
    pub(crate) requests: InputRequests,
    #[cfg_attr(not(feature = "request-state"), allow(dead_code))]
    pub(crate) state: Option<Vec<u8>>,
}

impl Ask {
    /// An empty request; add questions with [`Ask::elicit`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask the user `params`; the client answers under `key`, which
    /// [`answer`] reads back on the retry.
    #[must_use]
    pub fn elicit(mut self, key: impl Into<String>, params: &ElicitParams) -> Self {
        self.requests
            .insert(key.into(), InputRequest::elicitation(params));
        self
    }

    /// Keep `state` for the retry, which reads it with
    /// [`CallContext::request_state`](crate::CallContext::request_state).
    /// The server seals it (an HMAC bound to this tool and an expiry), so the
    /// client can neither read-modify it undetected nor use it on another
    /// tool. It is not secret from the client, and it can be replayed until
    /// it expires: keep in it only what the retry re-checks.
    #[cfg(feature = "request-state")]
    #[must_use]
    pub fn with_state(mut self, state: impl Into<Vec<u8>>) -> Self {
        self.state = Some(state.into());
        self
    }
}

/// The client's answer to the elicitation asked under `key`, from a retried
/// call; `None` if the retry carries none.
///
/// # Errors
/// `-32602` if the answer is not a well-formed elicitation result.
pub fn answer(params: &CallToolParams, key: &str) -> Result<Option<ElicitResult>, ErrorData> {
    let Some(raw) = params.input_responses.as_ref().and_then(|r| r.get(key)) else {
        return Ok(None);
    };
    ElicitResult::from_value(raw).map(Some).map_err(|e| {
        ErrorData::new(
            ErrorCode::INVALID_PARAMS,
            format!("inputResponses[{key:?}]: {e}"),
        )
    })
}
