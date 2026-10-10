//! Multi-round-trip requests (MRTR, 2026-07-28): instead of calling the
//! client mid-request, a stateless server answers `tools/call`,
//! `prompts/get` or `resources/read` with an [`InputRequiredResult`]; the
//! client fulfils the requests and retries with `inputResponses` and the
//! echoed `requestState`.

use crate::codec::{field, object, opt_string, opt_value, string, Obj, Wire};
use crate::page::ResultType;
use crate::prompt::GetPromptResult;
use crate::resource::ReadResourceResult;
use crate::task::CreateTaskResult;
use crate::tool::CallToolResult;
use crate::{Error, Result};
use rusty_json::Value;
use std::collections::BTreeMap;

const INPUT_REQUIRED: &str = ResultType::INPUT_REQUIRED;

/// A request the server wants fulfilled: elicitation, sampling or roots.
/// The parameters stay raw JSON (the elicitation schema is opaque by
/// decision; see [`ElicitParams`] for the typed elicitation envelope).
#[derive(Clone, Debug, PartialEq)]
pub struct InputRequest {
    /// `elicitation/create`, `sampling/createMessage` or `roots/list`.
    pub method: String,
    /// The request's parameters.
    pub params: Option<Value>,
}

impl InputRequest {
    /// The methods a server may put in an [`InputRequiredResult`].
    pub const METHODS: [&'static str; 3] =
        ["elicitation/create", "sampling/createMessage", "roots/list"];

    /// An elicitation request.
    pub fn elicitation(params: &ElicitParams) -> Self {
        Self {
            method: "elicitation/create".to_owned(),
            params: Some(params.to_value()),
        }
    }
}

impl Wire for InputRequest {
    fn to_value(&self) -> Value {
        Obj::new()
            .set("method", self.method.as_str())
            .opt_value("params", &self.params)
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "InputRequest";
        object(v, W)?;
        let method = string(v, W, "method")?;
        if !Self::METHODS.contains(&method.as_str()) {
            return Err(Error::decode(W, format!("unsupported method {method:?}")));
        }
        Ok(Self {
            method,
            params: opt_value(v, "params"),
        })
    }
}

/// The requests of one round, by the key the client answers under.
pub type InputRequests = BTreeMap<String, InputRequest>;

fn encode_requests(requests: &InputRequests) -> Value {
    let mut out = Value::object();
    for (key, request) in requests {
        out.insert(key.as_str(), request.to_value());
    }
    out
}

fn decode_requests(v: &Value) -> Result<InputRequests> {
    v.as_object()
        .ok_or_else(|| Error::decode("inputRequests", "not an object"))?
        .iter()
        .map(|(k, r)| Ok((k.clone(), InputRequest::from_value(r)?)))
        .collect()
}

/// The result that asks for more input before the request can finish.
#[derive(Clone, Debug, PartialEq)]
pub struct InputRequiredResult {
    /// What to fulfil.
    pub input_requests: Option<InputRequests>,
    /// Opaque state to echo back. It is untrusted on return: a server that
    /// keeps anything meaningful in it must authenticate it.
    pub request_state: Option<String>,
    /// Result `_meta`, raw JSON.
    pub meta: Option<Value>,
}

impl InputRequiredResult {
    /// Ask for input.
    pub fn from_input_requests(requests: InputRequests) -> Self {
        Self {
            input_requests: Some(requests),
            request_state: None,
            meta: None,
        }
    }

    /// Hand back state only (a retry later, no questions).
    pub fn from_request_state(state: impl Into<String>) -> Self {
        Self {
            input_requests: None,
            request_state: Some(state.into()),
            meta: None,
        }
    }
}

impl Wire for InputRequiredResult {
    fn to_value(&self) -> Value {
        Obj::new()
            .set("resultType", INPUT_REQUIRED)
            .opt_value(
                "inputRequests",
                &self.input_requests.as_ref().map(encode_requests),
            )
            .opt("requestState", self.request_state.clone())
            .opt_value("_meta", &self.meta)
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "InputRequiredResult";
        object(v, W)?;
        if opt_string(v, W, "resultType")?.as_deref() != Some(INPUT_REQUIRED) {
            return Err(Error::decode(W, "resultType is not \"input_required\""));
        }
        let input_requests = opt_value(v, "inputRequests")
            .as_ref()
            .map(decode_requests)
            .transpose()?;
        let request_state = opt_string(v, W, "requestState")?;
        if input_requests.is_none() && request_state.is_none() {
            return Err(Error::decode(
                W,
                "needs at least one of inputRequests or requestState",
            ));
        }
        Ok(Self {
            input_requests,
            request_state,
            meta: opt_value(v, "_meta"),
        })
    }
}

fn result_type(v: &Value) -> Option<&str> {
    v.get("resultType").and_then(Value::as_str)
}

/// What `prompts/get` or `resources/read` answers: the result, or a
/// request for more input.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome<T> {
    /// The finished result.
    Complete(T),
    /// More input is needed first.
    InputRequired(InputRequiredResult),
}

