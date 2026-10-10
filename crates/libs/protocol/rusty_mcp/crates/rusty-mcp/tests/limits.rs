#![allow(clippy::unwrap_used)]
//! Load shedding over a real socket, in front of a real MCP server.
//!
//! The unit tests drive the layer directly. These answer the questions only
//! the whole stack can: does shedding happen ahead of the authorization check,
//! and — the one worth being empirical about — does a timeout kill a
//! long-lived `subscriptions/listen`?
//!
//! That last one cannot be settled by reading the layer. It depends on whether
//! the transport returns the SSE response promptly and streams afterwards, or
//! holds the response future open for the life of the subscription. If it were
//! the latter, a timeout would silently break change notifications for anyone
//! who enabled one.

mod support;

use std::{sync::Arc, time::Duration};

use rusty_mcp::auth::{AuthConfig, RequireAuthLayer, StaticTokenValidator, VerifiedToken};
use rusty_mcp::limits::LimitsLayer;
use support::{call, post, spawn, tools_list};

fn sleep_call(ms: u64) -> String {
    call("sleep", &format!(r#"{{"ms":{ms}}}"#))
}

#[tokio::test(flavor = "multi_thread")]
async fn the_layer_sheds_when_the_server_is_full() {
    let (addr, _) = spawn(|mcp| mcp.layer(LimitsLayer::new().with_max_concurrent(1))).await;

    let held = tokio::spawn(async move {
        post(addr, "tools/call", Some("sleep"), sleep_call(400), None).await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;

    let shed = post(addr, "tools/list", None, tools_list(), None).await;
    assert_eq!(shed.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        shed.headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok()),
        Some("1"),
        "a shed response should tell the client when to come back"
    );

    assert!(held.await.unwrap().status().is_success());
}

#[tokio::test(flavor = "multi_thread")]
async fn capacity_returns_once_the_slow_request_finishes() {
    // A limit that did not release would turn one slow call into an outage.
    let (addr, _) = spawn(|mcp| mcp.layer(LimitsLayer::new().with_max_concurrent(1))).await;

    assert!(
        post(addr, "tools/call", Some("sleep"), sleep_call(50), None)
            .await
            .status()
            .is_success()
    );
    for _ in 0..3 {
        assert!(
            post(addr, "tools/list", None, tools_list(), None)
                .await
                .status()
                .is_success(),
            "capacity should have come back"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn shedding_happens_before_the_token_is_checked() {
    // The ordering claim. Limits sit outside authorization, so an unauthorized
    // request that arrives while the server is full is shed with `503` and
    // never costs a validation — under a flood, that difference is the point.
    let validator = StaticTokenValidator::new().with_token(
        "good-token",
        VerifiedToken::new(["https://mcp.example.com/mcp"]),
    );
    let auth = AuthConfig::new("https://mcp.example.com/mcp", Arc::new(validator))
        .unwrap()
        .with_authorization_servers(["https://auth.example.com"]);

    let (addr, _) = spawn(|mcp| {
        mcp.layer(RequireAuthLayer::new(auth))
            .layer(LimitsLayer::new().with_max_concurrent(1))
    })
    .await;

    // Fill the one slot with an authorized request.
    let held = tokio::spawn(async move {
        post(
            addr,
            "tools/call",
            Some("sleep"),
            sleep_call(400),
            Some("good-token"),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;

    // No token at all. If auth ran first this would be a 401.
    let response = post(addr, "tools/list", None, tools_list(), None).await;
    assert_eq!(
        response.status(),
        reqwest::StatusCode::SERVICE_UNAVAILABLE,
        "a 401 here would mean the token was validated before capacity was checked"
    );
    assert!(held.await.unwrap().status().is_success());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_inline_tool_that_runs_too_long_is_cut_off() {
    let (addr, _) =
        spawn(|mcp| mcp.layer(LimitsLayer::new().with_timeout(Duration::from_millis(150)))).await;

    let response = post(addr, "tools/call", Some("sleep"), sleep_call(2_000), None).await;
    assert_eq!(response.status(), reqwest::StatusCode::GATEWAY_TIMEOUT);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_inside_the_timeout_is_untouched() {
    let (addr, _) =
        spawn(|mcp| mcp.layer(LimitsLayer::new().with_timeout(Duration::from_secs(10)))).await;

    let response = post(addr, "tools/call", Some("sleep"), sleep_call(20), None).await;
    assert!(response.status().is_success());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_long_lived_subscription_survives_a_short_timeout() {
    // `subscriptions/listen` is long-lived by design; if the timeout wrapped
    // the whole subscription rather than the response that starts it, enabling
    // a timeout would silently break change notifications for everyone who
    // turned one on.
    const TIMEOUT: Duration = Duration::from_millis(200);

    let (addr, changes) = spawn(|mcp| mcp.layer(LimitsLayer::new().with_timeout(TIMEOUT))).await;

    let body = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"subscriptions/listen","params":{{"notifications":{{"resourcesListChanged":true}},"_meta":{{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{{}}}}}}}}"#
    );
    let mut stream = post(addr, "subscriptions/listen", None, body, None).await;
    assert!(stream.status().is_success());

    // The acknowledgement, promptly.
    let ack = tokio::time::timeout(Duration::from_secs(5), stream.chunk())
        .await
        .expect("the acknowledgement arrives")
        .unwrap()
        .expect("a first chunk");
    assert!(String::from_utf8_lossy(&ack).contains("acknowledged"));

    // Well past the timeout, so a timeout covering the whole subscription
    // would already have killed it.
    tokio::time::sleep(TIMEOUT * 5).await;
    changes.resources_changed();

    let event = tokio::time::timeout(Duration::from_secs(5), stream.chunk())
        .await
        .expect("an event should arrive long after the timeout would have fired")
        .unwrap()
        .expect("the subscription should still be live");
    assert!(String::from_utf8_lossy(&event).contains("resources/list_changed"));

    // End the stream so its handler thread does not outlive the test.
    changes.close();
}

#[tokio::test(flavor = "multi_thread")]
async fn no_limits_configured_leaves_the_server_unbounded() {
    // The default. Nothing here should change for a server that never opts in.
    let (addr, _) = spawn(|mcp| mcp.layer(LimitsLayer::new())).await;

    let handles: Vec<_> = (0..8)
        .map(|_| {
            tokio::spawn(async move {
                post(addr, "tools/call", Some("sleep"), sleep_call(100), None).await
            })
        })
        .collect();
    for handle in handles {
        assert!(handle.await.unwrap().status().is_success());
    }
}
