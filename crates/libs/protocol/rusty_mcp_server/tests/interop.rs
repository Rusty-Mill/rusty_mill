#![allow(clippy::unwrap_used)]
//! The server against an independent client: the `rmcp` client, over a real
//! child process and real stdio, in both handshake modes. This is the
//! evidence that the server's reading of the spec matches the library the
//! other MCP crates use today.

mod common;

use common::{exercise, Mode, Recorder};
use rmcp::model::ProtocolVersion;
use rmcp::service::ClientLifecycleMode;
use rmcp::transport::TokioChildProcess;
use rmcp::{ClientServiceExt, ServiceExt};
use std::path::PathBuf;
use tokio::process::Command;

/// The example server, rebuilt if it is stale: a binary left over from an
/// older source would test the wrong server. `cargo build` is a no-op when
/// the example is current, and `cargo test --test interop` alone does not
/// build examples at all.
fn server_binary() -> PathBuf {
    // target/<profile>/deps/interop-<hash> -> target/<profile>/examples/interop_server
    let exe = std::env::current_exe().unwrap();
    let profile_dir = exe.parent().unwrap().parent().unwrap();
    let mut build = std::process::Command::new(env!("CARGO"));
    build.args([
        "build",
        "--example",
        "interop_server",
        "-p",
        "rusty_mcp_server",
    ]);
    if profile_dir.file_name().is_some_and(|n| n == "release") {
        build.arg("--release");
    }
    let status = build.status().unwrap();
    assert!(status.success(), "building the example server failed");
    let bin = profile_dir
        .join("examples")
        .join(format!("interop_server{}", std::env::consts::EXE_SUFFIX));
    assert!(bin.exists(), "{} missing after the build", bin.display());
    bin
}

fn unique_path(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("interop-{tag}-{}-{nanos}", std::process::id()))
}

async fn run(mode: Mode) {
    let cancel_file = unique_path("cancel");
    let mut command = Command::new(server_binary());
    command.env("INTEROP_CANCEL_FILE", &cancel_file);
    let transport = TokioChildProcess::new(command).unwrap();
    let recorder = Recorder::default();
    let client = match mode {
        Mode::Classic => recorder.clone().serve(transport).await.unwrap(),
        Mode::Stateless => recorder
            .clone()
            .serve_with_lifecycle(
                transport,
                ClientLifecycleMode::Discover {
                    preferred_versions: vec![ProtocolVersion::V_2026_07_28],
                },
            )
            .await
            .unwrap(),
    };
    let seen = cancel_file.clone();
    exercise(mode, client, recorder, &move || seen.exists()).await;
    let _ = std::fs::remove_file(&cancel_file);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_rmcp_client_works_against_the_server_with_the_initialize_handshake() {
    run(Mode::Classic).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_rmcp_client_works_against_the_server_with_server_discover() {
    run(Mode::Stateless).await;
}
