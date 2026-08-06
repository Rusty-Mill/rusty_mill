//! [`MockModel`] — a scripted model for tests and offline examples.

use adk_core::{Args, Content, FunctionCall, Part, Result, Role};
use async_trait::async_trait;
use futures::stream::{self, BoxStream};
use std::sync::Mutex;

use crate::model::Model;
use crate::request::{LlmRequest, LlmResponse};

/// What a [`MockModel`] does for one call.
enum Script {
    /// Return this response.
    Response(LlmResponse),
    /// Return these text chunks as a stream, then an aggregated final response.
    Stream(Vec<String>),
}

/// A model that replays a scripted sequence of responses.
///
/// Each call consumes the next script entry. Once the script is exhausted the
/// model falls back to echoing the last user message, which keeps a test from
/// hanging on an unexpected extra turn — an exhausted script produces a
/// harmless terminal response rather than an error that masks the real
/// assertion.
pub struct MockModel {
    name: String,
    scripts: Mutex<std::collections::VecDeque<Script>>,
    calls: Mutex<Vec<LlmRequest>>,
}

impl MockModel {
    /// Builds a model with an empty script.
    pub fn new() -> Self {
        Self {
            name: "mock".to_string(),
            scripts: Mutex::new(Default::default()),
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Builds a named model with an empty script.
    pub fn echo(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::new()
        }
    }

    /// Queues a text response.
    pub fn push_text(self, text: impl Into<String>) -> Self {
        self.push_response(LlmResponse::text(text))
    }

    /// Queues a response.
    pub fn push_response(self, response: LlmResponse) -> Self {
        self.scripts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push_back(Script::Response(response));
        self
    }

    /// Queues a request to call `name` with `args`.
    pub fn push_function_call(self, name: impl Into<String>, args: Args) -> Self {
        self.push_response(LlmResponse::from_content(Content::new(
            Role::Model,
            vec![Part::FunctionCall(FunctionCall::new(name, args))],
        )))
    }

    /// Queues a request to call `name` with JSON arguments.
    ///
    /// # Panics
    ///
    /// Panics if `args` is not a JSON object. This is a test helper, and a
    /// non-object argument set is a mistake in the test itself.
    pub fn push_call_json(self, name: impl Into<String>, args: serde_json::Value) -> Self {
        let args = match args {
            serde_json::Value::Object(map) => map,
            other => panic!("tool arguments must be a JSON object, got {other}"),
        };
        self.push_function_call(name, args)
    }

    /// Queues a streamed response, delivered as the given text chunks.
    pub fn push_stream<I, S>(self, chunks: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.scripts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push_back(Script::Stream(chunks.into_iter().map(Into::into).collect()));
        self
    }

    /// Every request this model has received, in order.
    pub fn recorded_requests(&self) -> Vec<LlmRequest> {
        self.calls.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// How many times the model has been called.
    pub fn call_count(&self) -> usize {
        self.calls.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Whether every queued script entry has been consumed.
    pub fn is_exhausted(&self) -> bool {
        self.scripts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
    }

    fn record(&self, request: &LlmRequest) {
        self.calls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(request.clone());
    }

    fn next_script(&self) -> Option<Script> {
        self.scripts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pop_front()
    }

    fn fallback(request: &LlmRequest) -> LlmResponse {
        let last = request
            .contents
            .iter()
            .rev()
            .find(|c| c.role == Role::User)
            .map(Content::text)
            .unwrap_or_default();
        LlmResponse::text(format!("[mock] {last}"))
    }
}

impl Default for MockModel {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Model for MockModel {
    fn name(&self) -> &str {
        &self.name
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    async fn generate_content(&self, request: LlmRequest) -> Result<LlmResponse> {
        self.record(&request);
        Ok(match self.next_script() {
            Some(Script::Response(r)) => r,
            Some(Script::Stream(chunks)) => LlmResponse::text(chunks.concat()),
            None => Self::fallback(&request),
        })
    }

    fn generate_content_stream<'a>(
        &'a self,
        request: LlmRequest,
    ) -> BoxStream<'a, Result<LlmResponse>> {
        self.record(&request);
        let responses: Vec<Result<LlmResponse>> = match self.next_script() {
            Some(Script::Stream(chunks)) => {
                let mut out: Vec<Result<LlmResponse>> = chunks
                    .iter()
                    .map(|c| Ok(LlmResponse::chunk(c.clone())))
                    .collect();
                out.push(Ok(LlmResponse {
                    turn_complete: true,
                    finish_reason: Some("STOP".into()),
                    ..Default::default()
                }));
                out
            }
            Some(Script::Response(r)) => vec![Ok(r)],
            None => vec![Ok(Self::fallback(&request))],
        };
        Box::pin(stream::iter(responses))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn scripted_responses_are_returned_in_order() {
        let model = MockModel::new().push_text("first").push_text("second");
        assert_eq!(
            model
                .generate_content(LlmRequest::new("mock"))
                .await
                .unwrap()
                .text_content(),
            "first"
        );
        assert_eq!(
            model
                .generate_content(LlmRequest::new("mock"))
                .await
                .unwrap()
                .text_content(),
            "second"
        );
        assert!(model.is_exhausted());
    }

    #[tokio::test]
    async fn an_exhausted_script_echoes_the_last_user_message() {
        let model = MockModel::new();
        let request = LlmRequest::new("mock").push_content(Content::user_text("ping"));
        let response = model.generate_content(request).await.unwrap();
        assert_eq!(response.text_content(), "[mock] ping");
    }

    #[tokio::test]
    async fn function_calls_can_be_scripted() {
        let model = MockModel::new().push_call_json("get_weather", json!({"city": "Paris"}));
        let response = model
            .generate_content(LlmRequest::new("mock"))
            .await
            .unwrap();
        let calls = response.content.as_ref().unwrap().function_calls();
        assert_eq!(calls[0].name, "get_weather");
        assert_eq!(calls[0].args["city"], "Paris");
    }

    #[tokio::test]
    async fn requests_are_recorded_for_assertions() {
        let model = MockModel::new().push_text("ok");
        model
            .generate_content(LlmRequest::new("mock").with_system_instruction("be terse"))
            .await
            .unwrap();
        assert_eq!(model.call_count(), 1);
        assert_eq!(
            model.recorded_requests()[0].system_instruction.as_deref(),
            Some("be terse")
        );
    }

    #[tokio::test]
    async fn a_streamed_script_collapses_when_called_non_streaming() {
        let model = MockModel::new().push_stream(["a", "b"]);
        let response = model
            .generate_content(LlmRequest::new("mock"))
            .await
            .unwrap();
        assert_eq!(response.text_content(), "ab");
    }
}
