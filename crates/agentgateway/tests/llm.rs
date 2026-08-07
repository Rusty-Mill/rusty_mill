//! End-to-end tests for the `ai` backend.
//!
//! A mock provider stands in for OpenAI and Anthropic. It records the request
//! body it actually received, which is what makes the translation assertions
//! meaningful: the question is not "did the gateway answer" but "did the
//! provider get the shape its API requires".

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex;

use agentgateway::{Gateway, serve};
use agentgateway_config::Config;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

/// A port nothing in this binary has been handed yet.
///
/// Binding to port 0 and dropping the listener leaves a window in which the
/// same port can be handed out twice, and two tests racing for it fail with
/// `Address already in use`. Remembering what has been issued closes the
/// window between tests, which is where the collisions actually came from.
async fn free_port() -> u16 {
    use std::collections::HashSet;
    use std::sync::{LazyLock, Mutex};
    static ISSUED: LazyLock<Mutex<HashSet<u16>>> = LazyLock::new(Default::default);

    for _ in 0..64 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("should bind");
        let port = listener.local_addr().expect("should have an addr").port();
        drop(listener);
        if ISSUED.lock().expect("lock").insert(port) {
            return port;
        }
    }
    panic!("could not find a port this binary has not already used");
}

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
    let port = free_port().await;
    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - routes:
          - name: llm
            policies:
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
