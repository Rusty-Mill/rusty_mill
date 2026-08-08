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
//! of survive intact. Anthropic and Gemini get real translations, in
//! [`translate`] and [`stream`], because the shapes genuinely differ.
//!
//! Gemini differs in one more way worth knowing up front: it names the model
//! and the method in the URL rather than the body, so its address is built per
//! request from what the caller asked for. See [`translate::gemini`].
//!
//! # Requests are buffered, responses are not
//!
//! The request body has to be read to translate it, and chat requests are
//! small JSON. Responses stream: an LLM response is the one thing a client
//! most wants incrementally, and collecting it would turn a token-by-token
//! answer into a long silence followed by a wall of text.

pub mod guard;
mod provider;
mod shape;
pub mod stream;
pub mod tools;
pub mod translate;

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agentgateway_config::{AiBackend, BackendAuth, Policies, RateLimitKind};
use agentgateway_core::{Headers, RateLimiter, Retry, RetryAfter, Rewrite};
use bytes::Bytes;
use futures_util::StreamExt as _;
use http::{HeaderMap, HeaderName, HeaderValue, Request, Response, StatusCode, Uri, header};
use http_body_util::{BodyExt, StreamBody};
use hyper::body::Frame;
use serde_json::Value;

use guard::{Decision, Guard, webhook::Context as GuardContext};
pub use provider::{Provider, ProviderError};
use shape::{Shape, caching::Caching};
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
    /// A route's `requestHeaderModifier` named something HTTP cannot represent.
    #[error(transparent)]
    HeaderModifier(#[from] agentgateway_core::HeaderError),
    /// A route's `urlRewrite` named an authority HTTP cannot represent.
    #[error(transparent)]
    Rewrite(#[from] agentgateway_core::RewriteError),
    /// A route's `localRateLimit` describes a bucket that cannot work.
    #[error(transparent)]
    RateLimit(#[from] agentgateway_core::RateLimitError),
    /// A route's `promptGuard` holds a pattern that will not compile.
    #[error(transparent)]
    Guard(#[from] guard::GuardError),
    /// A `urlRewrite` was asked to act on an endpoint that is not a URL.
    #[error(
        "{at}.urlRewrite: `{endpoint}` is not an absolute URL, so there is nothing to rewrite; \
         check `hostOverride`"
    )]
    Endpoint {
        /// Where in the configuration it came from.
        at: String,
        /// The endpoint that would have been dialled.
        endpoint: String,
    },
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
    /// The route's `requestHeaderModifier`, applied to the provider request.
    ///
    /// The request that leaves here is built by this crate rather than
    /// forwarded, so nothing else would ever apply it: an `ai` route's
    /// modifier used to parse and do nothing.
    request_headers: Option<Headers>,
    /// The provider's endpoint, with the route's `urlRewrite` applied.
    ///
    /// Resolved once, because a rewrite of a fixed address is itself fixed.
    /// For OpenAI and Anthropic this is the whole URL a request goes to. For
    /// Gemini it is the base, and the model and method are appended per
    /// request — see [`Provider::request_url`].
    endpoint: String,
    /// The route's `retry`, applied to the provider request.
    ///
    /// Consumed only by the `host` proxy before: an `ai` route asking for
    /// three attempts got exactly one. See [`LlmBackend::send`].
    retry: Option<Retry>,
    /// The route's `ai` policy, applied to the request body.
    ///
    /// Runs on the OpenAI-shaped body before translation, which is the only
    /// place a rule written once means the same thing for every provider.
    shape: Option<Shape>,
    /// The route's `ai.promptCaching`, applied after translation.
    ///
    /// A cache breakpoint is a provider-specific annotation on a
    /// provider-specific shape, so unlike the rest of the policy it cannot run
    /// on the OpenAI body. See [`shape::caching`].
    caching: Option<Caching>,
    /// The route's `ai.promptGuard`, applied to the prompt and the answer.
    ///
    /// A response rule makes a streamed answer buffer; see [`guard`] for why
    /// that is the only honest option.
    guard: Option<Guard>,
    /// The route's `localRateLimit` entries of `type: tokens`.
    ///
    /// Shared rather than borrowed because a streamed response reports its
    /// usage from inside a `'static` stream, long after `handle` has returned
    /// the body.
    tokens: Option<Arc<RateLimiter>>,
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
    ///
    /// `matched_prefix` is the route's sole `pathPrefix` match, when it has
    /// exactly one; it is what a `urlRewrite.path.prefix` anchors on. See
    /// [`resolve_endpoint`].
    pub fn new(
        backend: &AiBackend,
        policies: &Policies,
        matched_prefix: Option<&str>,
        at: &str,
    ) -> Result<Self, LlmError> {
        let provider = Provider::new(&backend.provider, at)?;
        let model = provider.forced_model().map(str::to_string);
        let endpoint = resolve_endpoint(&provider, policies, matched_prefix, at)?;

        let request_headers = match policies.request_header_modifier.as_ref() {
            Some(modifier) => Some(Headers::new(
                modifier,
                &format!("{at}.requestHeaderModifier"),
            )?),
            None => None,
        };

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

        // Only `type: tokens` here. Request limits are charged before dispatch,
        // where they apply to every backend kind; these need a token count that
        // only exists once the provider has answered.
        let tokens = RateLimiter::for_kind(&policies.local_rate_limit, RateLimitKind::Tokens, at)?
            .map(Arc::new);

        // OpenAI caches long prefixes by itself and takes no configuration for
        // it, so a breakpoint there would be a field nobody reads.
        let caching = Caching::new(
            policies
                .ai
                .as_ref()
                .and_then(|ai| ai.prompt_caching.as_ref())
                .filter(|_| provider.caches_explicitly()),
        );

        // What a moderation rule may borrow: an `openAI` route's own URL and
        // key. A route on anything else lends nothing, because its key is not
        // an OpenAI credential -- see `guard::moderation`.
        let moderation_endpoint = provider.moderation_endpoint();
        let borrowable =
            moderation_endpoint
                .as_deref()
                .map(|endpoint| guard::moderation::Borrowable {
                    endpoint,
                    key: key.as_deref(),
                });

        let guard = Guard::new(
            policies.ai.as_ref().and_then(|ai| ai.prompt_guard.as_ref()),
            borrowable,
            provider.name(),
            &format!("{at}.ai.promptGuard"),
        )?;
        if guard.as_ref().is_some_and(Guard::guards_response) {
            tracing::info!(
                route = %at,
                "a `promptGuard` response rule buffers streamed answers on this route: a \
                 pattern can straddle a chunk boundary, and text already sent cannot be recalled"
            );
        }

        Ok(LlmBackend {
            provider,
            model,
            key,
            request_headers,
            endpoint,
            retry: policies.retry.as_ref().and_then(Retry::new),
            shape: Shape::new(policies.ai.as_ref()),
            caching,
            guard,
            tokens,
            client,
        })
    }

    /// The provider this backend routes to.
    pub fn provider(&self) -> &Provider {
        &self.provider
    }

    /// The URL this backend will POST to.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Send the translated request, retrying if the route asked for it.
    ///
    /// An `ai` request is replayable by construction. The proxy has to decide
    /// whether a body can be buffered, and refuses to retry when it cannot;
    /// here the body was already read and translated before anything could be
    /// sent, so the question never arises and `attempts` always means what it
    /// says.
    ///
    /// What is retried is the same on both paths, from the same policy: a
    /// listed status, or a connect failure. Not a timeout and not any other
    /// transport error — those may have reached the provider and been billed,
    /// with only the response lost, and replaying would pay for the tokens
    /// twice.
    ///
    /// Streaming is unaffected: the decision is made on the response head,
    /// which arrives before the first token, so a retried stream has not
    /// started coming back yet.
    ///
    /// `Err` carries the response to return when no attempt produced one.
    async fn send(
        &self,
        url: &str,
        headers: HeaderMap,
        payload: Bytes,
    ) -> Result<reqwest::Response, Response<LlmBody>> {
        let attempts = self.retry.as_ref().map_or(1, Retry::max_attempts);
        let mut last = None;

        for attempt in 0..attempts {
            if attempt > 0
                && let Some(retry) = &self.retry
                && let Some(wait) = retry.backoff(attempt)
            {
                tokio::time::sleep(wait).await;
            }

            let sending = self
                .client
                .post(url)
                .headers(headers.clone())
                .body(payload.clone());

            let retryable_left = attempt + 1 < attempts;
            match sending.send().await {
                Ok(response) => {
                    let status = response.status().as_u16();
                    let retry_this = retryable_left
                        && self
                            .retry
                            .as_ref()
                            .is_some_and(|retry| retry.retries_status(status));
                    if !retry_this {
                        return Ok(response);
                    }
                    tracing::debug!(
                        provider = self.provider.name(),
                        status,
                        attempt = attempt + 1,
                        "retrying a provider response"
                    );
                    last = Some(response);
                }
                Err(err) => {
                    tracing::warn!(
                        provider = self.provider.name(),
                        %err,
                        "provider request failed"
                    );
                    if !(retryable_left && err.is_connect()) {
                        return Err(error(
                            StatusCode::BAD_GATEWAY,
                            "the model provider could not be reached",
                        ));
                    }
                }
            }
        }

        // Every attempt was retryable and none broke the pattern. The last
        // response is the provider's own answer, and passing it through beats
        // inventing a gateway error that hides what the provider said.
        last.ok_or_else(|| {
            error(
                StatusCode::BAD_GATEWAY,
                "the model provider could not be reached",
            )
        })
    }

    /// Serve one OpenAI-compatible request.
    ///
    /// Generic over the body because the gateway may have buffered it already
    /// — `extAuthz.includeBody` reads it before dispatch — and a signature
    /// naming hyper's stream would force it back into one it cannot be.
    pub async fn handle<B>(&self, request: Request<B>) -> Response<LlmBody>
    where
        B: http_body::Body<Data = Bytes> + Send + 'static,
        B::Error: std::error::Error + Send + Sync + 'static,
    {
        // Snapshotted before the body is taken, because that is the only
        // moment the caller's headers and the claims `jwtAuth` verified both
        // still exist. A `promptGuard` webhook's header expressions read them.
        let context = self.guard_context(&request);

        let collected =
            match http_body_util::Limited::new(request.into_body(), MAX_REQUEST_BYTES as usize)
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

        // `llmRequest.*` is the request as the caller wrote it, which only
        // exists once the body is parsed -- so the context is completed here
        // rather than built complete. Cloning costs an allocation, and only a
        // route with a webhook rule pays it.
        let context = match self.guard.as_ref().is_some_and(Guard::needs_context) {
            true => context.with_llm_request(body.clone()),
            false => context,
        };

        // Before shaping, so a rule scans what the *caller* sent rather than
        // what the operator's own `prompts` added. An operator refusing their
        // own system prompt would be a strange thing to arrange.
        if let Some(guard) = &self.guard {
            match guard.check_request(&mut body, &context).await {
                Decision::Rejected(rejection) => {
                    tracing::info!(
                        provider = self.provider.name(),
                        status = rejection.status,
                        "refusing a request: a `promptGuard` rule matched"
                    );
                    return refuse(&rejection);
                }
                Decision::Masked => tracing::debug!("masked the prompt"),
                Decision::Allowed => {}
            }
        }

        // The route's `ai` policy first: it resolves the name the caller used
        // and fills in what they left out, so everything below sees a request
        // as the operator meant it to arrive.
        if let Some(shape) = &self.shape {
            shape.apply(&mut body);
        }

        // Then the backend's own model, which wins over all of it: it is
        // backend configuration rather than route policy, which makes it the
        // most specific statement about where this traffic goes.
        if let Some(model) = &self.model {
            translate::set_model(&mut body, model);
        }
        let model = translate::requested_model(&body)
            .unwrap_or_default()
            .to_string();
        let streaming = translate::is_streaming(&body);

        let upstream_body = match self.provider.translate_request(&body) {
            Ok(mut body) => {
                if let Some(caching) = &self.caching {
                    caching.apply(&mut body);
                }
                body
            }
            Err(err) => {
                return error(
                    StatusCode::BAD_REQUEST,
                    &format!("the request could not be translated: {err}"),
                );
            }
        };

        // Only Gemini's URL depends on the request, and only after the model
        // has been resolved: the caller's name, the route's aliases and the
        // backend's own override have all had their say by here. Checked
        // rather than interpolated, because it is a client-controlled string
        // going into a URL this gateway signs with its own key.
        let url = match self.provider.needs_model_in_path() {
            false => self.endpoint.clone(),
            true => match translate::gemini::model_path(&model) {
                Ok(path) => self.provider.request_url(&self.endpoint, &path, streaming),
                Err(err) => return error(StatusCode::BAD_REQUEST, &err.to_string()),
            },
        };

        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        for (name, value) in self.provider.auth_headers(self.key.as_deref()) {
            if let (Ok(name), Ok(value)) = (
                HeaderName::try_from(name),
                HeaderValue::try_from(value.as_str()),
            ) {
                headers.insert(name, value);
            }
        }
        // After the provider's own headers, matching the `host` proxy: a route
        // that names a header means it, even one the provider set. Removing
        // `authorization` here is how you stop a key reaching the provider,
        // which is worth being able to say.
        if let Some(modifier) = &self.request_headers {
            modifier.apply(&mut headers);
        }

        // Checked here rather than before dispatch, because what it counts is
        // a number only this backend ever sees. Nothing is charged yet: the
        // cost of a call is not knowable until the provider reports it.
        if let Some(limiter) = &self.tokens
            && let Err(wait) = limiter.admit()
        {
            tracing::info!(
                provider = self.provider.name(),
                retry_after_s = wait.seconds(),
                "refusing a request: the token budget is spent"
            );
            return limited(wait);
        }

        // Serialized once rather than per attempt: the body does not change
        // between them, and a retry should cost a request, not a re-encode.
        let payload = match serde_json::to_vec(&upstream_body) {
            Ok(payload) => Bytes::from(payload),
            Err(err) => {
                return error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("the translated request could not be serialized: {err}"),
                );
            }
        };

        let response = match self.send(&url, headers, payload).await {
            Ok(response) => response,
            Err(unreachable) => return unreachable,
        };

        let status =
            StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);

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

        // A response rule buffers a streamed answer. See `guard` for why a
        // sliding window would be a silent leak rather than a smaller cost.
        let guards_response = self.guard.as_ref().is_some_and(Guard::guards_response);
        match (streaming, guards_response) {
            (true, false) => self.stream(response, status, &model),
            (true, true) => {
                self.guarded_stream(response, status, &model, &context)
                    .await
            }
            (false, _) => self.buffered(response, status, &model, &context).await,
        }
    }

    async fn buffered(
        &self,
        response: reqwest::Response,
        status: StatusCode,
        model: &str,
        context: &GuardContext,
    ) -> Response<LlmBody> {
        let bytes = match response.bytes().await {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::warn!(%err, "reading the provider response failed");
                return error(
                    StatusCode::BAD_GATEWAY,
                    "the provider response was truncated",
                );
            }
        };

        let parsed: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        let translated = self.provider.translate_response(&parsed, now());

        if let Some(usage) = self.provider.usage(&parsed) {
            settle(&self.tokens, self.provider.name(), model, usage, false);
        }

        let mut body = match translated {
            Some(value) => value,
            // OpenAI-compatible: hand back exactly what arrived rather than
            // re-serializing a parse of it, so nothing is lost in the round
            // trip.
            None if !self.guards_response() => return raw(status, bytes),
            None => parsed,
        };

        // Rewriting means re-serializing, so a guarded route gives up the
        // byte-for-byte passthrough. That is the cost of asking for the answer
        // to be inspected, and it only applies where a rule was configured.
        if let Some(guard) = &self.guard {
            let mut text = answer_text(&body).unwrap_or_default();
            match guard.check_text(&mut text, context).await {
                Decision::Rejected(rejection) => {
                    tracing::info!(
                        provider = self.provider.name(),
                        status = rejection.status,
                        "refusing an answer: a `promptGuard` rule matched"
                    );
                    return refuse(&rejection);
                }
                Decision::Masked => set_answer_text(&mut body, text),
                Decision::Allowed => {}
            }
        }

        raw(status, Bytes::from(body.to_string()))
    }

    /// Whether a response rule exists on this route.
    fn guards_response(&self) -> bool {
        self.guard.as_ref().is_some_and(Guard::guards_response)
    }

    /// Serve a streamed answer that has to be inspected before it goes out.
    ///
    /// The whole thing is collected, translated into the chunks a client would
    /// have seen, checked, and then sent. The text is emitted as **one**
    /// content chunk, because after masking it is no longer the text the
    /// provider chunked and pretending otherwise would mean inventing
    /// boundaries.
    ///
    /// A refusal here is an ordinary JSON error rather than an event stream:
    /// nothing has been sent yet, so the client can still be told plainly.
    async fn guarded_stream(
        &self,
        response: reqwest::Response,
        status: StatusCode,
        model: &str,
        context: &GuardContext,
    ) -> Response<LlmBody> {
        let bytes = match response.bytes().await {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::warn!(%err, "reading the provider response failed");
                return error(
                    StatusCode::BAD_GATEWAY,
                    "the provider response was truncated",
                );
            }
        };

        let translating = self.provider.translates_stream();
        let mut parser = EventParser::default();
        let mut translator = ChunkTranslator::new(now());
        let mut chunks: Vec<Value> = Vec::new();
        let mut usage = Usage::default();

        for (event, data) in parser.push(&bytes) {
            if translating {
                chunks.extend(translator.event(&event, &data));
            } else {
                // `data: [DONE]` is not JSON and parses to null. It is the
                // sentinel rather than a chunk, and re-emitting it here would
                // hand a client `data: null` before the real one.
                if !data.is_object() {
                    continue;
                }
                if let Some(seen) = translate::openai_usage(&data) {
                    usage = seen;
                }
                chunks.push(data);
            }
        }
        if translating {
            usage = translator.usage();
        }

        let mut text: String = chunks.iter().filter_map(chunk_text).collect();
        if let Some(guard) = &self.guard {
            match guard.check_text(&mut text, context).await {
                Decision::Rejected(rejection) => {
                    tracing::info!(
                        provider = self.provider.name(),
                        status = rejection.status,
                        "refusing a streamed answer: a `promptGuard` rule matched"
                    );
                    return refuse(&rejection);
                }
                Decision::Masked | Decision::Allowed => {}
            }
        }

        if usage.total() > 0 {
            settle(&self.tokens, self.provider.name(), model, usage, true);
        }

        let mut body = String::new();
        let mut text = Some(text);
        for mut chunk in chunks {
            if chunk_text(&chunk).is_some() {
                match text.take() {
                    // The first content chunk carries all of it.
                    Some(whole) => set_chunk_text(&mut chunk, whole),
                    // The rest carried text that is now in that one.
                    None => continue,
                }
            }
            body.push_str(&stream::frame(&chunk));
        }
        body.push_str(stream::DONE);

        Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, "text/event-stream")
            .header(header::CACHE_CONTROL, "no-cache")
            .body(full(Bytes::from(body)))
            .unwrap_or_else(|_| error(StatusCode::INTERNAL_SERVER_ERROR, ""))
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
        // The stream outlives this call, and the usage it will report is what
        // the bucket has to be charged, so the limiter goes in with it.
        let tokens = self.tokens.clone();

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
                            settle(&tokens, provider, &model, usage, true);
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
                    settle(&tokens, provider, &model, usage, true);
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
}

