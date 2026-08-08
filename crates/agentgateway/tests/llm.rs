//! End-to-end tests for the `ai` backend.
//!
//! A mock provider stands in for OpenAI and Anthropic. It records the request
//! body it actually received, which is what makes the translation assertions
//! meaningful: the question is not "did the gateway answer" but "did the
//! provider get the shape its API requires".

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use agentgateway::{Gateway, serve};
use agentgateway_config::Config;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

mod common;
use common::free_port;

/// What the provider saw.
#[derive(Default)]
struct Seen {
    body: Option<Value>,
    headers: Vec<(String, String)>,
    path: String,
}

/// A provider that answers with `reply`, recording what it was asked.
async fn provider(reply: Value, sse: Option<String>) -> (u16, Arc<Mutex<Seen>>) {
    use axum::{Router, extract::Request, routing::any};

    let port = free_port().await;
    let seen = Arc::new(Mutex::new(Seen::default()));
    let recorder = Arc::clone(&seen);

    let app = Router::new().fallback(any(move |request: Request| {
        let recorder = Arc::clone(&recorder);
        let reply = reply.clone();
        let sse = sse.clone();
        async move {
            let path = request.uri().path().to_string();
            let headers: Vec<(String, String)> = request
                .headers()
                .iter()
                .map(|(k, v)| {
                    (
                        k.as_str().to_string(),
                        v.to_str().unwrap_or_default().to_string(),
                    )
                })
                .collect();

            let bytes = axum::body::to_bytes(request.into_body(), 1 << 20)
                .await
                .unwrap_or_default();

            if let Ok(mut seen) = recorder.lock() {
                seen.body = serde_json::from_slice(&bytes).ok();
                seen.headers = headers;
                seen.path = path;
            }

            match sse {
                Some(stream) => axum::response::Response::builder()
                    .header("content-type", "text/event-stream")
                    .body(axum::body::Body::from(stream))
                    .expect("response should build"),
                None => axum::response::Response::builder()
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(reply.to_string()))
                    .expect("response should build"),
            }
        }
    }));

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .expect("provider should bind");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    (port, seen)
}

/// Boot a gateway with one `ai` route pointed at the mock provider.
async fn start(
    provider_kind: &str,
    provider_port: u16,
    extra: &str,
) -> (String, CancellationToken) {
    start_with(provider_kind, provider_port, extra, "").await
}

/// The same, with extra route policies spliced in.
async fn start_with(
    provider_kind: &str,
    provider_port: u16,
    extra: &str,
    extra_policies: &str,
) -> (String, CancellationToken) {
    let port = free_port().await;
    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - routes:
          - name: llm
            policies:
{extra_policies}
              backendAuth:
                key: test-key
            backends:
              - ai:
                  provider:
                    {provider_kind}:
                      hostOverride: "http://127.0.0.1:{provider_port}"
{extra}
"#
    );

    let config = Config::from_yaml(&yaml).expect("config should parse");
    config.validate().expect("config should validate");
    let gateway = Gateway::build(&config, None)
        .await
        .expect("gateway should build");

    let shutdown = CancellationToken::new();
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("should parse");
    let _serving = serve::run_with_shutdown(gateway, vec![addr], shutdown.clone())
        .await
        .expect("gateway should bind");

    (
        format!("http://127.0.0.1:{port}/v1/chat/completions"),
        shutdown,
    )
}

fn chat_request() -> Value {
    json!({
        "model": "gpt-4o",
        "messages": [
            {"role": "system", "content": "Be brief."},
            {"role": "user", "content": "Hello"},
        ],
    })
}

async fn post(url: &str, body: &Value) -> reqwest::Response {
    reqwest::Client::new()
        .post(url)
        .json(body)
        .send()
        .await
        .expect("request should reach the gateway")
}

fn openai_reply() -> Value {
    json!({
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "created": 1,
        "model": "gpt-4o",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "Hi"},
                     "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 8, "completion_tokens": 2, "total_tokens": 10},
    })
}

