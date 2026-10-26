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
        .env_remove("RUSTY_MUX_SESSION")
        .env_remove("RUSTY_MUX_WINDOW")
        .env_remove("RUSTY_MUX_BLOCK")
        .env_remove("RUSTY_MUX_CONFIG")
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

    // Resizing without the lease, with the refusal on stdout as JSON.
    let output = ctl(
        &socket,
        &[
            "--json", "resize", "--block", &block, "--cols", "80", "--rows", "24",
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).contains("\"ok\":false"));
}

#[test]
fn format_prints_the_screen_and_list_the_tree() {
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
        "echo HI$((6*7))\r",
    ];
    ok(&socket, &chain);
    let capture = ["terminal.format", "--block", &block, "--history", "0"];
    let deadline = std::time::Instant::now() + PATIENCE;
    while !ok(&socket, &capture).lines().any(|l| l == "HI42") {
        assert!(std::time::Instant::now() < deadline, "no HI42 on screen");
        std::thread::sleep(Duration::from_millis(20));
    }

    let listing = ok(&socket, &["--json", "list"]);
    let kinds: Vec<String> = listing
        .lines()
        .map(|line| {
            let rusty_json::Value::Object(map) =
                rusty_json::Value::from_json_str(line).expect("json")
            else {
                panic!("object")
            };
            match map.get("kind") {
                Some(rusty_json::Value::String(kind)) => kind.clone(),
                _ => "done".into(),
            }
        })
        .collect();
    assert_eq!(kinds, ["session", "window", "block", "done"]);
    assert!(listing.contains(&format!("\"block\":\"{block}\"")));
}

#[test]
fn terminal_methods_read_the_block() {
    let socket = start_server();
    let block = block(&socket);
    assert_eq!(ok(&socket, &["terminal.pwd", "--block", &block]), "/");
    assert_eq!(ok(&socket, &["terminal.title", "--block", &block]), "");
    let json = ok(&socket, &["--json", "terminal.pwd", "--block", &block]);
    assert!(json.contains("\"text\":\"/\""), "{json}");
}

#[test]
fn api_lists_and_describes_methods_without_a_server() {
    let nowhere = Path::new("/nonexistent/rusty-mux.sock");
    let listing = ok(nowhere, &["api", "list"]);
    assert!(listing.lines().any(|l| l.starts_with("terminal.format ")));
    let described = ok(nowhere, &["api", "describe", "resize"]);
    let rusty_json::Value::Object(map) =
        rusty_json::Value::from_json_str(&described).expect("json")
    else {
        panic!("{described}");
    };
    assert_eq!(map.get("name"), Some(&"resize".into()));
    for args in [&["api", "describe", "nope"][..], &["api"], &["api", "call"]] {
        assert_eq!(ctl(nowhere, args).status.code(), Some(2), "{args:?}");
    }
}

#[test]
fn targets_resolve_by_name_position_and_suffix() {
    let socket = start_server();
    ok(&socket, &["create-session", "--name", "Build"]);
    ok(
        &socket,
        &["create-window", "--session", "build", "--name", "logs"],
    );
    let block = ok(
        &socket,
        &[
            "create-block",
            "--window",
            "build/logs",
            "--cols",
            "80",
            "--rows",
            "24",
        ],
    );
    ok(
        &socket,
        &[
            "terminal.format",
            "--block",
            "build/logs/1",
            "--history",
            "0",
        ],
    );
    ok(
        &socket,
        &["terminal.format", "--block", "1/1/1", "--history", "0"],
    );
    let suffix = &block[block.len() - 6..];
    ok(&socket, &["close-block", "--block", suffix]);
    assert!(!ok(&socket, &["list"]).contains(&block));
}

#[test]
fn a_missing_or_ambiguous_target_exits_3() {
    let socket = start_server();
    let missing = "00000000-0000-0000-0000-000000000001";
    let output = ctl(&socket, &["close-block", "--block", missing]);
    assert_eq!(output.status.code(), Some(3), "a full id the server lacks");
    let output = ctl(&socket, &["close-block", "--block", "nope"]);
    assert_eq!(output.status.code(), Some(3), "a name nothing has");

    ok(&socket, &["create-session", "--name", "dup"]);
    ok(&socket, &["create-session", "--name", "dup"]);
    let output = ctl(
        &socket,
        &["create-window", "--session", "dup", "--name", "w"],
    );
    assert_eq!(output.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("session `dup` is ambiguous"), "{stderr}");
}

#[test]
fn a_missing_id_defaults_to_where_ctl_runs() {
    let socket = start_server();
    let block = block(&socket);
    let output = ctl_command(&socket, &["close-block"])
        .env("RUSTY_MUX_BLOCK", &block)
        .output()
        .expect("run");
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!ok(&socket, &["list"]).contains(&block));
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
fn no_server_exits_4() {
    let output = ctl(&socket_path("none"), &["create-session", "--name", "x"]);
    assert_eq!(output.status.code(), Some(4));
}

#[test]
fn garbage_from_the_server_exits_1() {
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
    assert_eq!(output.status.code(), Some(1));
}