/// The URL an `ai` route dials, with `urlRewrite` applied.
///
/// `urlRewrite` names parts of the one address the gateway dials. On an `ai`
/// route that address is the provider's endpoint, and it is *built* rather
/// than forwarded — so the policy used to parse and do nothing here, the same
/// way `requestHeaderModifier` did.
///
/// # `authority` replaces the host, `hostOverride` sets the base
///
/// The two compose rather than competing, because they are not the same
/// operation: `hostOverride` is a base URL and carries the scheme, while
/// `authority` replaces only the host and port. Setting both means "this route
/// talks to a self-hosted compatible endpoint, and its egress goes through
/// that address" — which is the `host` proxy's arrangement of a backend
/// address plus a rewrite, in the shape an `ai` backend has.
///
/// That is deliberately not the rule `mcp` follows for `via` versus
/// `urlRewrite.authority`, where one wins. Those two *are* the same operation
/// spelled twice, so one had to.
///
/// # `path` acts on the provider's path, not the client's
///
/// A client's request path never reaches the provider — the endpoint's path is
/// the provider's API, `/v1/chat/completions` or `/v1/messages`. So `full`
/// replaces that, which is how an Azure-style or gateway-mounted deployment is
/// reached, and `prefix` transforms it against the route's own matched prefix,
/// exactly as an `mcp` target's configured path is transformed.
///
/// On a Gemini route the endpoint is only the base, `/v1beta`, since the model
/// and method are appended per request. A rewrite therefore acts on that base
/// and the request's own suffix still follows it, which is what makes pointing
/// a Gemini route at a proxy work.
fn resolve_endpoint(
    provider: &Provider,
    policies: &Policies,
    matched_prefix: Option<&str>,
    at: &str,
) -> Result<String, LlmError> {
    let endpoint = provider.endpoint();
    let Some(rewrite) = policies.url_rewrite.as_ref() else {
        return Ok(endpoint);
    };
    let rewrite = Rewrite::new(rewrite, &format!("{at}.urlRewrite"))?;

    let unrewritable = || LlmError::Endpoint {
        at: at.to_string(),
        endpoint: endpoint.clone(),
    };
    // A rewrite that cannot be applied is a startup failure rather than a
    // silent no-op: the config says the gateway should be dialling somewhere
    // else, and serving traffic to the original address instead is the
    // outcome nobody asked for.
    let uri = Uri::try_from(endpoint.as_str()).map_err(|_| unrewritable())?;
    let scheme = uri.scheme().ok_or_else(unrewritable)?.clone();
    let authority = match rewrite.authority() {
        Some(authority) => authority.clone(),
        None => uri.authority().ok_or_else(unrewritable)?.clone(),
    };

    // A replacement may carry its own query, which is how Azure's mandatory
    // `?api-version=` is set. The provider endpoints have none of their own,
    // so there is nothing to merge with.
    let path_and_query = match rewrite.path(uri.path(), matched_prefix) {
        Some(path) => path,
        None => uri
            .path_and_query()
            .map(|pq| pq.as_str().to_string())
            .unwrap_or_else(|| "/".into()),
    };

    Uri::builder()
        .scheme(scheme)
        .authority(authority)
        .path_and_query(path_and_query)
        .build()
        .map(|uri| uri.to_string())
        .map_err(|_| unrewritable())
}

