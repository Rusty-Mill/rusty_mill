//! End-to-end tests for the `retry` and `localRateLimit` policies.
//!
//! The upstream here is scripted: it answers from a queue of statuses, so a
//! test can say "fail twice, then succeed" and assert the client saw only the
//! success. It also counts hits, which is how "did not retry" is distinguished
//! from "retried and got the same answer".

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use agentgateway::{Gateway, serve};
use agentgateway_config::Config;
use tokio_util::sync::CancellationToken;

mod common;
use common::free_port;

/// An upstream that answers from a script.
struct Upstream {
    port: u16,
    hits: Arc<AtomicUsize>,
}

/// Start an upstream that returns each status in `script`, in order.
///
/// Once the script runs out it answers `200`, so "fail twice then recover" is
/// written as `[503, 503]`.
async fn upstream(script: Vec<u16>) -> Upstream {
    use axum::{Router, routing::any};

    let port = free_port().await;
    let hits = Arc::new(AtomicUsize::new(0));
    let queue = Arc::new(Mutex::new(script.into_iter().collect::<Vec<_>>()));

    let counter = Arc::clone(&hits);
    let app = Router::new().fallback(any(move || {
        let counter = Arc::clone(&counter);
        let queue = Arc::clone(&queue);
        async move {
            let index = counter.fetch_add(1, Ordering::Relaxed);
            let status = queue
                .lock()
                .map(|q| q.get(index).copied().unwrap_or(200))
                .unwrap_or(200);
            axum::http::StatusCode::from_u16(status).unwrap_or(axum::http::StatusCode::OK)
        }
    }));

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .expect("upstream should bind");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    Upstream { port, hits }
}

async fn start(route: &str) -> (String, CancellationToken) {
    let port = free_port().await;
    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - routes:
{route}
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

    (format!("http://127.0.0.1:{port}"), shutdown)
}