fn anthropic_reply() -> Value {
    json!({
        "id": "msg_1",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4",
        "content": [{"type": "text", "text": "Hi"}],
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 8, "output_tokens": 2},
    })
}

#[tokio::test]
async fn an_openai_request_reaches_the_provider_unchanged() {
    // The point of not building a typed model: fields this gateway has never
    // heard of must survive the trip.
    let (provider_port, seen) = provider(openai_reply(), None).await;
    let (url, shutdown) = start("openAI", provider_port, "").await;

    let mut body = chat_request();
    body["tools"] = json!([{"type": "function", "function": {"name": "lookup"}}]);
    body["response_format"] = json!({"type": "json_object"});

    let response = post(&url, &body).await;
    assert_eq!(response.status(), 200);

    let seen = seen.lock().expect("lock");
    let sent = seen.body.clone().expect("the provider should see a body");
    assert_eq!(
        sent["tools"], body["tools"],
        "tool definitions must survive"
    );
    assert_eq!(sent["response_format"], body["response_format"]);
    assert_eq!(seen.path, "/v1/chat/completions");

    shutdown.cancel();
}

#[tokio::test]
async fn the_openai_credential_is_a_bearer_token() {
    let (provider_port, seen) = provider(openai_reply(), None).await;
    let (url, shutdown) = start("openAI", provider_port, "").await;

    post(&url, &chat_request()).await;

    let seen = seen.lock().expect("lock");
    let auth = seen
        .headers
        .iter()
        .find(|(name, _)| name == "authorization")
        .map(|(_, value)| value.clone())
        .unwrap_or_default();
    assert_eq!(auth, "Bearer test-key");

    shutdown.cancel();
}

