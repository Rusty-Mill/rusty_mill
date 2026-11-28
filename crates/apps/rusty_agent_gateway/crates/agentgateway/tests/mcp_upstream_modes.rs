//! End-to-end tests for a target's `mode` (Let Em Loose #1377, D223).
//!
//! The pool tests in `agentgateway-mcp` cover the keying. These cover the
//! wiring: two client sessions through the gateway reach two different child
//! processes of a `per_session` target, one session keeps its own child,
//! ending a session (`DELETE`) kills that child, and a stateless call gets a
//! child for that call alone.

use std::net::SocketAddr;

use agentgateway_config::Config;
use rusty_tokio::sync::CancellationToken;

mod common;
use common::mcp;
use common::{free_port, yaml_path};

fn mock_server() -> String {
    let mut path = std::env::current_exe().expect("test binary should have a path");
    path.pop();
    path.pop();
    path.push("examples");
    path.push(format!("mock_mcp_server{}", std::env::consts::EXE_SUFFIX));
    assert!(path.exists(), "fixture not built at {}", path.display());
    yaml_path(&path)
}

/// A gateway with one `stdio` target in `mode` (none when `None`).
async fn gateway(mode: Option<&str>) -> (String, CancellationToken) {
    let port = free_port();
    let mode = mode.map_or_else(String::new, |m| {
        format!("\n                      mode: {m}")
    });
    let yaml = format!(
        r#"
binds:
  - port: {port}
    listeners:
      - routes:
          - name: mcp
            matches:
              - path:
                  pathPrefix: /mcp
            backends:
              - mcp:
                  targets:
                    - name: box
                      stdio:
                        cmd: "{server}"
                        env:
                          MOCK_LABEL: box
                          MOCK_TOOLS: "whoami"
                          MOCK_ECHO_PID: "1"{mode}
"#,
        server = mock_server()
    );
    let config = Config::from_yaml(&yaml).expect("config should parse");
    config.validate().expect("config should validate");
    let gateway = common::build(&config, None)
        .await
        .expect("gateway should build");
    let shutdown = CancellationToken::new();
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("should parse");
    let _serving = common::serve(gateway, vec![addr], shutdown.clone())
        .await
        .expect("gateway should bind");
    (format!("http://127.0.0.1:{port}/mcp"), shutdown)
}

/// A classic client, so the gateway gives it an `Mcp-Session-Id`.
async fn session(url: &str) -> mcp::Client {
    mcp::connect_classic(mcp::HttpConfig::new(url))
        .await
        .expect("client should complete the MCP handshake")
}

/// Which child answered.
async fn pid(client: &mcp::Client) -> u32 {
    let result = client
        .call_tool("box_whoami", None)
        .await
        .expect("the call should succeed");
    let text = mcp::text(&result.content);
    text.rsplit_once("pid=")
        .and_then(|(_, pid)| pid.trim().parse().ok())
        .unwrap_or_else(|| panic!("no pid in {text:?}"))
}

#[cfg(target_os = "linux")]
fn alive(pid: u32) -> bool {
    // A reaped child has no /proc entry; a zombie would still show its state.
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .map(|stat| !stat.contains(") Z "))
        .unwrap_or(false)
}

#[rusty_tokio::test]
async fn a_target_without_mode_gives_each_session_its_own_child() {
    let (url, shutdown) = gateway(None).await;
    let a = session(&url).await;
    let b = session(&url).await;

    let first = pid(&a).await;
    assert_eq!(pid(&a).await, first, "a session keeps its child");
    let other = pid(&b).await;
    assert_ne!(other, first, "two sessions never share a per_session child");

    #[cfg(target_os = "linux")]
    {
        use std::time::{Duration, Instant};

        assert!(alive(first));
        let _ = a.close().await; // sends DELETE
        let deadline = Instant::now() + Duration::from_secs(5);
        while alive(first) && Instant::now() < deadline {
            rusty_tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(!alive(first), "ending the session kills its child");
        assert!(alive(other), "the other session's child is untouched");
    }

    let _ = b.close().await;
    shutdown.cancel();
}

#[rusty_tokio::test]
async fn a_stateless_call_gets_a_child_for_that_call_alone() {
    let (url, shutdown) = gateway(None).await;
    // The default client speaks the stateless revision: no session.
    let client = mcp::connect(url.as_str())
        .await
        .expect("client should complete the MCP handshake");
    let first = pid(&client).await;
    let second = pid(&client).await;
    assert_ne!(first, second, "no state carries from one call to the next");

    #[cfg(target_os = "linux")]
    {
        use std::time::{Duration, Instant};

        let deadline = Instant::now() + Duration::from_secs(5);
        while (alive(first) || alive(second)) && Instant::now() < deadline {
            rusty_tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(
            !alive(first) && !alive(second),
            "each child ends with its call"
        );
    }
    shutdown.cancel();
}

#[rusty_tokio::test]
async fn a_pooled_target_shares_one_child() {
    let (url, shutdown) = gateway(Some("pooled")).await;
    let a = session(&url).await;
    let b = session(&url).await;
    assert_eq!(pid(&a).await, pid(&b).await);
    let _ = a.close().await;
    let _ = b.close().await;
    shutdown.cancel();
}
