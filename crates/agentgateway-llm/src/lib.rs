//! The LLM gateway: an OpenAI-compatible front end over several providers.
//!
//! An `ai` backend terminates the OpenAI chat-completions API and speaks
//! whatever the configured provider speaks, so a client switches provider by
//! editing the gateway's configuration rather than its own code.
//!
//! # What is and is not translated
//!
//! For an OpenAI-compatible provider the body is forwarded essentially
//! unchanged — only `model` is overridden and credentials swapped — so tool
//! definitions, `response_format` and anything else this crate has never heard
//! of survive intact. Anthropic gets a real translation, in [`translate`] and
//! [`stream`], because the shapes genuinely differ.
//!
//! # Requests are buffered, responses are not
//!
//! The request body has to be read to translate it, and chat requests are
//! small JSON. Responses stream: an LLM response is the one thing a client
//! most wants incrementally, and collecting it would turn a token-by-token
//! answer into a long silence followed by a wall of text.

mod provider;
pub mod stream;
pub mod translate;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agentgateway_config::{AiBackend, BackendAuth, Policies};
use bytes::Bytes;
use futures_util::StreamExt as _;
use http::{HeaderValue, Request, Response, StatusCode, header};
use http_body_util::{BodyExt, StreamBody};
use hyper::body::{Frame, Incoming};
use serde_json::Value;

pub use provider::{Provider, ProviderError};
use stream::{ChunkTranslator, EventParser};
use translate::Usage;

/// Largest request body accepted on an `ai` route.
///
/// A chat request is small JSON. The bound exists because the body must be
/// buffered to be translated, and an unbounded buffer on a public endpoint is
/// a memory limit waiting to be found.
pub const MAX_REQUEST_BYTES: u64 = 4 * 1024 * 1024;

/// Failure to build an LLM backend.
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    /// The provider is not served by this build.
    #[error(transparent)]
    Provider(#[from] ProviderError),
}

/// The body an LLM response carries.
pub type LlmBody = http_body_util::combinators::BoxBody<Bytes, std::convert::Infallible>;

/// An `ai` backend.
pub struct LlmBackend {
    provider: Provider,
    /// Model forced by configuration, overriding whatever the caller asked for.
    model: Option<String>,
    /// Credential presented to the provider.
    key: Option<String>,
    client: reqwest::Client,
}

impl std::fmt::Debug for LlmBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmBackend")
            .field("provider", &self.provider.name())
            .field("model", &self.model)
            // Never the key.
            .finish_non_exhaustive()
    }
}

impl LlmBackend {
    /// Build a backend for a route's `ai` configuration.
    pub fn new(backend: &AiBackend, policies: &Policies, at: &str) -> Result<Self, LlmError> {
        let provider = Provider::new(&backend.provider, at)?;
        let model = provider.forced_model().map(str::to_string);

        // `passthrough` is meaningless here: a provider API key is not the
        // caller's bearer token, and forwarding one as the other would send a
        // user's credential to OpenAI.
        let key = match policies.backend_auth.as_ref() {
            Some(BackendAuth::Key(key)) => Some(key.clone()),
            _ => None,
        };
        if key.is_none() {
            tracing::warn!(
                route = %at,
                "no `backendAuth.key` on an `ai` route; the provider will almost certainly \
                 reject every request"
            );
        }

        let timeout = policies
            .timeout
            .as_ref()
            .and_then(|t| t.backend_request_timeout)
            .map(Duration::from)
            // LLM calls are slow by nature. `reqwest`'s no-timeout default
            // would let a hung provider hold a connection forever, and 30s
            // would cut off a long completion that was working fine.
            .unwrap_or(Duration::from_secs(300));

        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .unwrap_or_default();

        Ok(LlmBackend {
            provider,
            model,
            key,
            client,
        })
    }

    /// The provider this backend routes to.
    pub fn provider(&self) -> &Provider {
        &self.provider
    }

    /// Serve one OpenAI-compatible request.
    pub async fn handle(&self, request: Request<Incoming>) -> Response<LlmBody> {
        let collected = match http_body_util::Limited::new(
            request.into_body(),
            MAX_REQUEST_BYTES as usize,
        )
        .collect()
        .await
        {
            Ok(body) => body.to_bytes(),
            Err(_) => {
                return error(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "the request body is too large to translate",
                );
            }
        };

        let mut body: Value = match serde_json::from_slice(&collected) {
            Ok(body) => body,
            Err(err) => {
                return error(
                    StatusCode::BAD_REQUEST,
                    &format!("the request body is not valid JSON: {err}"),
                );
            }
        };

        // Configuration wins over the caller: an operator pinning a model is
        // making a routing decision, not suggesting one.
        if let Some(model) = &self.model {
            translate::set_model(&mut body, model);
        }
        let model = translate::requested_model(&body)
            .unwrap_or_default()
            .to_string();
        let streaming = translate::is_streaming(&body);

        let upstream_body = match self.provider.translate_request(&body) {
            Ok(body) => body,
            Err(err) => {
                return error(
                    StatusCode::BAD_REQUEST,
                    &format!("the request could not be translated: {err}"),
                );
            }
        };

        let mut upstream = self
            .client
            .post(self.provider.endpoint())
            .header(header::CONTENT_TYPE, "application/json")
            .json(&upstream_body);
        for (name, value) in self.provider.auth_headers(self.key.as_deref()) {
            upstream = upstream.header(name, value);
        }

        let response = match upstream.send().await {
            Ok(response) => response,
            Err(err) => {
                tracing::warn!(provider = self.provider.name(), %err, "provider request failed");
                return error(
                    StatusCode::BAD_GATEWAY,
                    "the model provider could not be reached",
                );
            }
        };

        let status = StatusCode::from_u16(response.status().as_u16())
            .unwrap_or(StatusCode::BAD_GATEWAY);

        // An error from the provider is passed through as-is rather than
        // reshaped: the message is the useful part, and a gateway that
        // rewrites "invalid api key" into "bad gateway" costs an afternoon.
        if !status.is_success() {
            let body = response.bytes().await.unwrap_or_default();
            tracing::warn!(
                provider = self.provider.name(),
                %status,
                "provider returned an error"
            );
            return raw(status, body);
        }

        if streaming {
            self.stream(response, status, &model)
        } else {
            self.buffered(response, status, &model).await
        }
    }