/// Record what a call cost, and charge it to the route's token budget.
///
/// One place for both, because they read the same number and a path that
/// logged usage without charging it would be a limit that quietly does not
/// apply to streamed responses — which is most of the traffic worth limiting.
fn settle(
    limiter: &Option<Arc<RateLimiter>>,
    provider: &str,
    model: &str,
    usage: Usage,
    streamed: bool,
) {
    record_usage(provider, model, usage, streamed);
    if let Some(limiter) = limiter {
        limiter.charge(usage.total());
    }
}

impl LlmBackend {
    /// What a `promptGuard` webhook's header expressions may read.
    ///
    /// Only built when a route has a webhook rule: snapshotting headers costs
    /// an allocation per request, and most routes have nothing to read them.
    fn guard_context<B>(&self, request: &Request<B>) -> GuardContext {
        if !self.guard.as_ref().is_some_and(Guard::needs_context) {
            return GuardContext::default();
        }
        let headers = request
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                Some((name.as_str().to_string(), value.to_str().ok()?.to_string()))
            })
            .collect();
        // The claims `jwtAuth` verified, which the gateway put here. A caller
        // cannot forge them by sending a header.
        let claims = request
            .extensions()
            .get::<TokenClaims>()
            .map(|c| c.0.clone());
        GuardContext::new(headers, claims)
    }
}

