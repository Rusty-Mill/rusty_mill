//! `rusty-mux-ctl` against a real server: output, chains, events and every
//! exit code (0, 1, 2, 3, 4, and 130 from an interrupt).

#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixListener as StdListener;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use rusty_mux_server::{Config, serve};
use rusty_tokio::io::UnixListener;
use rusty_tokio::{Builder, LocalOptions};

const PATIENCE: Duration = Duration::from_secs(20);

fn socket_path(tag: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "rusty-mux-ctl-{tag}-{}-{n}.sock",
        std::process::id()
    ))
}

fn start_server() -> PathBuf {
    let path = socket_path("srv");
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let bind_path = path.clone();
    std::thread::spawn(move || {
        let runtime = Builder::new_current_thread()
            .build_local(&LocalOptions::default())
            .expect("runtime");
        runtime.block_on(async move {
            let listener = UnixListener::bind(&bind_path).expect("bind");
            ready_tx.send(()).expect("signal ready");
            let config = Config {
                program: "/bin/sh".into(),
                args: Vec::new(),
                cwd: "/".into(),
            };
            serve(listener, config).await.expect("serve");
        });
    });
    ready_rx.recv_timeout(PATIENCE).expect("server bound");
    path
}

fn ctl_command(socket: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rusty-mux-ctl"));
    command
        .env_remove("RUSTY_MUX_SOCKET")
        .arg("--socket")
        .arg(socket)
        .args(args);
    command
}

fn ctl(socket: &Path, args: &[&str]) -> Output {
    ctl_command(socket, args)
        .output()
        .expect("run rusty-mux-ctl")
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("utf-8")
}

fn ok(socket: &Path, args: &[&str]) -> String {
    let output = ctl(socket, args);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout(&output).trim().to_owned()
}

/// Creates a session, window and block; returns the block id.
fn block(socket: &Path) -> String {
    let session = ok(socket, &["create-session", "--name", "s"]);
    let window = ok(
        socket,
        &["create-window", "--session", &session, "--name", "w"],
    );
    ok(
        socket,
        &[
            "create-block",
            "--window",
            &window,
            "--cols",
            "80",
            "--rows",
            "24",
        ],
    )
}

#[test]
fn a_request_prints_its_new_id() {
    let socket = start_server();
    let session = ok(&socket, &["create-session", "--name", "main"]);
    assert!(session.parse::<rusty_uuid::Uuid>().is_ok(), "{session}");

    let json = ok(&socket, &["--json", "create-session", "--name", "other"]);
    let value = rusty_json::Value::from_json_str(&json).expect("json");
    let rusty_json::Value::Object(map) = value else {
        panic!("object")
    };
    assert_eq!(map.get("ok"), Some(&rusty_json::Value::Bool(true)));
    assert!(map.get("created").is_some());
}

