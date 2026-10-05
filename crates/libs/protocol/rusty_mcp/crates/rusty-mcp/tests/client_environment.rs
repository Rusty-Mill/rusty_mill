//! Environment-backed client credentials, isolated from parallel test processes.
#![cfg(feature = "client")]

use rusty_mcp::client::{McpClient, McpClientError, McpServerSpec, McpTransport};
use rusty_mcp::client_auth::{AuthError, McpAuth, McpAuthSecret, resolve};

const CHILD_CASE: &str = "RUSTY_MCP_ENV_TEST_CASE";
const SECRET: &str = "RUSTY_MCP_ENV_TEST_SECRET";

// Re-execute only this test with a child-specific environment. Command::env
// changes the child's startup environment, never the shared test process's.
fn in_child(case: &str, value: Option<&str>) -> bool {
    if std::env::var(CHILD_CASE).as_deref() == Ok(case) {
        return true;
    }
    let mut command = std::process::Command::new(std::env::current_exe().expect("test executable"));
    command.args(["--exact", case, "--nocapture"]);
    command.env(CHILD_CASE, case);
    match value {
        Some(value) => command.env(SECRET, value),
        None => command.env_remove(SECRET),
    };
    let output = command.output().expect("run isolated credential test");
    assert!(
        output.status.success(),
        "{case} failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    false
}

fn env_auth() -> McpAuth {
    McpAuth::Bearer {
        token: McpAuthSecret::Env {
            env: SECRET.to_owned(),
        },
    }
}

#[tokio::test]
async fn env_indirection_resolves_when_set() {
    if !in_child(
        "env_indirection_resolves_when_set",
        Some("synthetic-from-env"),
    ) {
        return;
    }
    let resolved = resolve(&env_auth())
        .await
        .expect("resolve environment token");
    assert_eq!(
        resolved.authorization.as_deref(),
        Some("Bearer synthetic-from-env")
    );
    assert!(resolved.extra_headers.is_empty());
}

#[tokio::test]
async fn env_indirection_errors_when_unset() {
    if !in_child("env_indirection_errors_when_unset", None) {
        return;
    }
    let error = resolve(&env_auth())
        .await
        .expect_err("missing token must fail");
    assert!(matches!(error, AuthError::MissingEnv { name } if name == SECRET));
}

#[tokio::test]
async fn http_transport_auth_missing_env_surfaces_as_auth_error() {
    if !in_child(
        "http_transport_auth_missing_env_surfaces_as_auth_error",
        None,
    ) {
        return;
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let spec = McpServerSpec {
        transport: McpTransport::Http,
        url: Some(format!(
            "http://{}/mcp",
            listener.local_addr().expect("address")
        )),
        auth: Some(env_auth()),
        ..McpServerSpec::default()
    };
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        McpClient::connect("missing-secret", &spec),
    )
    .await
    .expect("auth must fail promptly")
    .expect_err("missing token must fail");
    assert!(
        matches!(error, McpClientError::Auth(AuthError::MissingEnv { name }) if name == SECRET)
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept())
            .await
            .is_err(),
        "missing credentials must fail before contacting the server"
    );
}
