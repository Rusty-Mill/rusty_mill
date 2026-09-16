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
    query: Option<String>,
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
            let query = request.uri().query().map(str::to_string);
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
                seen.query = query;
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

#[tokio::test]
async fn a_body_buffered_for_ext_authz_still_reaches_the_provider() {
    // `extAuthz.includeBody` reads the body before dispatch, so an `ai` route
    // is handed one that was already consumed once. The translation has to see
    // the same bytes it would have.
    let (provider_port, seen) = provider(openai_reply(), None).await;
    let authz_port = free_port().await;
    let allow = axum::Router::new().fallback(axum::routing::any(|| async { "" }));
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{authz_port}"))
        .await
        .expect("authorizer should bind");
    tokio::spawn(async move {
        let _ = axum::serve(listener, allow).await;
    });

    let (url, shutdown) = start_with(
        "openAI",
        provider_port,
        "",
        &format!(
            "              extAuthz:\n                target: \"http://127.0.0.1:{authz_port}\"\n                includeBody: 4096"
        ),
    )
    .await;

    let response = post(&url, &chat_request()).await;
    assert!(response.status().is_success(), "{}", response.status());

    let seen = seen.lock().expect("lock");
    let sent = seen.body.clone().expect("the provider should see a body");
    assert_eq!(
        sent["messages"][1]["content"], "Hello",
        "reading the body for the authorizer must not consume it"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn an_ai_policy_shapes_the_request_the_provider_sees() {
    // `modelAliases`, `prompts`, `defaults` and `overrides` all land on the
    // OpenAI-shaped body before translation, which is the only place a rule
    // written once means the same thing for every provider.
    let (provider_port, seen) = provider(openai_reply(), None).await;
    let (url, shutdown) = start_with(
        "openAI",
        provider_port,
        "",
        concat!(
            "              ai:\n",
            "                modelAliases:\n",
            "                  fast: gpt-4o-mini\n",
            "                prompts:\n",
            "                  prepend:\n",
            "                    - role: system\n",
            "                      content: House rules.\n",
            "                  append:\n",
            "                    - role: user\n",
            "                      content: In English.\n",
            "                defaults:\n",
            "                  temperature: 0.2\n",
            "                overrides:\n",
            "                  max_tokens: 512\n",
        ),
    )
    .await;

    let mut body = chat_request();
    body["model"] = json!("fast");
    body["max_tokens"] = json!(999_999);
    let response = post(&url, &body).await;
    assert!(response.status().is_success(), "{}", response.status());

    let seen = seen.lock().expect("lock");
    let sent = seen.body.clone().expect("the provider should see a body");
    assert_eq!(sent["model"], "gpt-4o-mini", "the alias resolved");
    assert_eq!(sent["temperature"], 0.2, "a default filled what was absent");
    assert_eq!(
        sent["max_tokens"], 512,
        "an override replaced what the caller sent"
    );

    let messages = sent["messages"].as_array().expect("an array");
    assert_eq!(
        messages.len(),
        4,
        "two configured around two of the caller's"
    );
    assert_eq!(messages[0]["content"], "House rules.");
    assert_eq!(messages[3]["content"], "In English.");

    shutdown.cancel();
}

#[tokio::test]
async fn a_configured_model_still_wins_over_an_alias() {
    // The backend's own `model:` is backend configuration rather than route
    // policy -- the most specific statement about where traffic goes.
    let (provider_port, seen) = provider(openai_reply(), None).await;
    let (url, shutdown) = start_with(
        "openAI",
        provider_port,
        "                      model: gpt-4o",
        concat!(
            "              ai:\n",
            "                modelAliases:\n",
            "                  fast: gpt-4o-mini\n",
        ),
    )
    .await;

    let mut body = chat_request();
    body["model"] = json!("fast");
    post(&url, &body).await;

    let seen = seen.lock().expect("lock");
    let sent = seen.body.clone().expect("the provider should see a body");
    assert_eq!(sent["model"], "gpt-4o");

    shutdown.cancel();
}

#[tokio::test]
async fn prompt_caching_marks_an_anthropic_request() {
    // A cache breakpoint is an annotation on the *translated* shape, so this
    // is the one part of the policy that runs after translation.
    let (provider_port, seen) = provider(anthropic_reply(), None).await;
    let (url, shutdown) = start_with(
        "anthropic",
        provider_port,
        "",
        concat!(
            "              ai:\n",
            "                promptCaching:\n",
            "                  cacheSystem: true\n",
            "                  cacheMessages: true\n",
        ),
    )
    .await;

    post(&url, &chat_request()).await;

    let seen = seen.lock().expect("lock");
    let sent = seen.body.clone().expect("the provider should see a body");
    assert_eq!(
        sent["system"][0]["cache_control"],
        json!({"type": "ephemeral"}),
        "the system prompt was promoted to a block and marked: {sent}"
    );
    let messages = sent["messages"].as_array().expect("an array");
    let last = messages.last().expect("at least one turn");
    assert_eq!(
        last["content"][0]["cache_control"],
        json!({"type": "ephemeral"}),
        "{sent}"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn prompt_caching_leaves_an_openai_request_alone() {
    // OpenAI caches long prefixes by itself and takes no configuration, so a
    // breakpoint there would be a field nobody reads.
    let (provider_port, seen) = provider(openai_reply(), None).await;
    let (url, shutdown) = start_with(
        "openAI",
        provider_port,
        "",
        concat!(
            "              ai:\n",
            "                promptCaching:\n",
            "                  cacheSystem: true\n",
            "                  cacheMessages: true\n",
        ),
    )
    .await;

    post(&url, &chat_request()).await;

    let seen = seen.lock().expect("lock");
    let sent = seen.body.clone().expect("the provider should see a body");
    assert_eq!(
        sent["messages"][0]["content"], "Be brief.",
        "untouched: {sent}"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn an_ai_policy_of_only_unimplemented_keys_changes_nothing() {
    // `promptGuard` is reported by `--check`; it must not quietly alter the
    // request on the way past.
    let (provider_port, seen) = provider(openai_reply(), None).await;
    let (url, shutdown) = start_with(
        "openAI",
        provider_port,
        "",
        concat!(
            "              ai:\n",
            "                promptGuard:\n",
            "                  request: []\n",
        ),
    )
    .await;

    post(&url, &chat_request()).await;

    let seen = seen.lock().expect("lock");
    let sent = seen.body.clone().expect("the provider should see a body");
    assert_eq!(sent["messages"].as_array().expect("an array").len(), 2);
    assert_eq!(sent["model"], "gpt-4o");

    shutdown.cancel();
}

/// An OpenAI request that defines a tool and asks the model to use it.
fn tool_request() -> Value {
    json!({
        "model": "claude-sonnet-4",
        "messages": [{"role": "user", "content": "Weather in Oslo?"}],
        "tools": [{
            "type": "function",
            "function": {
                "name": "get_weather",
                "description": "Look up the weather",
                "parameters": {
                    "type": "object",
                    "properties": {"city": {"type": "string"}},
                },
            },
        }],
        "tool_choice": "required",
    })
}

/// An Anthropic response that called one.
fn anthropic_tool_reply() -> Value {
    json!({
        "id": "msg_1",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4",
        "content": [
            {"type": "text", "text": "Looking that up."},
            {"type": "tool_use", "id": "toolu_1", "name": "get_weather",
             "input": {"city": "Oslo"}},
        ],
        "stop_reason": "tool_use",
        "usage": {"input_tokens": 20, "output_tokens": 8},
    })
}

#[tokio::test]
async fn tool_definitions_reach_anthropic_instead_of_being_dropped() {
    // They were dropped entirely, which was worse than unsupported: the
    // finish-reason mapping already translated `tool_use`, so a client could
    // be told a tool ran and never be told which.
    let (provider_port, seen) = provider(anthropic_tool_reply(), None).await;
    let (url, shutdown) = start("anthropic", provider_port, "").await;

    let response = post(&url, &tool_request()).await;
    assert!(response.status().is_success(), "{}", response.status());

    let seen = seen.lock().expect("lock");
    let sent = seen.body.clone().expect("the provider should see a body");
    let tools = sent["tools"].as_array().expect("tools should survive");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], "get_weather");
    assert_eq!(
        tools[0]["input_schema"]["properties"]["city"]["type"], "string",
        "`parameters` becomes `input_schema`: {sent}"
    );
    assert_eq!(
        sent["tool_choice"],
        json!({"type": "any"}),
        "`required` and `any` are the same instruction: {sent}"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_tool_call_comes_back_in_openai_shape() {
    let (provider_port, _seen) = provider(anthropic_tool_reply(), None).await;
    let (url, shutdown) = start("anthropic", provider_port, "").await;

    let response = post(&url, &tool_request()).await;
    let body: Value = response.json().await.expect("should be JSON");

    let choice = &body["choices"][0];
    assert_eq!(choice["finish_reason"], "tool_calls");
    let calls = choice["message"]["tool_calls"]
        .as_array()
        .expect("the call the finish reason claims: {body}");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["id"], "toolu_1");
    assert_eq!(calls[0]["type"], "function");
    assert_eq!(calls[0]["function"]["name"], "get_weather");
    assert_eq!(
        calls[0]["function"]["arguments"], r#"{"city":"Oslo"}"#,
        "the input object becomes an argument string: {body}"
    );
    assert_eq!(
        choice["message"]["content"], "Looking that up.",
        "text beside the call survives"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_tool_result_goes_back_as_part_of_a_user_turn() {
    // Anthropic has no `tool` role: a result is a block inside a user turn.
    let (provider_port, seen) = provider(anthropic_reply(), None).await;
    let (url, shutdown) = start("anthropic", provider_port, "").await;

    let mut body = tool_request();
    body["messages"] = json!([
        {"role": "user", "content": "Weather in Oslo?"},
        {"role": "assistant", "content": null, "tool_calls": [{
            "id": "toolu_1", "type": "function",
            "function": {"name": "get_weather", "arguments": "{\"city\":\"Oslo\"}"},
        }]},
        {"role": "tool", "tool_call_id": "toolu_1", "content": "17 degrees"},
        {"role": "tool", "tool_call_id": "toolu_2", "content": "cloudy"},
    ]);
    post(&url, &body).await;

    let seen = seen.lock().expect("lock");
    let sent = seen.body.clone().expect("the provider should see a body");
    let turns = sent["messages"].as_array().expect("an array");
    assert_eq!(
        turns.len(),
        3,
        "two results join one user turn, because Anthropic refuses two in a row: {sent}"
    );
    assert_eq!(turns[1]["content"][0]["type"], "tool_use");
    assert_eq!(turns[2]["role"], "user");
    let results = turns[2]["content"].as_array().expect("an array");
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["type"], "tool_result");
    assert_eq!(results[0]["tool_use_id"], "toolu_1");
    assert_eq!(results[1]["content"], "cloudy");

    shutdown.cancel();
}

#[tokio::test]
async fn a_streamed_tool_call_is_reframed_as_openai_deltas() {
    let stream = concat!(
        "event: message_start\ndata: {\"message\":{\"id\":\"msg_1\",\"model\":\"claude-sonnet-4\",\"usage\":{\"input_tokens\":9}}}\n\n",
        "event: content_block_start\ndata: {\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"get_weather\",\"input\":{}}}\n\n",
        "event: content_block_delta\ndata: {\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"city\\\":\"}}\n\n",
        "event: content_block_delta\ndata: {\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\\\"Oslo\\\"}\"}}\n\n",
        "event: message_delta\ndata: {\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":4}}\n\n",
    );
    let (provider_port, _seen) = provider(anthropic_reply(), Some(stream.to_string())).await;
    let (url, shutdown) = start("anthropic", provider_port, "").await;

    let mut body = tool_request();
    body["stream"] = json!(true);
    let response = post(&url, &body).await;
    let text = response.text().await.expect("should read the stream");

    let chunks: Vec<Value> = text
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter(|payload| *payload != "[DONE]")
        .filter_map(|payload| serde_json::from_str(payload).ok())
        .collect();

    assert_eq!(
        chunks[0]["choices"][0]["delta"]["role"], "assistant",
        "the role is announced even when the response opens with a call: {text}"
    );
    let opening = &chunks[1]["choices"][0]["delta"]["tool_calls"][0];
    assert_eq!(opening["index"], 0);
    assert_eq!(opening["id"], "toolu_1");
    assert_eq!(opening["function"]["name"], "get_weather");

    let arguments: String = chunks
        .iter()
        .filter_map(|c| c["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"].as_str())
        .collect();
    assert_eq!(arguments, r#"{"city":"Oslo"}"#, "{text}");

    assert!(
        text.ends_with("data: [DONE]\n\n"),
        "a client waits for the sentinel: {text}"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn cache_tools_marks_the_tool_definitions_now_that_they_exist() {
    let (provider_port, seen) = provider(anthropic_reply(), None).await;
    let (url, shutdown) = start_with(
        "anthropic",
        provider_port,
        "",
        concat!(
            "              ai:\n",
            "                promptCaching:\n",
            "                  cacheTools: true\n",
        ),
    )
    .await;

    post(&url, &tool_request()).await;

    let seen = seen.lock().expect("lock");
    let sent = seen.body.clone().expect("the provider should see a body");
    let tools = sent["tools"].as_array().expect("an array");
    assert_eq!(
        tools.last().expect("one tool")["cache_control"],
        json!({"type": "ephemeral"}),
        "{sent}"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn openai_tool_definitions_still_pass_through_untouched() {
    // The passthrough path must not have picked up a translation it does not
    // need.
    let (provider_port, seen) = provider(openai_reply(), None).await;
    let (url, shutdown) = start("openAI", provider_port, "").await;

    let body = tool_request();
    post(&url, &body).await;

    let seen = seen.lock().expect("lock");
    let sent = seen.body.clone().expect("the provider should see a body");
    assert_eq!(sent["tools"], body["tools"], "byte-for-byte: {sent}");
    assert_eq!(sent["tool_choice"], "required");

    shutdown.cancel();
}

/// A guard that refuses a prompt carrying a password.
const REJECT_CREDENTIALS: &str = concat!(
    "              ai:\n",
    "                promptGuard:\n",
    "                  request:\n",
    "                    - regex:\n",
    "                        action: reject\n",
    "                        rules:\n",
    "                          - pattern: \"password[=:]\\\\s*\\\\S+\"\n",
    "                      rejection:\n",
    "                        status: 422\n",
    "                        body: '{\"error\":{\"message\":\"no credentials\"}}'\n",
);

#[tokio::test]
async fn a_prompt_guard_refuses_before_the_provider_is_called() {
    let (provider_port, seen) = provider(openai_reply(), None).await;
    let (url, shutdown) = start_with("openAI", provider_port, "", REJECT_CREDENTIALS).await;

    let mut body = chat_request();
    body["messages"] = json!([{"role": "user", "content": "my password= hunter2"}]);
    let response = post(&url, &body).await;

    assert_eq!(response.status(), 422);
    let answer: Value = response.json().await.expect("should be JSON");
    assert_eq!(
        answer["error"]["message"], "no credentials",
        "the operator's own body, not ours"
    );
    assert!(
        seen.lock().expect("lock").body.is_none(),
        "a refused prompt must not reach the provider"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_prompt_the_guard_permits_reaches_the_provider_unchanged() {
    let (provider_port, seen) = provider(openai_reply(), None).await;
    let (url, shutdown) = start_with("openAI", provider_port, "", REJECT_CREDENTIALS).await;

    let response = post(&url, &chat_request()).await;
    assert!(response.status().is_success(), "{}", response.status());

    let seen = seen.lock().expect("lock");
    let sent = seen.body.clone().expect("the provider should see a body");
    assert_eq!(sent["messages"][1]["content"], "Hello");

    shutdown.cancel();
}

#[tokio::test]
async fn a_mask_rule_rewrites_the_prompt_and_lets_it_through() {
    let (provider_port, seen) = provider(openai_reply(), None).await;
    let (url, shutdown) = start_with(
        "openAI",
        provider_port,
        "",
        concat!(
            "              ai:\n",
            "                promptGuard:\n",
            "                  request:\n",
            "                    - regex:\n",
            "                        action: mask\n",
            "                        rules:\n",
            "                          - builtin: email\n",
        ),
    )
    .await;

    let mut body = chat_request();
    body["messages"] = json!([{"role": "user", "content": "write to a.b@example.com"}]);
    let response = post(&url, &body).await;
    assert!(response.status().is_success(), "{}", response.status());

    let seen = seen.lock().expect("lock");
    let sent = seen.body.clone().expect("the provider should see a body");
    assert_eq!(
        sent["messages"][0]["content"], "write to <EMAIL>",
        "a builtin says what it found: {sent}"
    );

    shutdown.cancel();
}

/// An OpenAI reply carrying a phone number.
fn leaky_reply() -> Value {
    let mut reply = openai_reply();
    reply["choices"][0]["message"]["content"] = json!("you can call 555-867-5309 today");
    reply
}

/// A guard that masks phone numbers in the answer.
const MASK_RESPONSE: &str = concat!(
    "              ai:\n",
    "                promptGuard:\n",
    "                  response:\n",
    "                    - regex:\n",
    "                        action: mask\n",
    "                        rules:\n",
    "                          - builtin: phoneNumber\n",
);

#[tokio::test]
async fn a_response_guard_masks_a_buffered_answer() {
    let (provider_port, _seen) = provider(leaky_reply(), None).await;
    let (url, shutdown) = start_with("openAI", provider_port, "", MASK_RESPONSE).await;

    let response = post(&url, &chat_request()).await;
    let answer: Value = response.json().await.expect("should be JSON");
    assert_eq!(
        answer["choices"][0]["message"]["content"], "you can call <PHONE_NUMBER> today",
        "{answer}"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_response_guard_catches_a_pattern_split_across_stream_chunks() {
    // The whole reason a response rule buffers: scanning each chunk on its own
    // would miss this, and the first half is already at the client by the time
    // the second shows what it started.
    let stream = format!(
        "data: {}\n\ndata: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({
            "id": "c1", "object": "chat.completion.chunk", "created": 1, "model": "gpt-4o",
            "choices": [{"index": 0, "delta": {"role": "assistant"}, "finish_reason": null}],
        }),
        json!({
            "id": "c1", "object": "chat.completion.chunk", "created": 1, "model": "gpt-4o",
            "choices": [{"index": 0, "delta": {"content": "call 555-"}, "finish_reason": null}],
        }),
        json!({
            "id": "c1", "object": "chat.completion.chunk", "created": 1, "model": "gpt-4o",
            "choices": [{"index": 0, "delta": {"content": "867-5309"}, "finish_reason": "stop"}],
        }),
    );
    let (provider_port, _seen) = provider(openai_reply(), Some(stream)).await;
    let (url, shutdown) = start_with("openAI", provider_port, "", MASK_RESPONSE).await;

    let mut body = chat_request();
    body["stream"] = json!(true);
    let response = post(&url, &body).await;
    assert!(response.status().is_success(), "{}", response.status());
    let text = response.text().await.expect("should read the stream");

    assert!(
        text.contains("<PHONE_NUMBER>"),
        "the number spanned two chunks and must still be caught: {text}"
    );
    assert!(
        !text.contains("867-5309"),
        "no part of it may survive: {text}"
    );
    assert!(
        text.ends_with("data: [DONE]\n\n"),
        "still a stream a client can read: {text}"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_guarded_stream_sends_the_masked_text_as_one_chunk() {
    // After masking it is no longer the text the provider chunked, and
    // inventing boundaries for it would be making something up.
    let stream = format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({
            "id": "c1", "object": "chat.completion.chunk", "created": 1, "model": "gpt-4o",
            "choices": [{"index": 0, "delta": {"content": "one "}, "finish_reason": null}],
        }),
        json!({
            "id": "c1", "object": "chat.completion.chunk", "created": 1, "model": "gpt-4o",
            "choices": [{"index": 0, "delta": {"content": "two"}, "finish_reason": "stop"}],
        }),
    );
    let (provider_port, _seen) = provider(openai_reply(), Some(stream)).await;
    let (url, shutdown) = start_with("openAI", provider_port, "", MASK_RESPONSE).await;

    let mut body = chat_request();
    body["stream"] = json!(true);
    let text = post(&url, &body)
        .await
        .text()
        .await
        .expect("should read the stream");

    let contents: Vec<String> = text
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter(|payload| *payload != "[DONE]")
        .filter_map(|payload| serde_json::from_str::<Value>(payload).ok())
        .filter_map(|chunk| {
            chunk["choices"][0]["delta"]["content"]
                .as_str()
                .map(str::to_string)
        })
        .collect();
    assert_eq!(contents, vec!["one two".to_string()], "{text}");

    shutdown.cancel();
}

#[tokio::test]
async fn a_response_guard_can_refuse_a_stream_before_anything_is_sent() {
    // Nothing has gone out yet, so the client can still be told plainly --
    // an ordinary JSON error rather than an event stream carrying a refusal.
    let stream = format!(
        "data: {}\n\ndata: [DONE]\n\n",
        json!({
            "id": "c1", "object": "chat.completion.chunk", "created": 1, "model": "gpt-4o",
            "choices": [{"index": 0, "delta": {"content": "the ssn is 123-45-6789"}},],
        }),
    );
    let (provider_port, _seen) = provider(openai_reply(), Some(stream)).await;
    let (url, shutdown) = start_with(
        "openAI",
        provider_port,
        "",
        concat!(
            "              ai:\n",
            "                promptGuard:\n",
            "                  response:\n",
            "                    - regex:\n",
            "                        action: reject\n",
            "                        rules:\n",
            "                          - builtin: ssn\n",
            "                      rejection:\n",
            "                        status: 502\n",
        ),
    )
    .await;

    let mut body = chat_request();
    body["stream"] = json!(true);
    let response = post(&url, &body).await;
    assert_eq!(response.status(), 502);
    assert!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("application/json")),
        "a refusal is not an event stream"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_route_with_only_a_request_guard_still_streams_chunk_by_chunk() {
    // A request rule costs a stream nothing, and it would be a poor trade if
    // it did.
    let stream = format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({
            "id": "c1", "object": "chat.completion.chunk", "created": 1, "model": "gpt-4o",
            "choices": [{"index": 0, "delta": {"content": "one "}, "finish_reason": null}],
        }),
        json!({
            "id": "c1", "object": "chat.completion.chunk", "created": 1, "model": "gpt-4o",
            "choices": [{"index": 0, "delta": {"content": "two"}, "finish_reason": "stop"}],
        }),
    );
    let (provider_port, _seen) = provider(openai_reply(), Some(stream)).await;
    let (url, shutdown) = start_with("openAI", provider_port, "", REJECT_CREDENTIALS).await;

    let mut body = chat_request();
    body["stream"] = json!(true);
    let text = post(&url, &body)
        .await
        .text()
        .await
        .expect("should read the stream");

    let contents: Vec<String> = text
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter(|payload| *payload != "[DONE]")
        .filter_map(|payload| serde_json::from_str::<Value>(payload).ok())
        .filter_map(|chunk| {
            chunk["choices"][0]["delta"]["content"]
                .as_str()
                .map(str::to_string)
        })
        .collect();
    assert_eq!(
        contents,
        vec!["one ".to_string(), "two".to_string()],
        "two chunks, as the provider sent them: {text}"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_pattern_that_does_not_compile_fails_an_ai_route_at_startup() {
    let port = free_port().await;
    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - routes:
          - policies:
              backendAuth:
                key: test-key
              ai:
                promptGuard:
                  request:
                    - regex:
                        action: reject
                        rules:
                          - pattern: "["
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
        .expect_err("a rule that can never fire should not start");
    assert!(err.to_string().contains("regular expression"), "got: {err}");
}

#[tokio::test]
async fn a_guarded_stream_does_not_re_emit_the_done_sentinel_as_a_chunk() {
    // `data: [DONE]` is not JSON and parses to null. Collecting it with the
    // chunks would hand a client `data: null` before the real sentinel.
    let stream = format!(
        "data: {}\n\ndata: [DONE]\n\n",
        json!({
            "id": "c1", "object": "chat.completion.chunk", "created": 1, "model": "gpt-4o",
            "choices": [{"index": 0, "delta": {"content": "hello"}, "finish_reason": "stop"}],
        }),
    );
    let (provider_port, _seen) = provider(openai_reply(), Some(stream)).await;
    let (url, shutdown) = start_with("openAI", provider_port, "", MASK_RESPONSE).await;

    let mut body = chat_request();
    body["stream"] = json!(true);
    let text = post(&url, &body)
        .await
        .text()
        .await
        .expect("should read the stream");

    assert!(!text.contains("data: null"), "{text}");
    assert_eq!(
        text.matches("data: [DONE]").count(),
        1,
        "exactly one sentinel: {text}"
    );

    shutdown.cancel();
}

/// A guard webhook answering from a script.
///
/// `answers` is consulted per path, so one fixture serves both phases.
async fn guard_webhook(
    request_action: Value,
    response_action: Value,
) -> (u16, Arc<Mutex<Vec<(String, Value, Vec<(String, String)>)>>>) {
    use axum::{Router, extract::Request, routing::any};

    let port = free_port().await;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&seen);

    let app = Router::new().fallback(any(move |request: Request| {
        let recorder = Arc::clone(&recorder);
        let request_action = request_action.clone();
        let response_action = response_action.clone();
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
            let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            if let Ok(mut seen) = recorder.lock() {
                seen.push((path.clone(), body, headers));
            }

            let action = if path.contains("response") {
                response_action
            } else {
                request_action
            };
            axum::Json(json!({"action": action}))
        }
    }));

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .expect("webhook should bind");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    (port, seen)
}

fn webhook_policy(port: u16, phase: &str) -> String {
    format!(
        concat!(
            "              ai:\n",
            "                promptGuard:\n",
            "                  {phase}:\n",
            "                    - webhook:\n",
            "                        target:\n",
            "                          host: \"127.0.0.1:{port}\"\n",
        ),
        phase = phase,
        port = port
    )
}

#[tokio::test]
async fn a_webhook_sees_the_conversation_in_upstreams_shape() {
    let (webhook_port, seen) = guard_webhook(json!({"reason": "fine"}), json!({})).await;
    let (provider_port, _p) = provider(openai_reply(), None).await;
    let (url, shutdown) = start_with(
        "openAI",
        provider_port,
        "",
        &webhook_policy(webhook_port, "request"),
    )
    .await;

    let response = post(&url, &chat_request()).await;
    assert!(response.status().is_success(), "{}", response.status());

    let seen = seen.lock().expect("lock");
    let (path, body, _headers) = seen.first().expect("the webhook should be asked");
    assert_eq!(path, "/request", "upstream's default path");
    let messages = body["body"]["messages"]
        .as_array()
        .expect("upstream's shape: {body}");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(messages[1]["content"], "Hello");

    shutdown.cancel();
}

#[tokio::test]
async fn a_webhook_refusal_stops_the_request() {
    let (webhook_port, _seen) = guard_webhook(
        json!({"body": "blocked by policy", "status_code": 451}),
        json!({}),
    )
    .await;
    let (provider_port, provider_seen) = provider(openai_reply(), None).await;
    let (url, shutdown) = start_with(
        "openAI",
        provider_port,
        "",
        &webhook_policy(webhook_port, "request"),
    )
    .await;

    let response = post(&url, &chat_request()).await;
    assert_eq!(response.status(), 451);
    assert_eq!(
        response.text().await.expect("a body"),
        "blocked by policy",
        "the webhook's own message"
    );
    assert!(
        provider_seen.lock().expect("lock").body.is_none(),
        "a refused prompt must not reach the provider"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_webhook_mask_rewrites_the_prompt_it_was_shown() {
    let (webhook_port, _seen) = guard_webhook(
        json!({"body": {"messages": [
            {"role": "system", "content": "Be brief."},
            {"role": "user", "content": "REDACTED"},
        ]}}),
        json!({}),
    )
    .await;
    let (provider_port, provider_seen) = provider(openai_reply(), None).await;
    let (url, shutdown) = start_with(
        "openAI",
        provider_port,
        "",
        &webhook_policy(webhook_port, "request"),
    )
    .await;

    post(&url, &chat_request()).await;

    let seen = provider_seen.lock().expect("lock");
    let sent = seen.body.clone().expect("the provider should see a body");
    assert_eq!(sent["messages"][1]["content"], "REDACTED", "{sent}");

    shutdown.cancel();
}

#[tokio::test]
async fn a_webhook_guards_the_answer_too() {
    let (webhook_port, seen) = guard_webhook(
        json!({}),
        json!({"body": {"choices": [
            {"message": {"role": "assistant", "content": "cleaned up"}}
        ]}}),
    )
    .await;
    let (provider_port, _p) = provider(openai_reply(), None).await;
    let (url, shutdown) = start_with(
        "openAI",
        provider_port,
        "",
        &webhook_policy(webhook_port, "response"),
    )
    .await;

    let answer: Value = post(&url, &chat_request())
        .await
        .json()
        .await
        .expect("should be JSON");
    assert_eq!(answer["choices"][0]["message"]["content"], "cleaned up");

    let seen = seen.lock().expect("lock");
    let (path, body, _headers) = seen.first().expect("the webhook should be asked");
    assert_eq!(path, "/response");
    assert_eq!(
        body["body"]["choices"][0]["message"]["content"], "Hi",
        "it was shown what the provider actually said: {body}"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_webhook_that_cannot_be_reached_refuses_by_default() {
    // A content control that waves traffic through when its service is down
    // is not a content control.
    let dead = free_port().await;
    let (provider_port, provider_seen) = provider(openai_reply(), None).await;
    let (url, shutdown) = start_with(
        "openAI",
        provider_port,
        "",
        &webhook_policy(dead, "request"),
    )
    .await;

    let response = post(&url, &chat_request()).await;
    assert_eq!(
        response.status(),
        503,
        "nothing decided the content was unacceptable, so not a 400"
    );
    assert!(provider_seen.lock().expect("lock").body.is_none());

    shutdown.cancel();
}

#[tokio::test]
async fn failing_open_has_to_be_asked_for() {
    let dead = free_port().await;
    let (provider_port, provider_seen) = provider(openai_reply(), None).await;
    let (url, shutdown) = start_with(
        "openAI",
        provider_port,
        "",
        &format!(
            concat!(
                "              ai:\n",
                "                promptGuard:\n",
                "                  request:\n",
                "                    - webhook:\n",
                "                        target:\n",
                "                          host: \"127.0.0.1:{dead}\"\n",
                "                        failureMode: failOpen\n",
            ),
            dead = dead
        ),
    )
    .await;

    let response = post(&url, &chat_request()).await;
    assert!(response.status().is_success(), "{}", response.status());
    assert!(
        provider_seen.lock().expect("lock").body.is_some(),
        "failing open means the call goes through"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn header_expressions_read_the_callers_request_and_can_move_the_path() {
    let (webhook_port, seen) = guard_webhook(json!({}), json!({})).await;
    let (provider_port, _p) = provider(openai_reply(), None).await;
    let (url, shutdown) = start_with(
        "openAI",
        provider_port,
        "",
        &format!(
            concat!(
                "              ai:\n",
                "                promptGuard:\n",
                "                  request:\n",
                "                    - webhook:\n",
                "                        target:\n",
                "                          host: \"127.0.0.1:{port}\"\n",
                "                        headers:\n",
                "                          \":path\": '\"/api/guardrails/request\"'\n",
                "                          x-tenant: 'request.headers[\"x-tenant\"]'\n",
                "                        forwardHeaderMatches: [x-trace]\n",
            ),
            port = webhook_port
        ),
    )
    .await;

    reqwest::Client::new()
        .post(&url)
        .header("x-tenant", "acme")
        .header("x-trace", "abc123")
        .header("x-secret", "do-not-forward")
        .json(&chat_request())
        .send()
        .await
        .expect("the gateway should answer");

    let seen = seen.lock().expect("lock");
    let (path, _body, headers) = seen.first().expect("the webhook should be asked");
    assert_eq!(path, "/api/guardrails/request", "`:path` moved it");
    assert!(
        headers.iter().any(|(k, v)| k == "x-tenant" && v == "acme"),
        "a CEL header read the caller's own: {headers:?}"
    );
    assert!(
        headers.iter().any(|(k, v)| k == "x-trace" && v == "abc123"),
        "forwardHeaderMatches carried it: {headers:?}"
    );
    assert!(
        !headers.iter().any(|(k, _)| k == "x-secret"),
        "and an empty list forwards nothing else: {headers:?}"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_regex_rule_above_a_webhook_can_refuse_without_the_network_call() {
    // Rules run in order, so a cheap local rule placed first saves the call.
    let (webhook_port, seen) = guard_webhook(json!({}), json!({})).await;
    let (provider_port, _p) = provider(openai_reply(), None).await;
    let (url, shutdown) = start_with(
        "openAI",
        provider_port,
        "",
        &format!(
            concat!(
                "              ai:\n",
                "                promptGuard:\n",
                "                  request:\n",
                "                    - regex:\n",
                "                        action: reject\n",
                "                        rules:\n",
                "                          - pattern: forbidden\n",
                "                    - webhook:\n",
                "                        target:\n",
                "                          host: \"127.0.0.1:{port}\"\n",
            ),
            port = webhook_port
        ),
    )
    .await;

    let mut body = chat_request();
    body["messages"] = json!([{"role": "user", "content": "this is forbidden"}]);
    let response = post(&url, &body).await;
    assert_eq!(response.status(), 400);
    assert!(
        seen.lock().expect("lock").is_empty(),
        "the webhook was never called"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_header_expression_can_read_the_request_the_caller_wrote() {
    // `llmRequest.*` only exists once the body is parsed, so the context has
    // to be completed after that rather than built complete.
    let (webhook_port, seen) = guard_webhook(json!({}), json!({})).await;
    let (provider_port, _p) = provider(openai_reply(), None).await;
    let (url, shutdown) = start_with(
        "openAI",
        provider_port,
        "",
        &format!(
            concat!(
                "              ai:\n",
                "                promptGuard:\n",
                "                  request:\n",
                "                    - webhook:\n",
                "                        target:\n",
                "                          host: \"127.0.0.1:{port}\"\n",
                "                        headers:\n",
                "                          x-model: llmRequest.model\n",
            ),
            port = webhook_port
        ),
    )
    .await;

    post(&url, &chat_request()).await;

    let seen = seen.lock().expect("lock");
    let (_path, _body, headers) = seen.first().expect("the webhook should be asked");
    assert!(
        headers.iter().any(|(k, v)| k == "x-model" && v == "gpt-4o"),
        "saw {headers:?}"
    );

    shutdown.cancel();
}

/// A provider that also answers OpenAI's moderation endpoint.
///
/// One server for both, because a moderation rule with no key of its own calls
/// the route's own host rather than `api.openai.com` — which is the behaviour
/// under test as much as the verdict is.
async fn moderating_provider(
    flagged: bool,
    moderation_status: u16,
) -> (u16, Arc<Mutex<Vec<(Value, Vec<(String, String)>)>>>) {
    use axum::{Router, extract::Request, routing::any};

    let port = free_port().await;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&seen);

    let app = Router::new().fallback(any(move |request: Request| {
        let recorder = Arc::clone(&recorder);
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
            let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

            if !path.contains("moderations") {
                return axum::response::Response::builder()
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(openai_reply().to_string()))
                    .expect("response should build");
            }

            if let Ok(mut seen) = recorder.lock() {
                seen.push((body, headers));
            }
            axum::response::Response::builder()
                .status(moderation_status)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    json!({"id": "modr-1", "model": "omni-moderation-latest",
                           "results": [{"flagged": flagged, "categories": {"violence": flagged}}]})
                    .to_string(),
                ))
                .expect("response should build")
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

/// A `promptGuard` with one moderation rule, indented for `start_with`.
///
/// `rejection` is given as bare `key: value` lines and indented here, so a
/// test says what it wants rather than counting spaces.
fn moderation_policy(phase: &str, rejection: &[&str]) -> String {
    let mut lines = vec![
        "              ai:".to_string(),
        "                promptGuard:".to_string(),
        format!("                  {phase}:"),
        "                    - openAIModeration: {}".to_string(),
    ];
    if !rejection.is_empty() {
        lines.push("                      rejection:".to_string());
        lines.extend(
            rejection
                .iter()
                .map(|line| format!("                        {line}")),
        );
    }
    lines.join("\n")
}

#[tokio::test]
async fn a_flagged_prompt_is_refused_before_the_provider_is_called() {
    let (provider_port, seen) = moderating_provider(true, 200).await;
    let (url, shutdown) = start_with(
        "openAI",
        provider_port,
        "",
        &moderation_policy("request", &["status: 451", "body: this prompt was refused"]),
    )
    .await;

    let response = post(&url, &chat_request()).await;
    assert_eq!(response.status().as_u16(), 451);
    assert_eq!(
        response.text().await.expect("a body"),
        "this prompt was refused"
    );

    // The classifier was asked, and the chat endpoint never was: a refused
    // prompt costs nothing at the provider.
    assert_eq!(seen.lock().expect("lock").len(), 1);

    shutdown.cancel();
}

#[tokio::test]
async fn the_classifier_is_shown_every_message_and_paid_for_with_the_routes_key() {
    // A borrowed key travels only as far as the route it came from, so the
    // call lands on this same stub rather than on `api.openai.com`.
    let (provider_port, seen) = moderating_provider(false, 200).await;
    let (url, shutdown) = start_with(
        "openAI",
        provider_port,
        "",
        &moderation_policy("request", &[]),
    )
    .await;

    let response = post(&url, &chat_request()).await;
    assert!(response.status().is_success(), "{}", response.status());

    let seen = seen.lock().expect("lock");
    let (body, headers) = seen.first().expect("the classifier should be asked");
    assert_eq!(
        body["input"],
        json!(["Be brief.", "Hello"]),
        "every message, in order: {body}"
    );
    assert_eq!(
        body["model"], "omni-moderation-latest",
        "upstream's default classifier"
    );
    assert!(
        headers
            .iter()
            .any(|(name, value)| name == "authorization" && value == "Bearer test-key"),
        "the route's own key: {headers:?}"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_classifier_that_cannot_be_reached_refuses_the_request() {
    // Fails closed, and with a 503 rather than the rule's own rejection:
    // nothing decided this prompt was unacceptable.
    let (provider_port, _seen) = moderating_provider(false, 500).await;
    let (url, shutdown) = start_with(
        "openAI",
        provider_port,
        "",
        &moderation_policy("request", &["status: 451"]),
    )
    .await;

    let response = post(&url, &chat_request()).await;
    assert_eq!(response.status().as_u16(), 503);

    shutdown.cancel();
}

#[tokio::test]
async fn a_moderation_rule_on_the_response_phase_inspects_nothing() {
    // Upstream's response guard has no moderation variant. `--check` reports
    // the rule; the request itself is served as though it were not there.
    let (provider_port, seen) = moderating_provider(true, 200).await;
    let (url, shutdown) = start_with(
        "openAI",
        provider_port,
        "",
        &moderation_policy("response", &[]),
    )
    .await;

    let response = post(&url, &chat_request()).await;
    assert!(response.status().is_success(), "{}", response.status());
    assert!(
        seen.lock().expect("lock").is_empty(),
        "the classifier must not be called for an answer"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_moderation_rule_on_an_anthropic_route_fails_at_startup() {
    // The failure this refusal exists to prevent: borrowing the route's key
    // would send an Anthropic credential to OpenAI.
    let port = free_port().await;
    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - routes:
          - policies:
              backendAuth:
                key: sk-ant-secret
              ai:
                promptGuard:
                  request:
                    - openAIModeration: {{}}
            backends:
              - ai:
                  provider:
                    anthropic: {{}}
"#
    );

    let config = Config::from_yaml(&yaml).expect("should parse");
    let err = Gateway::build(&config, None)
        .await
        .map(|_| ())
        .expect_err("an Anthropic key must not be lent to OpenAI");
    assert!(err.to_string().contains("anthropic"), "got: {err}");
    assert!(err.to_string().contains("third party"), "got: {err}");
}

#[tokio::test]
async fn a_moderation_rule_with_its_own_key_serves_an_anthropic_route() {
    // The other half of the rule above: a key of its own is an OpenAI key, so
    // there is nothing to refuse.
    let port = free_port().await;
    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - routes:
          - policies:
              backendAuth:
                key: sk-ant-secret
              ai:
                promptGuard:
                  request:
                    - openAIModeration:
                        policies:
                          backendAuth:
                            key: sk-moderation
            backends:
              - ai:
                  provider:
                    anthropic: {{}}
"#
    );

    let config = Config::from_yaml(&yaml).expect("should parse");
    Gateway::build(&config, None)
        .await
        .map(|_| ())
        .expect("a rule with its own OpenAI key should start");
}

fn gemini_reply() -> Value {
    json!({
        "responseId": "resp-1",
        "modelVersion": "gemini-2.5-flash",
        "candidates": [{
            "content": {"role": "model", "parts": [{"text": "Hi"}]},
            "finishReason": "STOP",
            "index": 0,
        }],
        "usageMetadata": {
            "promptTokenCount": 8,
            "candidatesTokenCount": 2,
            "totalTokenCount": 10,
        },
    })
}

fn gemini_request() -> Value {
    json!({
        "model": "gemini-2.5-flash",
        "messages": [
            {"role": "system", "content": "Be brief."},
            {"role": "user", "content": "Hello"},
        ],
        "temperature": 0.2,
    })
}

#[tokio::test]
async fn a_gemini_request_names_the_model_and_the_method_in_the_path() {
    // Unlike the other two providers, the address depends on the request.
    let (provider_port, seen) = provider(gemini_reply(), None).await;
    let (url, shutdown) = start("gemini", provider_port, "").await;

    let response = post(&url, &gemini_request()).await;
    assert!(response.status().is_success(), "{}", response.status());

    let seen = seen.lock().expect("lock");
    assert_eq!(
        seen.path, "/v1beta/models/gemini-2.5-flash:generateContent",
        "the base is the endpoint; the rest is built per request"
    );
    assert!(
        seen.headers
            .iter()
            .any(|(name, value)| name == "x-goog-api-key" && value == "test-key"),
        "the key belongs in a header, not the query string: {:?}",
        seen.headers
    );
    assert!(
        !seen.headers.iter().any(|(name, _)| name == "authorization"),
        "a bearer token is not how Gemini is called: {:?}",
        seen.headers
    );

    let body = seen.body.as_ref().expect("a body");
    assert_eq!(body["contents"][0]["role"], "user");
    assert_eq!(body["contents"][0]["parts"][0]["text"], "Hello");
    assert_eq!(body["systemInstruction"]["parts"][0]["text"], "Be brief.");
    assert_eq!(body["generationConfig"]["temperature"], 0.2);
    assert!(
        body.get("model").is_none() && body.get("messages").is_none(),
        "Gemini rejects fields it does not know: {body}"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_gemini_response_comes_back_in_openai_shape() {
    let (provider_port, _seen) = provider(gemini_reply(), None).await;
    let (url, shutdown) = start("gemini", provider_port, "").await;

    let answered: Value = post(&url, &gemini_request())
        .await
        .json()
        .await
        .expect("a JSON answer");

    assert_eq!(answered["object"], "chat.completion");
    assert_eq!(answered["id"], "resp-1");
    assert_eq!(answered["model"], "gemini-2.5-flash");
    assert_eq!(answered["choices"][0]["message"]["role"], "assistant");
    assert_eq!(answered["choices"][0]["message"]["content"], "Hi");
    assert_eq!(answered["choices"][0]["finish_reason"], "stop");
    assert_eq!(answered["usage"]["prompt_tokens"], 8);
    assert_eq!(answered["usage"]["completion_tokens"], 2);
    assert_eq!(answered["usage"]["total_tokens"], 10);

    shutdown.cancel();
}

#[tokio::test]
async fn a_model_name_that_would_choose_another_endpoint_never_leaves_the_gateway() {
    // The one place a client's string reaches a URL the gateway signs with its
    // own key.
    let (provider_port, seen) = provider(gemini_reply(), None).await;
    let (url, shutdown) = start("gemini", provider_port, "").await;

    let mut hostile = gemini_request();
    hostile["model"] = json!("../../v1beta/tunedModels/private");
    let response = post(&url, &hostile).await;

    assert_eq!(response.status().as_u16(), 400);
    assert!(
        seen.lock().expect("lock").body.is_none(),
        "nothing should have been dialled"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_forced_model_decides_the_gemini_path() {
    // The URL is built after the whole policy chain has had its say, so the
    // backend's own model is what ends up in it.
    let (provider_port, seen) = provider(gemini_reply(), None).await;
    let (url, shutdown) = start(
        "gemini",
        provider_port,
        "                      model: gemini-2.5-pro",
    )
    .await;

    let response = post(&url, &gemini_request()).await;
    assert!(response.status().is_success(), "{}", response.status());
    assert_eq!(
        seen.lock().expect("lock").path,
        "/v1beta/models/gemini-2.5-pro:generateContent"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn an_alias_decides_the_gemini_path_too() {
    let (provider_port, seen) = provider(gemini_reply(), None).await;
    let (url, shutdown) = start_with(
        "gemini",
        provider_port,
        "",
        "              ai:\n                modelAliases:\n                  fast: gemini-2.5-flash-lite\n",
    )
    .await;

    let mut asked = gemini_request();
    asked["model"] = json!("fast");
    let response = post(&url, &asked).await;
    assert!(response.status().is_success(), "{}", response.status());
    assert_eq!(
        seen.lock().expect("lock").path,
        "/v1beta/models/gemini-2.5-flash-lite:generateContent"
    );

    shutdown.cancel();
}

/// The frames Gemini's `?alt=sse` stream sends, as one SSE body.
fn gemini_stream(frames: Vec<Value>) -> String {
    frames
        .iter()
        .map(|frame| format!("data: {frame}\n\n"))
        .collect()
}

#[tokio::test]
async fn a_gemini_stream_is_reframed_as_openai_chunks() {
    let stream = gemini_stream(vec![
        json!({"responseId": "resp-1", "modelVersion": "gemini-2.5-flash",
               "candidates": [{"content": {"role": "model", "parts": [{"text": "Hello"}]}}]}),
        json!({"responseId": "resp-1", "modelVersion": "gemini-2.5-flash",
               "candidates": [{"content": {"role": "model", "parts": [{"text": " there"}]},
                               "finishReason": "STOP"}],
               "usageMetadata": {"promptTokenCount": 8, "candidatesTokenCount": 2}}),
    ]);
    let (provider_port, seen) = provider(json!({}), Some(stream)).await;
    let (url, shutdown) = start("gemini", provider_port, "").await;

    let mut asked = gemini_request();
    asked["stream"] = json!(true);
    let response = post(&url, &asked).await;
    assert!(response.status().is_success(), "{}", response.status());
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("text/event-stream")
    );

    let body = response.text().await.expect("a body");
    let chunks: Vec<Value> = body
        .split("\n\n")
        .filter_map(|frame| frame.strip_prefix("data: "))
        .filter(|data| *data != "[DONE]")
        .map(|data| serde_json::from_str(data).expect("a chunk"))
        .collect();

    assert_eq!(chunks[0]["choices"][0]["delta"]["role"], "assistant");
    assert_eq!(chunks[1]["choices"][0]["delta"]["content"], "Hello");
    assert_eq!(chunks[2]["choices"][0]["delta"]["content"], " there");
    assert_eq!(chunks[3]["choices"][0]["finish_reason"], "stop");
    for chunk in &chunks {
        assert_eq!(chunk["object"], "chat.completion.chunk");
        assert_eq!(chunk["id"], "resp-1");
    }
    assert!(body.ends_with("data: [DONE]\n\n"), "{body}");

    // The stream is a different method on a different URL, and `alt=sse` is
    // what makes it a stream at all: without it Gemini answers with a JSON
    // array of responses rather than server-sent events.
    let seen = seen.lock().expect("lock");
    assert_eq!(
        seen.path,
        "/v1beta/models/gemini-2.5-flash:streamGenerateContent"
    );
    assert_eq!(seen.query.as_deref(), Some("alt=sse"));

    shutdown.cancel();
}

#[tokio::test]
async fn a_streamed_gemini_call_reaches_the_client_as_tool_call_deltas() {
    // Gemini sends the call whole rather than as argument fragments, so the
    // delta carries the entire `arguments` string at once.
    let stream = gemini_stream(vec![json!({
        "responseId": "resp-1",
        "modelVersion": "gemini-2.5-flash",
        "candidates": [{
            "content": {"role": "model", "parts": [
                {"functionCall": {"name": "get_weather", "args": {"city": "Oslo"}}},
            ]},
            "finishReason": "STOP",
        }],
    })]);
    let (provider_port, _seen) = provider(json!({}), Some(stream)).await;
    let (url, shutdown) = start("gemini", provider_port, "").await;

    let mut asked = gemini_request();
    asked["stream"] = json!(true);
    let body = post(&url, &asked).await.text().await.expect("a body");

    let chunks: Vec<Value> = body
        .split("\n\n")
        .filter_map(|frame| frame.strip_prefix("data: "))
        .filter(|data| *data != "[DONE]")
        .map(|data| serde_json::from_str(data).expect("a chunk"))
        .collect();

    let call = &chunks[1]["choices"][0]["delta"]["tool_calls"][0];
    assert_eq!(call["index"], 0);
    assert_eq!(call["id"], "call_0");
    assert_eq!(call["function"]["name"], "get_weather");
    assert_eq!(call["function"]["arguments"], r#"{"city":"Oslo"}"#);

    shutdown.cancel();
}

#[tokio::test]
async fn tool_definitions_reach_gemini_as_function_declarations() {
    let (provider_port, seen) = provider(gemini_reply(), None).await;
    let (url, shutdown) = start("gemini", provider_port, "").await;

    let mut asked = gemini_request();
    asked["tools"] = json!([{
        "type": "function",
        "function": {
            "name": "get_weather",
            "description": "Look it up",
            "parameters": {
                "type": "object",
                "additionalProperties": false,
                "properties": {"city": {"type": "string"}},
                "required": ["city"],
            },
        },
    }]);
    asked["tool_choice"] = json!("auto");

    let response = post(&url, &asked).await;
    assert!(response.status().is_success(), "{}", response.status());

    let seen = seen.lock().expect("lock");
    let body = seen.body.as_ref().expect("a body");
    let declaration = &body["tools"][0]["functionDeclarations"][0];
    assert_eq!(declaration["name"], "get_weather");
    assert_eq!(declaration["parameters"]["required"], json!(["city"]));
    assert!(
        declaration["parameters"]
            .get("additionalProperties")
            .is_none(),
        "Gemini refuses the whole request over a field it does not know: {body}"
    );
    assert_eq!(body["toolConfig"]["functionCallingConfig"]["mode"], "AUTO");

    shutdown.cancel();
}

#[tokio::test]
async fn a_gemini_tool_call_round_trips() {
    // The answer's call comes back with an id, and the result the client sends
    // under that id becomes a `functionResponse` naming the right function.
    let (provider_port, seen) = provider(
        json!({
            "responseId": "resp-1",
            "modelVersion": "gemini-2.5-flash",
            "candidates": [{
                "content": {"role": "model", "parts": [
                    {"functionCall": {"name": "get_weather", "args": {"city": "Oslo"}}},
                ]},
                "finishReason": "STOP",
            }],
        }),
        None,
    )
    .await;
    let (url, shutdown) = start("gemini", provider_port, "").await;

    let mut asked = gemini_request();
    asked["tools"] = json!([{
        "type": "function",
        "function": {"name": "get_weather", "parameters": {"type": "object"}},
    }]);
    let answered: Value = post(&url, &asked).await.json().await.expect("JSON");

    let call = &answered["choices"][0]["message"]["tool_calls"][0];
    assert_eq!(call["function"]["name"], "get_weather");
    let call_id = call["id"].as_str().expect("an id").to_string();

    // Now the second leg, as a client would send it back.
    let mut second = asked.clone();
    second["messages"] = json!([
        {"role": "user", "content": "weather in Oslo?"},
        answered["choices"][0]["message"],
        {"role": "tool", "tool_call_id": call_id, "content": "{\"temp\": 4}"},
    ]);
    let response = post(&url, &second).await;
    assert!(response.status().is_success(), "{}", response.status());

    let seen = seen.lock().expect("lock");
    let contents = seen.body.as_ref().expect("a body")["contents"]
        .as_array()
        .expect("contents")
        .clone();
    assert_eq!(contents.len(), 3);
    assert_eq!(contents[1]["role"], "model");
    assert_eq!(
        contents[1]["parts"][0]["functionCall"]["name"],
        "get_weather"
    );
    assert_eq!(contents[2]["role"], "user", "there is no tool role here");
    assert_eq!(
        contents[2]["parts"][0]["functionResponse"],
        json!({"name": "get_weather", "response": {"temp": 4}})
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_gemini_stream_is_charged_to_the_token_budget() {
    let stream = gemini_stream(vec![json!({
        "responseId": "resp-1",
        "modelVersion": "gemini-2.5-flash",
        "candidates": [{"content": {"parts": [{"text": "Hi"}]}, "finishReason": "STOP"}],
        "usageMetadata": {"promptTokenCount": 40, "candidatesTokenCount": 10},
    })]);
    let (provider_port, _seen) = provider(json!({}), Some(stream)).await;
    let (url, shutdown) = start_with(
        "gemini",
        provider_port,
        "",
        "              localRateLimit:\n                - type: tokens\n                  maxTokens: 50\n                  tokensPerFill: 50\n                  fillInterval: 60s\n",
    )
    .await;

    let mut asked = gemini_request();
    asked["stream"] = json!(true);
    let first = post(&url, &asked).await;
    assert!(first.status().is_success());
    let _ = first.text().await;

    // The budget is spent by what the stream reported, so the next one is
    // refused rather than served.
    let second = post(&url, &asked).await;
    assert_eq!(second.status().as_u16(), 429);

    shutdown.cancel();
}