    async fn buffered(
        &self,
        response: reqwest::Response,
        status: StatusCode,
        model: &str,
    ) -> Response<LlmBody> {
        let bytes = match response.bytes().await {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::warn!(%err, "reading the provider response failed");
                return error(StatusCode::BAD_GATEWAY, "the provider response was truncated");
            }
        };

        let parsed: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        let translated = self.provider.translate_response(&parsed, now());

        if let Some(usage) = self.provider.usage(&parsed) {
            self.record(model, usage, false);
        }

        let body = match translated {
            Some(value) => Bytes::from(value.to_string()),
            // OpenAI-compatible: hand back exactly what arrived rather than
            // re-serializing a parse of it, so nothing is lost in the round
            // trip.
            None => bytes,
        };
        raw(status, body)
    }

    fn stream(
        &self,
        response: reqwest::Response,
        status: StatusCode,
        model: &str,
    ) -> Response<LlmBody> {
        let translating = self.provider.translates_stream();
        let provider = self.provider.name();
        let model = model.to_string();

        let mut parser = EventParser::default();
        let mut translator = ChunkTranslator::new(now());
        let mut reported = false;

        let upstream = response.bytes_stream();
        let stream = async_stream::stream! {
            let mut upstream = std::pin::pin!(upstream);

            while let Some(chunk) = upstream.next().await {
                let chunk = match chunk {
                    Ok(chunk) => chunk,
                    Err(err) => {
                        tracing::warn!(provider, %err, "provider stream failed");
                        break;
                    }
                };

                if !translating {
                    // OpenAI-compatible: the frames are already what the
                    // client expects, so they go straight through and usage is
                    // read from the trailing chunk if the provider sends one.
                    for (_, data) in parser.push(&chunk) {
                        if let Some(usage) = translate::openai_usage(&data)
                            && !reported
                        {
                            reported = true;
                            record_usage(provider, &model, usage, true);
                        }
                    }
                    yield Ok(Frame::data(chunk));
                    continue;
                }

                for (event, data) in parser.push(&chunk) {
                    for chunk in translator.event(&event, &data) {
                        yield Ok(Frame::data(Bytes::from(stream::frame(&chunk))));
                    }
                }
            }

            if translating {
                // The sentinel is not a chunk, so the translator does not emit
                // it; without it an OpenAI client waits for a stream that has
                // already ended.
                yield Ok(Frame::data(Bytes::from_static(stream::DONE.as_bytes())));
                let usage = translator.usage();
                if usage.total() > 0 {
                    record_usage(provider, &model, usage, true);
                }
            }
        };

        let body = BodyExt::boxed(StreamBody::new(stream));
        Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, "text/event-stream")
            .header(header::CACHE_CONTROL, "no-cache")
            .body(body)
            .unwrap_or_else(|_| error(StatusCode::INTERNAL_SERVER_ERROR, ""))
    }

    fn record(&self, model: &str, usage: Usage, streamed: bool) {
        record_usage(self.provider.name(), model, usage, streamed);
    }
}

/// Report token usage.
///
/// A structured log line rather than a metric: token counts are per-request
/// and a request's model is unbounded cardinality for a metric label, so this
/// is the shape a log pipeline can aggregate without a metrics backend being
/// taken down by a client inventing model names.
fn record_usage(provider: &str, model: &str, usage: Usage, streamed: bool) {
    tracing::info!(
        provider,
        model,
        streamed,
        prompt_tokens = usage.prompt,
        completion_tokens = usage.completion,
        total_tokens = usage.total(),
        "llm request completed"
    );
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

fn raw(status: StatusCode, body: Bytes) -> Response<LlmBody> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(full(body))
        .unwrap_or_else(|_| {
            let mut fallback = Response::new(full(Bytes::new()));
            *fallback.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
            fallback
        })
}

fn error(status: StatusCode, message: &str) -> Response<LlmBody> {
    // OpenAI's error envelope, so a client's existing error handling works.
    let body = serde_json::json!({
        "error": {"message": message, "type": "gateway_error"}
    });
    let mut response = Response::new(full(Bytes::from(body.to_string())));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
}

fn full(bytes: Bytes) -> LlmBody {
    // `BodyExt::boxed` and `Either`'s inherent one both apply here, so name
    // the trait method explicitly.
    BodyExt::boxed(http_body_util::Full::new(bytes).map_err(|never| match never {}))
}