#[tokio::test]
async fn a_retryable_status_is_retried_until_it_succeeds() {
    let up = upstream(vec![503, 503]).await;
    let (base, shutdown) = start(&format!(
        r#"          - name: proxy
            policies:
              retry:
                attempts: 3
                codes: [503]
            backends:
              - host: "127.0.0.1:{}""#,
        up.port
    ))
    .await;

    let response = reqwest::get(format!("{base}/"))
        .await
        .expect("should answer");
    assert_eq!(
        response.status(),
        200,
        "two failures inside a 3-retry budget should be invisible to the client"
    );
    assert_eq!(
        up.hits.load(Ordering::Relaxed),
        3,
        "one try plus two retries"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn attempts_bounds_the_retries_and_the_last_failure_is_returned() {
    let up = upstream(vec![503, 503, 503, 503, 503]).await;
    let (base, shutdown) = start(&format!(
        r#"          - name: proxy
            policies:
              retry:
                attempts: 2
                codes: [503]
            backends:
              - host: "127.0.0.1:{}""#,
        up.port
    ))
    .await;

    let response = reqwest::get(format!("{base}/"))
        .await
        .expect("should answer");
    assert_eq!(
        response.status(),
        503,
        "when the budget runs out the client sees the real upstream failure"
    );
    assert_eq!(
        up.hits.load(Ordering::Relaxed),
        3,
        "attempts: 2 means two retries after the first try, not two tries total"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_status_not_listed_is_not_retried() {
    let up = upstream(vec![500]).await;
    let (base, shutdown) = start(&format!(
        r#"          - name: proxy
            policies:
              retry:
                attempts: 3
                codes: [503]
            backends:
              - host: "127.0.0.1:{}""#,
        up.port
    ))
    .await;

    let response = reqwest::get(format!("{base}/"))
        .await
        .expect("should answer");
    assert_eq!(response.status(), 500);
    assert_eq!(
        up.hits.load(Ordering::Relaxed),
        1,
        "500 was not opted into, so it must not be retried"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_successful_response_is_never_retried() {
    let up = upstream(vec![]).await;
    let (base, shutdown) = start(&format!(
        r#"          - name: proxy
            policies:
              retry:
                attempts: 5
                codes: [503]
            backends:
              - host: "127.0.0.1:{}""#,
        up.port
    ))
    .await;

    reqwest::get(format!("{base}/"))
        .await
        .expect("should answer");
    assert_eq!(up.hits.load(Ordering::Relaxed), 1);

    shutdown.cancel();
}

#[tokio::test]
async fn a_retry_tries_a_different_endpoint() {
    // Retrying the instance that just failed is the least likely way to
    // succeed, so each attempt takes the next endpoint in the ring.
    let bad = upstream(vec![503, 503, 503, 503]).await;
    let good = upstream(vec![]).await;
    let (base, shutdown) = start(&format!(
        r#"          - name: proxy
            policies:
              retry:
                attempts: 1
                codes: [503]
            backends:
              - host: "127.0.0.1:{}"
              - host: "127.0.0.1:{}""#,
        bad.port, good.port
    ))
    .await;

    let response = reqwest::get(format!("{base}/"))
        .await
        .expect("should answer");
    assert_eq!(
        response.status(),
        200,
        "the retry landed on the healthy one"
    );
    assert_eq!(bad.hits.load(Ordering::Relaxed), 1);
    assert_eq!(good.hits.load(Ordering::Relaxed), 1);

    shutdown.cancel();
}

#[tokio::test]
async fn a_connect_failure_is_retried_onto_a_live_endpoint() {
    // A connect failure never reached the upstream, so replaying it cannot
    // duplicate work -- the one transport error that is safe to retry.
    let dead = free_port().await;
    let live = upstream(vec![]).await;
    let (base, shutdown) = start(&format!(
        r#"          - name: proxy
            policies:
              retry:
                attempts: 1
            backends:
              - host: "127.0.0.1:{dead}"
              - host: "127.0.0.1:{}""#,
        live.port
    ))
    .await;

    let response = reqwest::get(format!("{base}/"))
        .await
        .expect("should answer");
    assert_eq!(response.status(), 200);
    assert_eq!(live.hits.load(Ordering::Relaxed), 1);

    shutdown.cancel();
}

#[tokio::test]
async fn a_post_body_survives_a_retry() {
    // The whole reason bodies are buffered: a replayed attempt has to send the
    // same payload, not an empty one.
    let port = free_port().await;
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let recorder = Arc::clone(&seen);

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .expect("should bind");
    tokio::spawn(async move {
        let app = axum::Router::new().fallback(axum::routing::any(move |body: String| {
            let recorder = Arc::clone(&recorder);
            async move {
                let mut seen = recorder.lock().expect("lock");
                seen.push(body);
                if seen.len() < 2 {
                    axum::http::StatusCode::SERVICE_UNAVAILABLE
                } else {
                    axum::http::StatusCode::OK
                }
            }
        }));
        let _ = axum::serve(listener, app).await;
    });

    let (base, shutdown) = start(&format!(
        r#"          - name: proxy
            policies:
              retry:
                attempts: 2
                codes: [503]
            backends:
              - host: "127.0.0.1:{port}""#
    ))
    .await;

    let response = reqwest::Client::new()
        .post(format!("{base}/"))
        .body("the payload")
        .send()
        .await
        .expect("should answer");

    assert_eq!(response.status(), 200);
    let bodies = seen.lock().expect("lock").clone();
    assert_eq!(
        bodies,
        vec!["the payload".to_string(), "the payload".to_string()],
        "the replayed attempt must carry the same body"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn requests_over_the_rate_limit_get_429_with_retry_after() {
    let up = upstream(vec![]).await;
    let (base, shutdown) = start(&format!(
        r#"          - name: limited
            policies:
              localRateLimit:
                - maxTokens: 2
                  tokensPerFill: 2
                  fillInterval: 60s
            backends:
              - host: "127.0.0.1:{}""#,
        up.port
    ))
    .await;

    for i in 0..2 {
        let response = reqwest::get(format!("{base}/"))
            .await
            .expect("should answer");
        assert_eq!(response.status(), 200, "request {i} is inside the burst");
    }

    let limited = reqwest::get(format!("{base}/"))
        .await
        .expect("should answer");
    assert_eq!(limited.status(), 429);
    assert_eq!(
        limited
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok()),
        Some("60"),
        "a 429 without Retry-After leaves the client guessing"
    );
    assert_eq!(
        up.hits.load(Ordering::Relaxed),
        2,
        "a limited request must not reach the backend"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_rate_limited_request_is_refused_before_authentication() {
    // Rate limiting exists partly to protect the auth path: an unauthenticated
    // flood costs a signature verification each. So the limit must be checked
    // first, and a request over it gets 429 rather than 401 -- even with no
    // credential at all.
    let up = upstream(vec![]).await;
    let (base, shutdown) = start(&format!(
        r#"          - name: limited
            policies:
              localRateLimit:
                - maxTokens: 1
                  tokensPerFill: 1
                  fillInterval: 60s
              jwtAuth:
                issuer: https://auth.example.com
                jwks:
                  url: https://auth.example.com/jwks.json
            backends:
              - host: "127.0.0.1:{}""#,
        up.port
    ))
    .await;

    // The first request is inside the limit, so it reaches auth and is refused
    // there for having no token.
    let first = reqwest::get(format!("{base}/"))
        .await
        .expect("should answer");
    assert_eq!(first.status(), 401);

    let second = reqwest::get(format!("{base}/"))
        .await
        .expect("should answer");
    assert_eq!(
        second.status(),
        429,
        "over the limit, the request is refused before auth is consulted"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn several_limits_all_have_to_permit_the_request() {
    // Burst of 5, sustained 1 per minute. The burst bucket alone would let
    // five straight through.
    let up = upstream(vec![]).await;
    let (base, shutdown) = start(&format!(
        r#"          - name: limited
            policies:
              localRateLimit:
                - maxTokens: 5
                  tokensPerFill: 5
                  fillInterval: 60s
                - maxTokens: 1
                  tokensPerFill: 1
                  fillInterval: 60s
            backends:
              - host: "127.0.0.1:{}""#,
        up.port
    ))
    .await;

    assert_eq!(
        reqwest::get(format!("{base}/"))
            .await
            .expect("should answer")
            .status(),
        200
    );
    assert_eq!(
        reqwest::get(format!("{base}/"))
            .await
            .expect("should answer")
            .status(),
        429,
        "the sustained bucket refuses even though the burst bucket has room"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_route_without_a_limit_is_unaffected() {
    let up = upstream(vec![]).await;
    let (base, shutdown) = start(&format!(
        r#"          - name: open
            backends:
              - host: "127.0.0.1:{}""#,
        up.port
    ))
    .await;

    for _ in 0..25 {
        assert_eq!(
            reqwest::get(format!("{base}/"))
                .await
                .expect("should answer")
                .status(),
            200
        );
    }
    assert_eq!(up.hits.load(Ordering::Relaxed), 25);

    shutdown.cancel();
}

#[test]
fn a_token_type_rate_limit_is_reported_as_unenforced() {
    // `type: tokens` counts LLM tokens, which needs the LLM gateway to exist.
    // Silently treating it as a request limit would enforce a completely
    // different policy than the one written.
    let config = Config::from_yaml(
        r#"
binds:
  - port: 3000
    listeners:
      - routes:
          - policies:
              localRateLimit:
                - maxTokens: 1000
                  tokensPerFill: 1000
                  fillInterval: 60s
                  type: tokens
            backends: [{host: "a:80"}]
"#,
    )
    .expect("should parse");

    let findings = config.lint();
    assert!(
        findings.iter().any(|f| f.contains("type=tokens")),
        "lint should flag it: {findings:?}"
    );
}
