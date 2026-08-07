//! End-to-end tests for the `extAuthz` policy.
//!
//! A mock authorizer records what it was asked and answers from a script, and
//! a mock upstream records what reached it. Both matter: the interesting
//! questions are whether a denied request stopped at the gateway, and whether
//! the headers an authorizer set actually travelled on.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

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

/// What the authorizer was asked.
#[derive(Default, Clone)]
struct Asked {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
}

/// How the authorizer should answer.
#[derive(Clone, Copy)]
enum Verdict {
    /// 200, setting `x-user-id` and `x-is-admin` on the response.
    Allow,
    /// The given status, with a JSON body explaining why.
    Deny(u16),
}

async fn authorizer(verdict: Verdict) -> (u16, Arc<Mutex<Option<Asked>>>, Arc<AtomicUsize>) {
    use axum::{Router, extract::Request, routing::any};

    let port = free_port().await;
    let asked = Arc::new(Mutex::new(None));
    let hits = Arc::new(AtomicUsize::new(0));

    let recorder = Arc::clone(&asked);
    let counter = Arc::clone(&hits);

    let app = Router::new().fallback(any(move |request: Request| {
        let recorder = Arc::clone(&recorder);
        let counter = Arc::clone(&counter);
        async move {
            counter.fetch_add(1, Ordering::Relaxed);
            if let Ok(mut asked) = recorder.lock() {
                *asked = Some(Asked {
                    method: request.method().as_str().to_string(),
                    path: request.uri().path().to_string(),
                    headers: request
                        .headers()
                        .iter()
                        .map(|(k, v)| {
                            (
                                k.as_str().to_string(),
                                v.to_str().unwrap_or_default().to_string(),
                            )
                        })
                        .collect(),
                });
            }

            match verdict {
                Verdict::Allow => axum::response::Response::builder()
                    .status(200)
                    .header("x-user-id", "u-42")
                    .header("x-is-admin", "true")
                    .body(axum::body::Body::empty())
                    .expect("response should build"),
                Verdict::Deny(status) => axum::response::Response::builder()
                    .status(status)
                    .header("content-type", "application/json")
                    .header("www-authenticate", "Bearer realm=\"agents\"")
                    .body(axum::body::Body::from(
                        json!({"reason": "not in group"}).to_string(),
                    ))
                    .expect("response should build"),
            }
        }
    }));

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .expect("authorizer should bind");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    (port, asked, hits)
}

/// An upstream that echoes the headers it saw.
async fn upstream() -> (u16, Arc<AtomicUsize>, Arc<Mutex<Vec<(String, String)>>>) {
    use axum::{Router, extract::Request, routing::any};

    let port = free_port().await;
    let hits = Arc::new(AtomicUsize::new(0));
    let seen = Arc::new(Mutex::new(Vec::new()));

    let counter = Arc::clone(&hits);
    let recorder = Arc::clone(&seen);

    let app = Router::new().fallback(any(move |request: Request| {
        let counter = Arc::clone(&counter);
        let recorder = Arc::clone(&recorder);
        async move {
            counter.fetch_add(1, Ordering::Relaxed);
            if let Ok(mut seen) = recorder.lock() {
                *seen = request
                    .headers()
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.as_str().to_string(),
                            v.to_str().unwrap_or_default().to_string(),
                        )
                    })
                    .collect();
            }
            axum::Json(json!({"served": true}))
        }
    }));

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .expect("upstream should bind");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    (port, hits, seen)
}