#[tokio::test]
async fn an_anthropic_request_is_translated_to_the_messages_api() {
    let (provider_port, seen) = provider(anthropic_reply(), None).await;
    let (url, shutdown) = start("anthropic", provider_port, "").await;

    post(&url, &chat_request()).await;

    let seen = seen.lock().expect("lock");
    let sent = seen.body.clone().expect("the provider should see a body");

    assert_eq!(seen.path, "/v1/messages", "a different endpoint entirely");
    assert_eq!(
        sent["system"], "Be brief.",
        "the system prompt is a field, not a turn"
    );
    assert_eq!(
        sent["messages"].as_array().expect("messages").len(),
        1,
        "only the user turn remains in the message list"
    );
    assert!(
        sent["max_tokens"].is_u64(),
        "Anthropic rejects a request without max_tokens: {sent}"
    );

    let key = seen.headers.iter().find(|(n, _)| n == "x-api-key");
    assert!(
        key.is_some(),
        "Anthropic uses x-api-key, not a bearer token"
    );
    assert!(
        seen.headers.iter().any(|(n, _)| n == "anthropic-version"),
        "the version header is required on every request"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn an_anthropic_response_comes_back_in_openai_shape() {
    // The whole promise of the gateway: the client's existing OpenAI parsing
    // works against a different provider.
    let (provider_port, _) = provider(anthropic_reply(), None).await;
    let (url, shutdown) = start("anthropic", provider_port, "").await;

    let body: Value = post(&url, &chat_request())
        .await
        .json()
        .await
        .expect("should be JSON");

    assert_eq!(body["object"], "chat.completion");
    assert_eq!(body["choices"][0]["message"]["content"], "Hi");
    assert_eq!(body["choices"][0]["message"]["role"], "assistant");
    assert_eq!(body["choices"][0]["finish_reason"], "stop");
    assert_eq!(body["usage"]["prompt_tokens"], 8);
    assert_eq!(
        body["usage"]["total_tokens"], 10,
        "Anthropic sends no total"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_configured_model_overrides_the_callers() {
    // An operator pinning a model is making a routing decision, not a
    // suggestion.
    let (provider_port, seen) = provider(anthropic_reply(), None).await;
    let (url, shutdown) = start(
        "anthropic",
        provider_port,
        "                      model: claude-sonnet-4",
    )
    .await;

    post(&url, &chat_request()).await;

    let seen = seen.lock().expect("lock");
    let sent = seen.body.clone().expect("body");
    assert_eq!(
        sent["model"], "claude-sonnet-4",
        "the caller asked for gpt-4o and configuration wins"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_provider_error_is_passed_through_rather_than_reshaped() {
    // "invalid api key" is the useful part; rewriting it as "bad gateway"
    // costs an afternoon.
    let port = free_port().await;
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .expect("should bind");
    tokio::spawn(async move {
        let app = axum::Router::new().fallback(axum::routing::any(|| async {
            (
                axum::http::StatusCode::UNAUTHORIZED,
                axum::Json(json!({"error": {"message": "invalid api key"}})),
            )
        }));
        let _ = axum::serve(listener, app).await;
    });

    let (url, shutdown) = start("openAI", port, "").await;
    let response = post(&url, &chat_request()).await;

    assert_eq!(response.status(), 401, "the provider's status is preserved");
    let body: Value = response.json().await.expect("should be JSON");
    assert_eq!(body["error"]["message"], "invalid api key");

    shutdown.cancel();
}

#[tokio::test]
async fn a_body_that_is_not_json_is_refused_in_openai_error_shape() {
    let (provider_port, _) = provider(openai_reply(), None).await;
    let (url, shutdown) = start("openAI", provider_port, "").await;

    let response = reqwest::Client::new()
        .post(&url)
        .header("content-type", "application/json")
        .body("not json")
        .send()
        .await
        .expect("should reach the gateway");

    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.expect("should be JSON");
    assert!(
        body["error"]["message"].is_string(),
        "a client's existing OpenAI error handling should work: {body}"
    );

    shutdown.cancel();
}

/// Anthropic's streaming events for a two-token answer.
fn anthropic_stream() -> String {
    [
        "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_s\",\"model\":\"claude-sonnet-4\",\"usage\":{\"input_tokens\":5}}}\n\n",
        "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hel\"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"lo\"}}\n\n",
        "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":2}}\n\n",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    ]
    .concat()
}

#[tokio::test]
async fn an_anthropic_stream_is_reframed_as_openai_chunks() {
    let (provider_port, _) = provider(Value::Null, Some(anthropic_stream())).await;
    let (url, shutdown) = start("anthropic", provider_port, "").await;

    let mut body = chat_request();
    body["stream"] = json!(true);

    let response = post(&url, &body).await;
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("text/event-stream")
    );

    let text = response.text().await.expect("should read the stream");

    let chunks: Vec<Value> = text
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter(|data| *data != "[DONE]")
        .filter_map(|data| serde_json::from_str(data).ok())
        .collect();

    assert!(!chunks.is_empty(), "no chunks parsed from: {text}");
    assert_eq!(chunks[0]["object"], "chat.completion.chunk");
    assert_eq!(
        chunks[0]["choices"][0]["delta"]["role"], "assistant",
        "OpenAI announces the role in its own first chunk"
    );

    let content: String = chunks
        .iter()
        .filter_map(|c| c["choices"][0]["delta"]["content"].as_str())
        .collect();
    assert_eq!(content, "Hello", "the deltas reassemble into the answer");

    let last = chunks.last().expect("a final chunk");
    assert_eq!(last["choices"][0]["finish_reason"], "stop");
    assert!(
        text.trim_end().ends_with("data: [DONE]"),
        "without the sentinel an OpenAI client waits forever: {text}"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn an_openai_stream_passes_straight_through() {
    // Already the right frames; re-framing them would only risk corrupting
    // fields this gateway does not model.
    let frames = "data: {\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hi\"}}]}\n\ndata: [DONE]\n\n";
    let (provider_port, _) = provider(Value::Null, Some(frames.to_string())).await;
    let (url, shutdown) = start("openAI", provider_port, "").await;

    let mut body = chat_request();
    body["stream"] = json!(true);

    let text = post(&url, &body)
        .await
        .text()
        .await
        .expect("should read the stream");

    assert_eq!(text, frames, "the frames should arrive byte-identical");

    shutdown.cancel();
}

#[tokio::test]
async fn an_unimplemented_provider_fails_at_startup() {
    let port = free_port().await;
    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - routes:
          - backends:
              - ai:
                  provider:
                    bedrock:
                      model: anthropic.claude-v2
"#
    );

    let config = Config::from_yaml(&yaml).expect("should parse");
    let err = Gateway::build(&config, None)
        .await
        .map(|_| ())
        .expect_err("bedrock is not served by this build");
    assert!(
        err.to_string().contains("bedrock"),
        "the error should name the provider: {err}"
    );
}

#[tokio::test]
async fn a_response_modifier_reaches_an_ai_completion() {
    // An `ai` backend answers from inside the gateway rather than through the
    // `host` proxy, so it never saw the modifier before.
    let (provider_port, _seen) = provider(openai_reply(), None).await;
    let (url, shutdown) = start_with(
        "openAI",
        provider_port,
        "                      model: gpt-4o",
        "              responseHeaderModifier:\n                set:\n                  x-served-by: rusty",
    )
    .await;

    let response = post(&url, &chat_request()).await;
    assert!(response.status().is_success(), "{}", response.status());

    let headers: Vec<(String, String)> = response
        .headers()
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_string(),
                v.to_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    assert!(
        headers
            .iter()
            .any(|(k, v)| k == "x-served-by" && v == "rusty"),
        "saw {headers:?}"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_request_modifier_reaches_the_model_provider() {
    // The request an `ai` route sends is built by the LLM crate rather than
    // forwarded through the `host` proxy, so this modifier used to parse and
    // then do nothing at all.
    let (provider_port, seen) = provider(openai_reply(), None).await;
    let (url, shutdown) = start_with(
        "openAI",
        provider_port,
        "",
        "              requestHeaderModifier:\n                set:\n                  x-tenant: acme\n                add:\n                  x-scope: models",
    )
    .await;

    let response = post(&url, &chat_request()).await;
    assert!(response.status().is_success(), "{}", response.status());

    let seen = seen.lock().expect("lock");
    assert!(
        seen.headers
            .iter()
            .any(|(k, v)| k == "x-tenant" && v == "acme"),
        "saw {:?}",
        seen.headers
    );
    assert!(
        seen.headers
            .iter()
            .any(|(k, v)| k == "x-scope" && v == "models"),
        "saw {:?}",
        seen.headers
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_request_modifier_can_take_back_the_provider_credential() {
    // The modifier runs after the provider's own headers, matching the `host`
    // proxy's ordering with `backendAuth`: a route that names a header means
    // it. Removing `authorization` is how an operator says "this route does
    // not hand a key to the provider", which is worth being able to say.
    let (provider_port, seen) = provider(openai_reply(), None).await;
    let (url, shutdown) = start_with(
        "openAI",
        provider_port,
        "",
        "              requestHeaderModifier:\n                remove: [authorization]",
    )
    .await;

    post(&url, &chat_request()).await;

    let seen = seen.lock().expect("lock");
    assert!(
        !seen.headers.iter().any(|(k, _)| k == "authorization"),
        "saw {:?}",
        seen.headers
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_request_modifier_can_replace_a_header_the_provider_set() {
    // `set` on `authorization` replaces the credential rather than appending a
    // second one, which is what a comma-joined value would do to a provider.
    let (provider_port, seen) = provider(anthropic_reply(), None).await;
    let (url, shutdown) = start_with(
        "anthropic",
        provider_port,
        "",
        "              requestHeaderModifier:\n                set:\n                  x-api-key: from-config",
    )
    .await;

    post(&url, &chat_request()).await;

    let seen = seen.lock().expect("lock");
    let keys: Vec<&(String, String)> = seen
        .headers
        .iter()
        .filter(|(k, _)| k == "x-api-key")
        .collect();
    assert_eq!(keys.len(), 1, "saw {:?}", seen.headers);
    assert_eq!(
        keys[0].1, "from-config",
        "the route's value wins over the provider's `backendAuth.key`"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn add_appends_to_a_header_the_provider_set_rather_than_replacing_it() {
    // The distinction between `set` and `add` on a name that is already taken.
    // Unlike the MCP path, which has one value per name to give the transport,
    // an `ai` request is a real `HeaderMap` and can carry both field lines.
    let (provider_port, seen) = provider(anthropic_reply(), None).await;
    let (url, shutdown) = start_with(
        "anthropic",
        provider_port,
        "",
        "              requestHeaderModifier:\n                add:\n                  x-api-key: second",
    )
    .await;

    post(&url, &chat_request()).await;

    let seen = seen.lock().expect("lock");
    let keys: Vec<&str> = seen
        .headers
        .iter()
        .filter(|(k, _)| k == "x-api-key")
        .map(|(_, v)| v.as_str())
        .collect();
    assert_eq!(keys, vec!["test-key", "second"], "saw {:?}", seen.headers);

    shutdown.cancel();
}

#[tokio::test]
async fn a_request_modifier_http_rejects_fails_an_ai_route_at_startup() {
    // Rather than dropping the header at runtime on every call, where nobody
    // would see it.
    let port = free_port().await;
    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - routes:
          - policies:
              requestHeaderModifier:
                set:
                  "not a name": v
              backendAuth:
                key: test-key
            backends:
              - ai:
                  provider:
                    openAI: {{}}
"#
    );

    let config = Config::from_yaml(&yaml).expect("should parse");
    let err = Gateway::build(&config, None)
        .await
        .map(|_| ())
        .expect_err("an unrepresentable header name should not start");
    assert!(
        err.to_string().contains("header name"),
        "the error should say what was wrong: {err}"
    );
}

#[tokio::test]
async fn a_full_path_rewrite_moves_the_provider_endpoint() {
    // The path the provider is asked for is its own API path, not the
    // client's -- a client's path never reaches it. This is the shape an
    // Azure-style or gateway-mounted deployment needs.
    let (provider_port, seen) = provider(openai_reply(), None).await;
    let (url, shutdown) = start_with(
        "openAI",
        provider_port,
        "",
        "              urlRewrite:\n                path:\n                  full: /openai/deployments/gpt4o/chat/completions",
    )
    .await;

    let response = post(&url, &chat_request()).await;
    assert!(response.status().is_success(), "{}", response.status());

    let seen = seen.lock().expect("lock");
    assert_eq!(seen.path, "/openai/deployments/gpt4o/chat/completions");

    shutdown.cancel();
}

#[tokio::test]
async fn a_prefix_rewrite_transforms_the_providers_own_path() {
    // `/v1/chat/completions` with the route's matched `/v1` replaced.
    let (provider_port, seen) = provider(openai_reply(), None).await;
    let port = free_port().await;
    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - routes:
          - name: llm
            matches:
              - path:
                  pathPrefix: /v1
            policies:
              urlRewrite:
                path:
                  prefix: /openai/v1
              backendAuth:
                key: test-key
            backends:
              - ai:
                  provider:
                    openAI:
                      hostOverride: "http://127.0.0.1:{provider_port}"
"#
    );

    let config = Config::from_yaml(&yaml).expect("config should parse");
    assert!(
        config.lint().is_empty(),
        "one `pathPrefix` is exactly what a prefix rewrite needs: {:?}",
        config.lint()
    );
    let gateway = Gateway::build(&config, None)
        .await
        .expect("gateway should build");
    let shutdown = CancellationToken::new();
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("should parse");
    let _serving = serve::run_with_shutdown(gateway, vec![addr], shutdown.clone())
        .await
        .expect("gateway should bind");

    let response = post(
        &format!("http://127.0.0.1:{port}/v1/chat/completions"),
        &chat_request(),
    )
    .await;
    assert!(response.status().is_success(), "{}", response.status());

    let seen = seen.lock().expect("lock");
    assert_eq!(seen.path, "/openai/v1/chat/completions");

    shutdown.cancel();
}

#[tokio::test]
async fn an_authority_rewrite_redirects_the_provider_request() {
    // Two providers listening; the route names one via `hostOverride` and the
    // rewrite sends the request to the other instead.
    let (intended, intended_seen) = provider(openai_reply(), None).await;
    let (elsewhere, elsewhere_seen) = provider(openai_reply(), None).await;
    let (url, shutdown) = start_with(
        "openAI",
        intended,
        "",
        &format!("              urlRewrite:\n                authority: \"127.0.0.1:{elsewhere}\""),
    )
    .await;

    let response = post(&url, &chat_request()).await;
    assert!(response.status().is_success(), "{}", response.status());

    assert!(
        elsewhere_seen.lock().expect("lock").body.is_some(),
        "the rewritten authority is where the request should land"
    );
    assert!(
        intended_seen.lock().expect("lock").body.is_none(),
        "and the original address should see nothing"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_url_rewrite_that_cannot_be_applied_fails_an_ai_route_at_startup() {
    // Serving traffic to the original address, when the config says to dial
    // somewhere else, is the outcome nobody asked for.
    let port = free_port().await;
    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - routes:
          - policies:
              urlRewrite:
                authority: "not a host"
              backendAuth:
                key: test-key
            backends:
              - ai:
                  provider:
                    openAI: {{}}
"#
    );

    let config = Config::from_yaml(&yaml).expect("should parse");
    let err = Gateway::build(&config, None)
        .await
        .map(|_| ())
        .expect_err("an unusable authority should not start");
    assert!(
        err.to_string().contains("not a host"),
        "the error should name the offending value: {err}"
    );
}

#[tokio::test]
async fn a_host_override_carrying_a_credential_fails_at_startup() {
    // It would be sent on every request from a place nobody reads, and logged
    // with the endpoint besides. `backendAuth.key` is where one belongs.
    let port = free_port().await;
    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - routes:
          - backends:
              - ai:
                  provider:
                    openAI:
                      hostOverride: "https://user:secret@compat.internal"
"#
    );

    let config = Config::from_yaml(&yaml).expect("should parse");
    let err = Gateway::build(&config, None)
        .await
        .map(|_| ())
        .expect_err("userinfo in an upstream address should not start");
    assert!(
        err.to_string().contains("backendAuth"),
        "the error should say where a credential belongs: {err}"
    );
}

/// A provider that answers `status` for its first `failures` calls, then
/// succeeds, counting every call it received.
async fn flaky_provider(failures: usize, status: u16) -> (u16, Arc<AtomicUsize>) {
    use axum::{Router, extract::Request, routing::any};

    let port = free_port().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);

    let app = Router::new().fallback(any(move |request: Request| {
        let counter = Arc::clone(&counter);
        async move {
            let _ = axum::body::to_bytes(request.into_body(), 1 << 20).await;
            let seen = counter.fetch_add(1, Ordering::Relaxed);
            if seen < failures {
                return axum::response::Response::builder()
                    .status(status)
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        json!({"error": {"message": "try again"}}).to_string(),
                    ))
                    .expect("response should build");
            }
            axum::response::Response::builder()
                .header("content-type", "application/json")
                .body(axum::body::Body::from(openai_reply().to_string()))
                .expect("response should build")
        }
    }));

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .expect("provider should bind");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    (port, calls)
}

/// Retry two listed statuses, with no backoff so the tests stay fast.
const RETRY: &str =
    "              retry:\n                attempts: 2\n                codes: [429, 503]";

#[tokio::test]
async fn a_listed_status_is_retried_on_an_ai_route() {
    // An `ai` route asking for three attempts used to get exactly one: the
    // policy was consumed only by the `host` proxy.
    let (provider_port, calls) = flaky_provider(2, 429).await;
    let (url, shutdown) = start_with("openAI", provider_port, "", RETRY).await;

    let response = post(&url, &chat_request()).await;
    assert!(response.status().is_success(), "{}", response.status());
    assert_eq!(
        calls.load(Ordering::Relaxed),
        3,
        "two retries after the first try is three attempts"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_status_the_route_did_not_list_is_not_retried() {
    // Nothing is retried on status unless `codes` names it: the provider
    // answered, so it certainly saw the request.
    let (provider_port, calls) = flaky_provider(2, 500).await;
    let (url, shutdown) = start_with("openAI", provider_port, "", RETRY).await;

    let response = post(&url, &chat_request()).await;
    assert_eq!(response.status(), 500);
    assert_eq!(calls.load(Ordering::Relaxed), 1);

    shutdown.cancel();
}

#[tokio::test]
async fn a_route_without_a_retry_policy_makes_one_attempt() {
    let (provider_port, calls) = flaky_provider(2, 429).await;
    let (url, shutdown) = start("openAI", provider_port, "").await;

    let response = post(&url, &chat_request()).await;
    assert_eq!(response.status(), 429);
    assert_eq!(calls.load(Ordering::Relaxed), 1);

    shutdown.cancel();
}

#[tokio::test]
async fn exhausting_the_attempts_returns_the_providers_own_last_answer() {
    // Rather than a gateway error that hides what the provider said. The
    // message is the useful part.
    let (provider_port, calls) = flaky_provider(99, 503).await;
    let (url, shutdown) = start_with("openAI", provider_port, "", RETRY).await;

    let response = post(&url, &chat_request()).await;
    assert_eq!(response.status(), 503);
    let body: Value = response.json().await.expect("should be JSON");
    assert_eq!(
        body["error"]["message"], "try again",
        "the provider's own body survives: {body}"
    );
    assert_eq!(calls.load(Ordering::Relaxed), 3);

    shutdown.cancel();
}

#[tokio::test]
async fn a_connect_failure_is_retried_but_a_provider_that_never_comes_up_still_fails() {
    // A connect failure never reached the provider, so replaying it cannot
    // duplicate work -- the one transport error that is safe to repeat.
    let dead = free_port().await;
    let (url, shutdown) = start_with("openAI", dead, "", RETRY).await;

    let response = post(&url, &chat_request()).await;
    assert_eq!(
        response.status(),
        502,
        "the gateway is fine, the provider is not"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn the_retried_request_still_carries_its_body_and_credential() {
    // The body is serialized once and replayed, so an attempt after the first
    // must not arrive empty or unauthenticated.
    let (provider_port, seen) = provider(openai_reply(), None).await;
    let (url, shutdown) = start_with("openAI", provider_port, "", RETRY).await;

    post(&url, &chat_request()).await;

    let seen = seen.lock().expect("lock");
    let sent = seen.body.clone().expect("the provider should see a body");
    assert_eq!(sent["messages"][1]["content"], "Hello");
    assert!(
        seen.headers
            .iter()
            .any(|(k, v)| k == "content-type" && v.starts_with("application/json")),
        "saw {:?}",
        seen.headers
    );
    assert!(
        seen.headers
            .iter()
            .any(|(k, v)| k == "authorization" && v == "Bearer test-key"),
        "saw {:?}",
        seen.headers
    );

    shutdown.cancel();
}

/// A tiny token budget: the first call fits, the second is refused.
const TOKEN_LIMIT: &str = "              localRateLimit:\n                - maxTokens: 10\n                  tokensPerFill: 10\n                  fillInterval: 60s\n                  type: tokens";

#[tokio::test]
async fn a_token_budget_is_charged_by_what_the_provider_reported() {
    // The mock reply declares 10 total tokens, which is the whole bucket. The
    // first call is admitted -- nothing is charged up front, because the cost
    // is not knowable until the provider answers -- and the second is refused.
    let (provider_port, _seen) = provider(openai_reply(), None).await;
    let (url, shutdown) = start_with("openAI", provider_port, "", TOKEN_LIMIT).await;

    let first = post(&url, &chat_request()).await;
    assert!(first.status().is_success(), "{}", first.status());

    let second = post(&url, &chat_request()).await;
    assert_eq!(second.status(), 429);
    let retry_after = second
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .expect("a refusal should say when to come back");
    assert!(
        (1..=60).contains(&retry_after),
        "got Retry-After: {retry_after}"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_route_without_a_token_budget_is_never_refused() {
    let (provider_port, _seen) = provider(openai_reply(), None).await;
    let (url, shutdown) = start("openAI", provider_port, "").await;

    for _ in 0..5 {
        let response = post(&url, &chat_request()).await;
        assert!(response.status().is_success(), "{}", response.status());
    }

    shutdown.cancel();
}

#[tokio::test]
async fn a_request_limit_and_a_token_limit_coexist_on_one_route() {
    // Requests are charged before dispatch and tokens after the provider
    // answers, so a route can carry both without one standing in for the other.
    let (provider_port, _seen) = provider(openai_reply(), None).await;
    let (url, shutdown) = start_with(
        "openAI",
        provider_port,
        "",
        "              localRateLimit:\n                - maxTokens: 1\n                  tokensPerFill: 1\n                  fillInterval: 60s\n                - maxTokens: 100000\n                  tokensPerFill: 100000\n                  fillInterval: 1h\n                  type: tokens",
    )
    .await;

    let first = post(&url, &chat_request()).await;
    assert!(first.status().is_success(), "{}", first.status());

    let second = post(&url, &chat_request()).await;
    assert_eq!(
        second.status(),
        429,
        "the request bucket holds one, and the token bucket is nowhere near spent"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_streamed_response_is_charged_too() {
    // Usage arrives in the trailing chunk, long after `handle` returned the
    // body. A limit that only applied to buffered responses would miss most of
    // the traffic worth limiting.
    let stream = format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({
            "id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1,
            "model": "gpt-4o",
            "choices": [{"index": 0, "delta": {"content": "hi"}, "finish_reason": null}],
        }),
        json!({
            "id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1,
            "model": "gpt-4o", "choices": [],
            "usage": {"prompt_tokens": 8, "completion_tokens": 2, "total_tokens": 10},
        }),
    );
    let (provider_port, _seen) = provider(openai_reply(), Some(stream)).await;
    let (url, shutdown) = start_with("openAI", provider_port, "", TOKEN_LIMIT).await;

    let mut body = chat_request();
    body["stream"] = json!(true);
    let first = post(&url, &body).await;
    assert!(first.status().is_success(), "{}", first.status());
    // Draining the stream is what delivers the trailing usage chunk.
    let text = first.text().await.expect("should read the stream");
    assert!(text.contains("total_tokens"), "got: {text}");

    let second = post(&url, &chat_request()).await;
    assert_eq!(
        second.status(),
        429,
        "the streamed call's usage should have spent the bucket"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_bucket_that_can_never_refill_fails_an_ai_route_at_startup() {
    let port = free_port().await;
    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - routes:
          - policies:
              localRateLimit:
                - maxTokens: 100
                  tokensPerFill: 0
                  fillInterval: 60s
                  type: tokens
            backends:
              - ai:
                  provider:
                    openAI: {{}}
"#
    );

    let config = Config::from_yaml(&yaml).expect("should parse");
    let err = Gateway::build(&config, None)
        .await
        .map(|_| ())
        .expect_err("a bucket that drains once and never refills should not start");
    assert!(err.to_string().contains("never refills"), "got: {err}");
}