/// A server from before SYNC: it welcomes the client and then counts the
/// request frames it receives until the client hangs up.
fn legacy_server() -> (PathBuf, std::sync::mpsc::Receiver<usize>) {
    use rusty_mux_core::ClientId;
    use rusty_mux_proto::{Features, Message, decode, encode};

    let path = socket_path("legacy");
    let listener = StdListener::bind(&path).expect("bind");
    let (count_tx, count_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        stream.set_read_timeout(Some(PATIENCE)).expect("timeout");
        let mut buf = Vec::new();
        let mut frames = 0usize;
        let mut chunk = [0u8; 4096];
        loop {
            while let Ok(Some((message, used))) = decode(&buf) {
                buf.drain(..used);
                if let Message::Hello { .. } = message {
                    let welcome = Message::Welcome {
                        features: Features::CHECKPOINT_V3,
                        client: ClientId::from_uuid(rusty_uuid::Uuid::nil()),
                        cursor: None,
                    };
                    stream
                        .write_all(&encode(&welcome).expect("encode"))
                        .expect("write");
                } else {
                    frames += 1;
                }
            }
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        }
        count_tx.send(frames).expect("report");
    });
    (path, count_rx)
}

#[test]
fn a_chain_the_server_cannot_run_sends_nothing() {
    let block = "00000000-0000-0000-0000-000000000001";
    let chains: [&[&str]; 4] = [
        &[
            "attach", "--block", block, "+", "input", "--block", block, "--data", "x",
        ],
        &["sync"],
        &["create-session", "--name", "s", "+", "list"],
        &["terminal.format", "--block", block, "--history", "0"],
    ];
    for chain in chains {
        let (path, frames) = legacy_server();
        assert_eq!(ctl(&path, chain).status.code(), Some(1), "{chain:?}");
        let sent = frames.recv_timeout(PATIENCE).expect("frame count");
        assert_eq!(sent, 0, "{chain:?} sent requests before refusing");
    }
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

/// Writes `text` as a config file for one test; returns its path.
fn config_file(tag: &str, text: &str) -> PathBuf {
    let path =
        std::env::temp_dir().join(format!("rusty-mux-ctl-{}-{tag}.json", std::process::id()));
    std::fs::write(&path, text).expect("write config");
    path
}

fn with_config(socket: &Path, config: &Path, args: &[&str]) -> Output {
    ctl_command(socket, args)
        .env("RUSTY_MUX_CONFIG", config)
        .output()
        .expect("run rusty-mux-ctl")
}

const WORKSPACE: &str = r#"{
  "actions": [{
    "name": "workspace",
    "title": "A session with one block",
    "args": ["name"],
    "run": [
      {"call": "create-session", "args": {"name": "${name}"}},
      {"call": "create-window", "args": {"session": "${name}", "name": "main"}},
      {"call": "create-block", "args": {"window": "${name}/main", "cols": 80, "rows": 24}}
    ]
  }],
  "binds": [{"keys": "ctrl+b>w", "action": "workspace", "args": {"name": "work"}}]
}"#;

#[test]
fn do_runs_a_configured_action_as_one_chain() {
    let socket = start_server();
    let config = config_file("do", WORKSPACE);
    let output = with_config(&socket, &config, &["do", "workspace", "--name", "agents"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(stdout(&output).lines().count(), 3, "three new ids");
    let listing = ok(&socket, &["list"]);
    assert!(
        listing.contains("agents") && listing.contains("main"),
        "{listing}"
    );

    let usage = |args: &[&str]| with_config(&socket, &config, args).status.code();
    assert_eq!(usage(&["do", "workspace"]), Some(2), "missing argument");
    assert_eq!(
        usage(&["do", "workspace", "--name", "a", "--x", "1"]),
        Some(2)
    );
    assert_eq!(usage(&["do", "nope"]), Some(2), "unknown action");
    assert_eq!(usage(&["do"]), Some(2));
    let refused = with_config(&socket, &config, &["do", "workspace", "--name", "agents"]);
    assert_eq!(
        refused.status.code(),
        Some(3),
        "the session name is taken, so its window target is ambiguous"
    );
    std::fs::remove_file(&config).expect("cleanup");
}

#[test]
fn config_check_and_list_need_no_server() {
    let nowhere = Path::new("/nonexistent/rusty-mux.sock");
    let good = config_file("good", WORKSPACE);
    let checked = with_config(nowhere, &good, &["config", "check"]);
    assert_eq!(stdout(&checked).trim(), "ok: 1 actions, 1 bindings");
    let listed = stdout(&with_config(nowhere, &good, &["config", "list"]));
    assert!(listed.starts_with("action workspace"), "{listed}");
    assert!(listed.contains("bind   ctrl+b>w"), "{listed}");

    let bad = config_file(
        "bad",
        r#"{"actions": [{"name": "x", "run": [{"call": "nope"}]}]}"#,
    );
    let path = bad.to_str().expect("utf-8");
    let refused = ctl(nowhere, &["config", "check", path]);
    assert_eq!(refused.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&refused.stderr).into_owned();
    assert!(
        stderr.contains("actions[0].run[0]: unknown method `nope`"),
        "{stderr}"
    );
    assert_eq!(
        with_config(nowhere, &bad, &["do", "x"]).status.code(),
        Some(1)
    );
    assert_eq!(ctl(nowhere, &["config", "frob"]).status.code(), Some(2));
    for path in [good, bad] {
        std::fs::remove_file(path).expect("cleanup");
    }
}
