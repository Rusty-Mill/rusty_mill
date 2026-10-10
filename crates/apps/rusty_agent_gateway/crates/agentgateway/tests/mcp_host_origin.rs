//! The MCP handler behind a route does not second-guess which hosts and
//! browser origins reach it: that is the route's matching and `cors` policy,
//! applied before the handler. A public virtual host and a browser origin must
//! get through; `cors` still decides which origins a browser may read.

use std::net::SocketAddr;

use agentgateway::{Gateway, serve};
use agentgateway_config::Config;
use serde_json::json;
use tokio_util::sync::CancellationToken;

mod common;
use common::free_port;

fn mock_server() -> String {
    let mut path = std::env::current_exe().expect("test binary should have a path");
    path.pop();
    path.pop();
    path.push("examples");
    path.push(format!("mock_mcp_server{}", std::env::consts::EXE_SUFFIX));
    path.display().to_string().replace('\\', "\\\\")
}

/// A gateway with one MCP route whose CORS allows only `https://app.example`.
async fn start() -> (String, CancellationToken) {
    let port = free_port().await;
    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - routes:
          - matches:
              - path:
                  pathPrefix: /mcp
            policies:
              cors:
                allowOrigins: ["https://app.example"]
            backends:
              - mcp:
                  targets:
                    - name: alpha
                      stdio:
                        cmd: "{server}"
                        env:
                          MOCK_LABEL: alpha
"#,
        server = mock_server(),
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
    (format!("http://127.0.0.1:{port}/mcp"), shutdown)
}

async fn initialize(url: &str, host: &str, origin: &str) -> reqwest::Response {
    reqwest::Client::new()
        .post(url)
        .header("host", host)
        .header("origin", origin)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .json(&json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "host-origin-test", "version": "1"}
            }
        }))
        .send()
        .await
        .expect("the request should reach the gateway")
}

#[tokio::test]
async fn a_public_host_and_a_browser_origin_reach_the_handler() {
    let (url, shutdown) = start().await;

    let response = initialize(&url, "mcp.public.example", "https://app.example").await;
    assert_eq!(
        response.status(),
        200,
        "the handler must not refuse a public host"
    );
    assert_eq!(
        response
            .headers()
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok()),
        Some("https://app.example"),
        "a permitted origin is answered with its CORS headers"
    );
    assert!(
        response
            .text()
            .await
            .expect("should read the body")
            .contains("protocolVersion"),
        "and gets the real initialize result"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn an_origin_cors_does_not_permit_gets_no_cors_headers() {
    let (url, shutdown) = start().await;

    // The request is served (CORS is enforced by the browser from these
    // headers, as before), but nothing says the page may read the answer.
    let response = initialize(&url, "mcp.public.example", "https://evil.example").await;
    assert_eq!(response.status(), 200);
    assert!(
        response
            .headers()
            .get("access-control-allow-origin")
            .is_none(),
        "a rejected origin must not be granted access: {:?}",
        response.headers()
    );

    shutdown.cancel();
}