/// The answer to `prompts/get`.
pub type GetPromptResponse = Outcome<GetPromptResult>;
/// The answer to `resources/read`.
pub type ReadResourceResponse = Outcome<ReadResourceResult>;

impl<T: Wire> Wire for Outcome<T> {
    fn to_value(&self) -> Value {
        match self {
            Outcome::Complete(r) => r.to_value(),
            Outcome::InputRequired(r) => r.to_value(),
        }
    }

    fn from_value(v: &Value) -> Result<Self> {
        if result_type(v) == Some(INPUT_REQUIRED) {
            InputRequiredResult::from_value(v).map(Outcome::InputRequired)
        } else {
            T::from_value(v).map(Outcome::Complete)
        }
    }
}

/// What `tools/call` answers: the result, a request for input, or a task
/// that continues the work.
#[derive(Clone, Debug, PartialEq)]
pub enum CallToolResponse {
    /// The finished result.
    Complete(CallToolResult),
    /// More input is needed first.
    InputRequired(InputRequiredResult),
    /// The work continues as a task.
    Task(CreateTaskResult),
}

impl Wire for CallToolResponse {
    fn to_value(&self) -> Value {
        match self {
            CallToolResponse::Complete(r) => r.to_value(),
            CallToolResponse::InputRequired(r) => r.to_value(),
            CallToolResponse::Task(r) => r.to_value(),
        }
    }

    fn from_value(v: &Value) -> Result<Self> {
        match result_type(v) {
            Some(INPUT_REQUIRED) => InputRequiredResult::from_value(v).map(Self::InputRequired),
            Some(ResultType::TASK) => CreateTaskResult::from_value(v).map(Self::Task),
            _ => CallToolResult::from_value(v).map(Self::Complete),
        }
    }
}

/// What the user did with an elicitation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElicitAction {
    /// They filled it in.
    Accept,
    /// They said no.
    Decline,
    /// They dismissed it.
    Cancel,
}

/// An elicitation: ask the user for input.
#[derive(Clone, Debug, PartialEq)]
pub enum ElicitParams {
    /// A form. `requested_schema` is a restricted JSON Schema, kept raw.
    Form {
        /// What to tell the user.
        message: String,
        /// The fields wanted.
        requested_schema: Value,
        /// `_meta`, raw JSON.
        meta: Option<Value>,
    },
    /// Send the user to a URL.
    Url {
        /// What to tell the user.
        message: String,
        /// Where to send them.
        url: String,
        /// Correlates the out-of-band flow.
        elicitation_id: String,
        /// `_meta`, raw JSON.
        meta: Option<Value>,
    },
}

impl Wire for ElicitParams {
    fn to_value(&self) -> Value {
        match self {
            ElicitParams::Form {
                message,
                requested_schema,
                meta,
            } => Obj::new()
                .set("mode", "form")
                .set("message", message.as_str())
                .set("requestedSchema", requested_schema.clone())
                .opt_value("_meta", meta),
            ElicitParams::Url {
                message,
                url,
                elicitation_id,
                meta,
            } => Obj::new()
                .set("mode", "url")
                .set("message", message.as_str())
                .set("url", url.as_str())
                .set("elicitationId", elicitation_id.as_str())
                .opt_value("_meta", meta),
        }
        .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "ElicitParams";
        object(v, W)?;
        let message = string(v, W, "message")?;
        let meta = opt_value(v, "_meta");
        // A missing `mode` is a form, as older peers send.
        match opt_string(v, W, "mode")?.as_deref().unwrap_or("form") {
            "form" => Ok(ElicitParams::Form {
                message,
                requested_schema: object(field(v, W, "requestedSchema")?, "requestedSchema")?
                    .clone(),
                meta,
            }),
            "url" => Ok(ElicitParams::Url {
                message,
                url: string(v, W, "url")?,
                elicitation_id: string(v, W, "elicitationId")?,
                meta,
            }),
            other => Err(Error::decode(W, format!("unknown mode {other:?}"))),
        }
    }
}

/// The client's answer to an elicitation.
#[derive(Clone, Debug, PartialEq)]
pub struct ElicitResult {
    /// What the user did.
    pub action: ElicitAction,
    /// The form values, when accepted.
    pub content: Option<Value>,
    /// Result `_meta`, raw JSON.
    pub meta: Option<Value>,
}

impl Wire for ElicitResult {
    fn to_value(&self) -> Value {
        Obj::new()
            .set(
                "action",
                match self.action {
                    ElicitAction::Accept => "accept",
                    ElicitAction::Decline => "decline",
                    ElicitAction::Cancel => "cancel",
                },
            )
            .opt_value("content", &self.content)
            .opt_value("_meta", &self.meta)
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "ElicitResult";
        object(v, W)?;
        let action = match string(v, W, "action")?.as_str() {
            "accept" => ElicitAction::Accept,
            "decline" => ElicitAction::Decline,
            "cancel" => ElicitAction::Cancel,
            other => return Err(Error::decode(W, format!("unknown action {other:?}"))),
        };
        Ok(Self {
            action,
            content: opt_value(v, "content"),
            meta: opt_value(v, "_meta"),
        })
    }
}