#[test]
fn a_chain_takes_the_lease_and_types() {
    let socket = start_server();
    let block = block(&socket);
    let chain = [
        "attach",
        "--block",
        &block,
        "+",
        "acquire-lease",
        "--block",
        &block,
        "+",
        "input",
        "--block",
        &block,
        "--data",
        "true\r",
    ];
    let output = ctl(&socket, &chain);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_refused_request_exits_1() {
    let socket = start_server();
    let block = block(&socket);
    // Unanswered input without the lease: the refusal still surfaces.
    let output = ctl(&socket, &["input", "--block", &block, "--data", "x"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("input:"));

    let missing = "00000000-0000-0000-0000-000000000001";
    let output = ctl(
        &socket,
        &[
            "--json",
            "create-window",
            "--session",
            missing,
            "--name",
            "w",
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).contains("\"ok\":false"));
}

/// Whether `block` still exists: only an existing block can be attached.
fn exists(socket: &Path, block: &str) -> bool {
    ctl(socket, &["attach", "--block", block]).status.code() == Some(0)
}

#[test]
fn a_refused_input_stops_the_chain() {
    let socket = start_server();
    let block = block(&socket);
    // Without the lease the input is refused, so close-block is never sent.
    let chain = [
        "input",
        "--block",
        &block,
        "--data",
        "x",
        "+",
        "close-block",
        "--block",
        &block,
    ];
    assert_eq!(ctl(&socket, &chain).status.code(), Some(1));
    assert!(
        exists(&socket, &block),
        "the block outlived the refused chain"
    );
}

#[test]
fn each_input_is_confirmed_in_order() {
    let socket = start_server();
    let block = block(&socket);
    let chain = [
        "--json",
        "attach",
        "--block",
        &block,
        "+",
        "acquire-lease",
        "--block",
        &block,
        "+",
        "input",
        "--block",
        &block,
        "--data",
        "true",
        "+",
        "input",
        "--block",
        &block,
        "--data",
        "\r",
        "+",
        "release-lease",
        "--block",
        &block,
        "+",
        "input",
        "--block",
        &block,
        "--data",
        "x",
        "+",
        "close-block",
        "--block",
        &block,
    ];
    let output = ctl(&socket, &chain);
    assert_eq!(output.status.code(), Some(1));
    let methods: Vec<(String, bool)> = stdout(&output)
        .lines()
        .map(|line| {
            let rusty_json::Value::Object(map) =
                rusty_json::Value::from_json_str(line).expect("json")
            else {
                panic!("object")
            };
            let method = match map.get("method") {
                Some(rusty_json::Value::String(m)) => m.clone(),
                other => panic!("method: {other:?}"),
            };
            (
                method,
                map.get("ok") == Some(&rusty_json::Value::Bool(true)),
            )
        })
        .collect();
    let expect = [
        ("attach", true),
        ("acquire-lease", true),
        ("input", true),
        ("input", true),
        ("release-lease", true),
        ("input", false),
    ];
    let expect: Vec<(String, bool)> = expect.iter().map(|(m, ok)| (m.to_string(), *ok)).collect();
    assert_eq!(methods, expect);
    assert!(
        exists(&socket, &block),
        "close-block after the refusal never ran"
    );
}

#[test]
fn a_bad_command_line_exits_2() {
    let socket = start_server();
    for args in [
        &["frobnicate"][..],
        &["create-session"],
        &["resize", "--block", "x"],
    ] {
        assert_eq!(ctl(&socket, args).status.code(), Some(2), "{args:?}");
    }
    let output = Command::new(env!("CARGO_BIN_EXE_rusty-mux-ctl"))
        .env_remove("RUSTY_MUX_SOCKET")
        .args(["create-session", "--name", "x"])
        .output()
        .expect("run");
    assert_eq!(output.status.code(), Some(2), "no socket");
    assert_eq!(
        ok(&socket, &["help"])
            .lines()
            .next()
            .map(|l| l.starts_with("usage")),
        Some(true)
    );
}

#[test]
fn no_server_exits_3() {
    let output = ctl(&socket_path("none"), &["create-session", "--name", "x"]);
    assert_eq!(output.status.code(), Some(3));
}

#[test]
fn garbage_from_the_server_exits_4() {
    let path = socket_path("junk");
    let listener = StdListener::bind(&path).expect("bind");
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut hello = [0u8; 64];
        let _ = stream.read(&mut hello);
        // A frame of an unknown type.
        stream.write_all(&[0, 0, 0, 1, 0x7f]).expect("write");
        std::thread::sleep(PATIENCE);
    });
    let output = ctl(&path, &["create-session", "--name", "x"]);
    assert_eq!(output.status.code(), Some(4));
}

fn cursor_of(line: &str) -> String {
    line.strip_prefix("cursor ")
        .expect("a cursor line")
        .to_owned()
}

#[test]
fn events_resume_from_the_printed_cursor() {
    let socket = start_server();
    let now = ok(&socket, &["events"]);
    let end = cursor_of(now.lines().last().expect("end cursor"));

    let session = ok(&socket, &["create-session", "--name", "later"]);
    let missed = ok(&socket, &["events", "--after", &end]);
    let lines: Vec<&str> = missed.lines().collect();
    assert_eq!(lines.first().map(|l| cursor_of(l)), Some(end.clone()));
    assert!(
        lines
            .iter()
            .any(|l| l.contains("session-created") && l.contains(&session)),
        "{missed}"
    );

    let (epoch, _) = end.split_once(':').expect("cursor");
    let ahead = format!("{epoch}:999999");
    let output = ctl(&socket, &["events", "--after", &ahead]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "an unreachable cursor is refused"
    );
}

#[test]
fn an_interrupted_follow_ends_with_sigint() {
    let socket = start_server();
    let mut child = ctl_command(&socket, &["events", "--follow"])
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn");
    let mut lines = BufReader::new(child.stdout.take().expect("stdout")).lines();
    let first = lines.next().expect("a line").expect("read");
    assert!(first.starts_with("cursor "), "{first}");

    ok(&socket, &["create-session", "--name", "seen"]);
    let seen = lines
        .by_ref()
        .map(|l| l.expect("read"))
        .find(|l| l.contains("session-created"));
    assert!(seen.is_some(), "follow prints live events");

    let pid = child.id().to_string();
    let killed = Command::new("kill")
        .args(["-INT", &pid])
        .status()
        .expect("kill");
    assert!(killed.success());
    let status = child.wait().expect("wait");
    // A shell reports this as exit code 130.
    assert_eq!(status.signal(), Some(2));
}
