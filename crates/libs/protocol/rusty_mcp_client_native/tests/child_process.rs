#![allow(clippy::unwrap_used)]
//! The client against a real child process: the example server over its
//! stdin and stdout.

use rusty_mcp_client_native::json::Value;
use rusty_mcp_client_native::proto::ContentBlock;
use rusty_mcp_client_native::{Client, ClientConfig, ClientError, NoHandler, StdioTransport};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

/// The example server, rebuilt if stale (`cargo test` alone does not build
/// examples, and a leftover binary would test the wrong server).
fn server_binary() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let profile_dir = exe.parent().unwrap().parent().unwrap();
    let mut build = Command::new(env!("CARGO"));
    build.args([
        "build",
        "--example",
        "server",
        "-p",
        "rusty_mcp_client_native",
    ]);
    if profile_dir.file_name().is_some_and(|n| n == "release") {
        build.arg("--release");
    }
    assert!(
        build.status().unwrap().success(),
        "building the example failed"
    );
    let bin = profile_dir
        .join("examples")
        .join(format!("server{}", std::env::consts::EXE_SUFFIX));
    assert!(bin.exists(), "{} missing after the build", bin.display());
    bin
}

fn add(a: i64, b: i64) -> Option<Value> {
    let mut v = Value::object();
    v.insert("a", a);
    v.insert("b", b);
    Some(v)
}

fn connect(config: ClientConfig) -> Client<StdioTransport, NoHandler> {
    let transport = StdioTransport::spawn(Command::new(server_binary())).unwrap();
    Client::connect(transport, config, NoHandler, Duration::from_secs(10)).unwrap()
}

#[test]
fn both_handshakes_work_against_a_child_process() {
    let mut stateless = connect(ClientConfig::new("c", "1"));
    assert_eq!(
        stateless.session().negotiated().unwrap().as_str(),
        "2026-07-28"
    );
    let r = stateless.call_tool("add", add(40, 2)).unwrap();
    assert!(matches!(&r.content[0], ContentBlock::Text { text, .. } if text == "42"));

    let mut classic_config = ClientConfig::new("c", "1");
    classic_config.versions.retain(|v| !v.is_stateless());
    let mut classic = connect(classic_config);
    assert_eq!(
        classic.session().negotiated().unwrap().as_str(),
        "2025-11-25"
    );
    assert_eq!(classic.list_tools().unwrap()[0].name, "add");
}

#[test]
fn dropping_the_client_ends_the_child_promptly() {
    let started = Instant::now();
    let client = connect(ClientConfig::new("c", "1"));
    drop(client);
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "the child was not given EOF and stopped"
    );
}

#[test]
fn a_program_that_is_not_a_server_is_reported_not_hung() {
    // `cat` echoes our own messages back: our requests look like the server's
    // (refused with method-not-found, which is echoed back as the "answer"),
    // so the handshake fails with that error instead of waiting forever.
    let Ok(transport) = StdioTransport::spawn(Command::new("cat")) else {
        return; // no `cat` here
    };
    let err = Client::connect(
        transport,
        ClientConfig::new("c", "1"),
        NoHandler,
        Duration::from_millis(300),
    )
    .err()
    .unwrap();
    assert!(
        matches!(err, ClientError::Timeout | ClientError::Rpc(_)),
        "{err}"
    );
}

#[test]
fn a_missing_program_is_an_io_error() {
    let err = StdioTransport::spawn(Command::new("/no/such/mcp-server")).err();
    assert!(err.is_some());
}
