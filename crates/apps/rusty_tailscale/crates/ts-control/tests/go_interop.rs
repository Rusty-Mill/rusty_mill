//! Byte-exact interop test: handshake and exchange transport records with a
//! Noise responder built on the REAL Go `tailscale.com/control/controlbase`
//! package (see `interop/noise-server-go/`).
//!
//! Requires the Go server binary; builds it on demand with `go build` if a
//! toolchain is available, otherwise the test is skipped with a loud
//! message (interop against live Headscale still covers the same code in
//! the Phase-2 end-to-end verification).

use std::path::PathBuf;
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::process::Command;
use ts_control::controlbase;
use ts_key::MachinePrivate;

const PROTOCOL_VERSION: u16 = 123;

fn server_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../interop/noise-server-go")
}

async fn ensure_server_binary() -> Option<PathBuf> {
    let dir = server_dir();
    let bin = dir.join("noise-server-go");
    if bin.exists() {
        return Some(bin);
    }
    let go = ["/usr/local/go/bin/go", "go"].iter().find(|g| {
        std::process::Command::new(g)
            .arg("version")
            .output()
            .is_ok()
    })?;
    let status = Command::new(go)
        .args(["build", "-o", "noise-server-go", "."])
        .current_dir(&dir)
        .status()
        .await
        .ok()?;
    status.success().then_some(bin)
}

#[tokio::test]
async fn handshake_and_echo_against_real_go_controlbase() {
    let Some(bin) = ensure_server_binary().await else {
        eprintln!("SKIPPED: no Go toolchain and no prebuilt noise-server-go binary");
        return;
    };

    let mut child = Command::new(&bin)
        .arg("-listen")
        .arg("127.0.0.1:0")
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn go noise server");

    // Parse "CONTROL_KEY mkey:<hex>" and "LISTENING <addr>".
    let stdout = child.stdout.take().unwrap();
    let mut lines = BufReader::new(stdout).lines();
    let control_key_line = lines.next_line().await.unwrap().expect("control key line");
    let listen_line = lines.next_line().await.unwrap().expect("listen line");
    let control_key: ts_types::MachinePublic = control_key_line
        .strip_prefix("CONTROL_KEY ")
        .expect("CONTROL_KEY prefix")
        .parse()
        .expect("valid mkey");
    let addr = listen_line
        .strip_prefix("LISTENING ")
        .expect("LISTENING prefix");

    // Handshake with the real Go responder.
    let machine_key = MachinePrivate::generate();
    let tcp = TcpStream::connect(addr).await.expect("connect");
    let mut conn = controlbase::connect(tcp, &machine_key, &control_key.0, PROTOCOL_VERSION)
        .await
        .expect("noise handshake against Go controlbase");

    // Go sends "GO-OK <client mkey>\n" — proves it authenticated *us*.
    let expected_banner = format!("GO-OK {}\n", machine_key.public());
    let mut banner = vec![0u8; expected_banner.len()];
    conn.read_exact(&mut banner).await.expect("read banner");
    assert_eq!(String::from_utf8_lossy(&banner), expected_banner);

    // Echo across record boundaries: > 4077 bytes forces multiple records
    // in each direction and exercises both nonce counters past 0.
    let payload: Vec<u8> = (0..10_000u32).map(|i| (i % 253) as u8).collect();
    conn.write_all(&payload).await.expect("write payload");
    conn.flush().await.expect("flush");
    let mut echoed = vec![0u8; payload.len()];
    conn.read_exact(&mut echoed).await.expect("read echo");
    assert_eq!(
        echoed, payload,
        "echo through Go transport must be lossless"
    );
}

#[tokio::test]
async fn wrong_control_key_fails_cleanly() {
    let Some(bin) = ensure_server_binary().await else {
        eprintln!("SKIPPED: no Go toolchain and no prebuilt noise-server-go binary");
        return;
    };

    let mut child = Command::new(&bin)
        .arg("-listen")
        .arg("127.0.0.1:0")
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn go noise server");
    let stdout = child.stdout.take().unwrap();
    let mut lines = BufReader::new(stdout).lines();
    let _ = lines.next_line().await.unwrap();
    let listen_line = lines.next_line().await.unwrap().expect("listen line");
    let addr = listen_line.strip_prefix("LISTENING ").unwrap();

    // Handshake against the wrong static key: the server can't decrypt our
    // initiation; we must get an error, never a hang or panic.
    let machine_key = MachinePrivate::generate();
    let wrong_key = MachinePrivate::generate().public().0;
    let tcp = TcpStream::connect(addr).await.expect("connect");
    let res = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        controlbase::connect(tcp, &machine_key, &wrong_key, PROTOCOL_VERSION),
    )
    .await
    .expect("must not hang");
    assert!(res.is_err(), "handshake with wrong control key must fail");
}