/// The claims a verified token carried, as the gateway stores them.
///
/// Declared here rather than depended on, so this crate does not take the MCP
/// crate for one type it only reads.
#[derive(Debug, Clone)]
pub struct TokenClaims(pub Value);

/// The assistant's own text in an OpenAI-shaped response.
fn answer_text(body: &Value) -> Option<String> {
    body.get("choices")?
        .get(0)?
        .get("message")?
        .get("content")?
        .as_str()
        .map(str::to_string)
}

/// Put rewritten text back where [`answer_text`] found it.
fn set_answer_text(body: &mut Value, text: String) {
    if let Some(content) = body
        .get_mut("choices")
        .and_then(|choices| choices.get_mut(0))
        .and_then(|choice| choice.get_mut("message"))
        .and_then(|message| message.get_mut("content"))
    {
        *content = Value::String(text);
    }
}

/// The text a streamed chunk carries, if it carries any.
fn chunk_text(chunk: &Value) -> Option<&str> {
    chunk
        .get("choices")?
        .get(0)?
        .get("delta")?
        .get("content")?
        .as_str()
}

/// Put rewritten text back where [`chunk_text`] found it.
fn set_chunk_text(chunk: &mut Value, text: String) {
    if let Some(content) = chunk
        .get_mut("choices")
        .and_then(|choices| choices.get_mut(0))
        .and_then(|choice| choice.get_mut("delta"))
        .and_then(|delta| delta.get_mut("content"))
    {
        *content = Value::String(text);
    }
}