/// Start a gateway whose one route carries `extAuthz`.
///
/// The policy arrives as bare YAML lines and is indented here, so a caller
/// never has to get the nesting right in a `format!` string.
async fn start(policy: &[String], upstream_port: u16) -> (String, CancellationToken) {
    let port = free_port().await;
    let policy = policy
        .iter()
        .map(|line| format!("                {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - routes:
          - name: guarded
            policies:
              extAuthz:
{policy}
            backends:
              - host: "127.0.0.1:{upstream_port}"
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
async fn an_allowed_request_reaches_the_upstream() {
    let (authz_port, asked, _) = authorizer(Verdict::Allow).await;
    let (up_port, up_hits, _) = upstream().await;
    let (base, shutdown) = start(
        &[format!("target: \"http://127.0.0.1:{authz_port}\"")],
        up_port,
    )
    .await;

    let response = reqwest::get(format!("{base}/api/thing"))
        .await
        .expect("should reach the gateway");
    assert_eq!(response.status(), 200);
    assert_eq!(up_hits.load(Ordering::Relaxed), 1);

    // The authorizer must see what is being authorized, not just that
    // something is.
    let asked = asked.lock().expect("lock").clone().expect("was asked");
    assert_eq!(asked.method, "GET");
    assert_eq!(asked.path, "/api/thing");

    shutdown.cancel();
}

#[tokio::test]
async fn a_denied_request_never_reaches_the_upstream() {
    let (authz_port, _, _) = authorizer(Verdict::Deny(403)).await;
    let (up_port, up_hits, _) = upstream().await;
    let (base, shutdown) = start(
        &[format!("target: \"http://127.0.0.1:{authz_port}\"")],
        up_port,
    )
    .await;

    let response = reqwest::get(format!("{base}/api/thing"))
        .await
        .expect("should reach the gateway");

    assert_eq!(response.status(), 403);
    assert_eq!(
        up_hits.load(Ordering::Relaxed),
        0,
        "a denied request must stop at the gateway"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn the_authorizers_own_reason_reaches_the_caller() {
    // An authorizer answering `not in group` is telling the caller something a
    // generic "forbidden" would throw away.
    let (authz_port, _, _) = authorizer(Verdict::Deny(403)).await;
    let (up_port, _, _) = upstream().await;
    let (base, shutdown) = start(
        &[format!("target: \"http://127.0.0.1:{authz_port}\"")],
        up_port,
    )
    .await;

    let response = reqwest::get(format!("{base}/api/thing"))
        .await
        .expect("should reach the gateway");
    assert_eq!(
        response
            .headers()
            .get("www-authenticate")
            .and_then(|v| v.to_str().ok()),
        Some("Bearer realm=\"agents\""),
        "a 401/403 without the challenge leaves the client nowhere to go"
    );

    let body: Value = response.json().await.expect("should be JSON");
    assert_eq!(body["reason"], "not in group");

    shutdown.cancel();
}

#[tokio::test]
async fn only_allow_listed_headers_travel_on_to_the_upstream() {
    // Without the list an authorizer could set any header the upstream trusts,
    // which turns authorization into impersonation.
    let (authz_port, _, _) = authorizer(Verdict::Allow).await;
    let (up_port, _, up_headers) = upstream().await;
    let (base, shutdown) = start(
        &[
            format!("target: \"http://127.0.0.1:{authz_port}\""),
            "allowedUpstreamHeaders: [x-user-id]".to_string(),
        ],
        up_port,
    )
    .await;

    reqwest::get(format!("{base}/api/thing"))
        .await
        .expect("should reach the gateway");

    let headers = up_headers.lock().expect("lock").clone();
    let get = |name: &str| {
        headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    };

    assert_eq!(
        get("x-user-id"),
        Some("u-42".to_string()),
        "the resolved identity should travel on"
    );
    assert_eq!(
        get("x-is-admin"),
        None,
        "a header the authorizer set but the policy did not allow must be dropped"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn nothing_travels_on_without_an_allow_list() {
    let (authz_port, _, _) = authorizer(Verdict::Allow).await;
    let (up_port, _, up_headers) = upstream().await;
    let (base, shutdown) = start(
        &[format!("target: \"http://127.0.0.1:{authz_port}\"")],
        up_port,
    )
    .await;

    reqwest::get(format!("{base}/api/thing"))
        .await
        .expect("should reach the gateway");

    let headers = up_headers.lock().expect("lock").clone();
    assert!(
        !headers.iter().any(|(k, _)| k == "x-user-id"),
        "an empty allow-list allows nothing, not everything: {headers:?}"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn only_allow_listed_request_headers_reach_the_authorizer() {
    let (authz_port, asked, _) = authorizer(Verdict::Allow).await;
    let (up_port, _, _) = upstream().await;
    let (base, shutdown) = start(
        &[
            format!("target: \"http://127.0.0.1:{authz_port}\""),
            "includeHeaders: [authorization]".to_string(),
        ],
        up_port,
    )
    .await;

    reqwest::Client::new()
        .get(format!("{base}/api/thing"))
        .header("authorization", "Bearer token")
        .header("cookie", "session=secret")
        .send()
        .await
        .expect("should reach the gateway");

    let asked = asked.lock().expect("lock").clone().expect("was asked");
    let has = |name: &str| asked.headers.iter().any(|(k, _)| k == name);

    assert!(
        has("authorization"),
        "the listed header should be forwarded"
    );
    assert!(
        !has("cookie"),
        "the authorizer has no need for a session cookie: {:?}",
        asked.headers
    );

    shutdown.cancel();
}

#[tokio::test]
async fn an_unreachable_authorizer_denies_by_default() {
    // The decision this whole policy turns on: an authorization service that
    // is down must not become an open door.
    let dead = free_port().await;
    let (up_port, up_hits, _) = upstream().await;
    let (base, shutdown) = start(&[format!("target: \"http://127.0.0.1:{dead}\"")], up_port).await;

    let response = reqwest::get(format!("{base}/api/thing"))
        .await
        .expect("the gateway should still answer");

    assert_eq!(
        response.status(),
        503,
        "nothing decided this request was forbidden, so 503 rather than 403"
    );
    assert_eq!(up_hits.load(Ordering::Relaxed), 0);

    shutdown.cancel();
}

#[tokio::test]
async fn failing_open_has_to_be_asked_for() {
    let dead = free_port().await;
    let (up_port, up_hits, _) = upstream().await;
    let (base, shutdown) = start(
        &[
            format!("target: \"http://127.0.0.1:{dead}\""),
            "failOpen: true".to_string(),
        ],
        up_port,
    )
    .await;

    let response = reqwest::get(format!("{base}/api/thing"))
        .await
        .expect("should reach the gateway");

    assert_eq!(response.status(), 200);
    assert_eq!(
        up_hits.load(Ordering::Relaxed),
        1,
        "failOpen serves what the authorizer never approved -- deliberately"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn a_slow_authorizer_hits_its_budget_and_denies() {
    let port = free_port().await;
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .expect("should bind");
    tokio::spawn(async move {
        let app = axum::Router::new().fallback(axum::routing::any(|| async {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            "too late"
        }));
        let _ = axum::serve(listener, app).await;
    });

    let (up_port, up_hits, _) = upstream().await;
    let (base, shutdown) = start(
        &[
            format!("target: \"http://127.0.0.1:{port}\""),
            "timeout: 200ms".to_string(),
        ],
        up_port,
    )
    .await;

    let started = std::time::Instant::now();
    let response = reqwest::get(format!("{base}/api/thing"))
        .await
        .expect("the gateway should answer");

    assert_eq!(response.status(), 503);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "the budget should fire long before the 10s authorizer; took {:?}",
        started.elapsed()
    );
    assert_eq!(up_hits.load(Ordering::Relaxed), 0);

    shutdown.cancel();
}

#[tokio::test]
async fn a_target_that_is_not_a_url_fails_at_startup() {
    // Rather than turning every request into a confusing 503.
    let port = free_port().await;
    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - routes:
          - policies:
              extAuthz:
                target: "authz:9000"
            backends:
              - host: "127.0.0.1:1"
"#
    );

    let config = Config::from_yaml(&yaml).expect("should parse");
    let err = Gateway::build(&config, None)
        .await
        .map(|_| ())
        .expect_err("a bare host:port is not a URL");
    assert!(err.to_string().contains("authz:9000"), "got: {err}");
}