/// Answer with what a `promptGuard` rule said to answer with.
///
/// A rule with no body of its own gets this crate's OpenAI error envelope, so
/// a client's existing error handling still works rather than seeing an empty
/// response it has to guess about.
fn refuse(rejection: &guard::Rejection) -> Response<LlmBody> {
    let status = StatusCode::from_u16(rejection.status).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY);
    let mut response = match &rejection.body {
        Some(body) => raw(status, Bytes::from(body.clone())),
        None => error(status, "this request was refused by a content rule"),
    };
    if let Some(modifier) = &rejection.headers
        && let Ok(headers) = Headers::new(modifier, "promptGuard.rejection.headers")
    {
        headers.apply(response.headers_mut());
    }
    response
}

/// Refuse a request whose route has spent its token budget.
fn limited(wait: RetryAfter) -> Response<LlmBody> {
    let mut response = error(
        StatusCode::TOO_MANY_REQUESTS,
        "this route's token budget is spent",
    );
    if let Ok(value) = HeaderValue::try_from(wait.seconds().to_string()) {
        response.headers_mut().insert(header::RETRY_AFTER, value);
    }
    response
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

#[cfg(test)]
mod tests {
    use agentgateway_config::{AiProvider, AiProviderParams, PathRewrite, UrlRewrite};

    use super::*;

    fn provider(kind: &str, host: Option<&str>) -> Provider {
        let params = AiProviderParams {
            model: None,
            host_override: host.map(str::to_string),
        };
        let provider = match kind {
            "openai" => AiProvider::OpenAi(params),
            _ => AiProvider::Anthropic(params),
        };
        Provider::new(&provider, "route[0]").expect("should resolve")
    }

    fn policies(authority: Option<&str>, path: Option<PathRewrite>) -> Policies {
        Policies {
            url_rewrite: Some(UrlRewrite {
                authority: authority.map(str::to_string),
                path,
            }),
            ..Default::default()
        }
    }

    fn endpoint(
        provider: &Provider,
        policies: &Policies,
        matched_prefix: Option<&str>,
    ) -> Result<String, LlmError> {
        resolve_endpoint(provider, policies, matched_prefix, "route[0]")
    }

    #[test]
    fn no_rewrite_leaves_the_provider_endpoint_alone() {
        let provider = provider("openai", None);
        assert_eq!(
            endpoint(&provider, &Policies::default(), None).expect("should resolve"),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn an_authority_replaces_the_host_and_keeps_the_scheme_and_path() {
        // The scheme stays because `authority` names a host and port and
        // nothing else; `hostOverride` is where a scheme is chosen.
        let provider = provider("openai", None);
        assert_eq!(
            endpoint(&provider, &policies(Some("llm.internal:8443"), None), None)
                .expect("should resolve"),
            "https://llm.internal:8443/v1/chat/completions"
        );
    }

    #[test]
    fn an_authority_composes_with_a_host_override_rather_than_losing_to_it() {
        // They are not the same operation: `hostOverride` is a base URL and
        // carries the scheme, `authority` replaces only host and port. A route
        // reaching a self-hosted endpoint over http through an egress address
        // needs both to mean something.
        let provider = provider("openai", Some("http://vllm.internal:8000"));
        assert_eq!(
            endpoint(&provider, &policies(Some("egress:15001"), None), None)
                .expect("should resolve"),
            "http://egress:15001/v1/chat/completions"
        );
    }

    #[test]
    fn a_full_path_rewrite_replaces_the_providers_api_path() {
        // The shape an Azure-style or gateway-mounted deployment needs.
        let provider = provider("openai", Some("https://acme.openai.azure.com"));
        let rewrite = policies(
            None,
            Some(PathRewrite::Full(
                "/openai/deployments/gpt4o/chat/completions".into(),
            )),
        );
        assert_eq!(
            endpoint(&provider, &rewrite, None).expect("should resolve"),
            "https://acme.openai.azure.com/openai/deployments/gpt4o/chat/completions"
        );
    }

    #[test]
    fn a_prefix_rewrite_transforms_the_providers_path_against_the_matched_prefix() {
        // `/v1/chat/completions` with `/v1` matched and replaced by `/openai/v1`.
        let provider = provider("openai", Some("http://compat.internal:8080"));
        let rewrite = policies(None, Some(PathRewrite::Prefix("/openai/v1".into())));
        assert_eq!(
            endpoint(&provider, &rewrite, Some("/v1")).expect("should resolve"),
            "http://compat.internal:8080/openai/v1/chat/completions"
        );
    }

    #[test]
    fn a_prefix_rewrite_with_no_prefix_to_anchor_on_leaves_the_path_alone() {
        // A route matching on zero or several prefixes cannot say which one a
        // request replaced. Leaving the path beats anchoring on a guess.
        let provider = provider("openai", None);
        let rewrite = policies(None, Some(PathRewrite::Prefix("/openai".into())));
        assert_eq!(
            endpoint(&provider, &rewrite, None).expect("should resolve"),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn anthropics_own_path_is_what_gets_rewritten() {
        // Not OpenAI's: the path being rewritten is the provider's API, and
        // the two providers do not share one.
        let provider = provider("anthropic", None);
        let rewrite = policies(None, Some(PathRewrite::Prefix("/proxy/v1".into())));
        assert_eq!(
            endpoint(&provider, &rewrite, Some("/v1")).expect("should resolve"),
            "https://api.anthropic.com/proxy/v1/messages"
        );
    }

    #[test]
    fn a_full_rewrite_may_carry_a_query() {
        // Azure rejects a request without `api-version`, and the endpoint has
        // nowhere else to put one.
        let provider = provider("openai", Some("https://acme.openai.azure.com"));
        let rewrite = policies(
            None,
            Some(PathRewrite::Full(
                "/openai/deployments/gpt4o/chat/completions?api-version=2024-02-01".into(),
            )),
        );
        assert_eq!(
            endpoint(&provider, &rewrite, None).expect("should resolve"),
            "https://acme.openai.azure.com/openai/deployments/gpt4o/chat/completions\
             ?api-version=2024-02-01"
        );
    }

    #[test]
    fn an_authority_and_a_path_rewrite_both_apply_together() {
        let provider = provider("openai", None);
        let rewrite = policies(
            Some("egress:15001"),
            Some(PathRewrite::Full("/upstream/chat".into())),
        );
        assert_eq!(
            endpoint(&provider, &rewrite, None).expect("should resolve"),
            "https://egress:15001/upstream/chat"
        );
    }

    #[test]
    fn a_rewrite_on_an_endpoint_that_is_not_a_url_fails_at_startup() {
        // Serving traffic to the original address, when the config says to
        // dial somewhere else, is the outcome nobody asked for.
        let provider = provider("openai", Some("not a url"));
        let err = endpoint(&provider, &policies(Some("other:443"), None), None)
            .expect_err("should not resolve");
        assert!(err.to_string().contains("hostOverride"), "got: {err}");
    }

    #[test]
    fn an_authority_carrying_a_credential_fails_at_startup() {
        let provider = provider("openai", None);
        let err = endpoint(&provider, &policies(Some("user:secret@host"), None), None)
            .expect_err("should not resolve");
        assert!(err.to_string().contains("backendAuth"), "got: {err}");
    }
}
