//! `rusty-mux-ctl` against a real server: output, chains, events and every
//! exit code (0, 1, 2, 3, 4, and 130 from an interrupt), on Linux and
//! Windows.

#![cfg(any(target_os = "linux", windows))]

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

#[cfg(windows)]
use platform_windows::WindowsUnixListener as StdListener;
use rusty_mux_server::{Config, ConsoleHost, serve};
use rusty_tokio::io::UnixListener;
use rusty_tokio::{Builder, LocalOptions};
#[cfg(unix)]
use std::os::unix::net::UnixListener as StdListener;

const PATIENCE: Duration = Duration::from_secs(20);

/// The shell each test block runs: `/bin/sh` in `/`, or `cmd.exe /Q` (no
/// command echo) in the temp directory. The server's own tests use the
/// same; one crate's integration tests cannot share another's helpers.
fn shell() -> Config {
    if cfg!(windows) {
        Config {
            program: "cmd.exe".into(),
            args: vec!["/Q".into()],
            cwd: std::env::temp_dir().into_os_string(),
            console_host: ConsoleHost::from_env().expect("RUSTY_CONSOLE_HOST"),
        }
    } else {
        Config {
            program: "/bin/sh".into(),
            args: Vec::new(),
            cwd: "/".into(),
            console_host: ConsoleHost::default(),
        }
    }
}

/// A command that does nothing and prints nothing.
const NOTHING: &str = if cfg!(windows) { "rem" } else { "true" };

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
            serve(listener, shell()).await.expect("serve");
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
        &format!("{NOTHING}\r"),
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
        // Output differs from the echo: `cmd.exe` drops the `^` escape.
        if cfg!(windows) {
            "echo HI^42\r"
        } else {
            "echo HI$((6*7))\r"
        },
    ];
    ok(&socket, &chain);
    let capture = ["terminal.format", "--block", &block, "--history", "0"];
    let deadline = std::time::Instant::now() + PATIENCE;
    // A prompt printed after the echo can share the output's line.
    let shown = |screen: String, text: &str| screen.lines().any(|l| l.ends_with(text));
    while !shown(ok(&socket, &capture), "HI42") {
        assert!(std::time::Instant::now() < deadline, "no HI42 on screen");
        std::thread::sleep(Duration::from_millis(20));
    }

    // Rows below the prompt are blank; `--trim` leaves them out.
    let rows = |args: &[&str]| stdout(&ctl(&socket, args)).lines().count();
    assert_eq!(rows(&capture), 24);
    let trimmed = [&capture[..], &["--trim"]].concat();
    assert!(rows(&trimmed) < 24);
    assert!(shown(ok(&socket, &trimmed), "HI42"));

    // The same command, typed as keys; only its output says KEY42.
    let typed = if cfg!(windows) {
        "echo KEY^42"
    } else {
        "echo KEY$((6*7))"
    };
    let mut keys = vec!["terminal.send-keys", "--block", &block];
    for key in typed.split("").filter(|k| !k.is_empty()) {
        keys.extend(["--key", key]);
    }
    keys.extend(["--key", "enter"]);
    let pressed = [
        &[
            "attach",
            "--block",
            &block,
            "+",
            "acquire-lease",
            "--block",
            &block,
            "+",
        ][..],
        &keys,
    ]
    .concat();
    ok(&socket, &pressed);
    let deadline = std::time::Instant::now() + PATIENCE;
    while !shown(ok(&socket, &capture), "KEY42") {
        assert!(std::time::Instant::now() < deadline, "no KEY42 on screen");
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
    // ctl's own connection is a client too.
    assert_eq!(kinds, ["session", "window", "block", "client", "done"]);
    assert!(listing.contains(&format!("\"block\":\"{block}\"")));
}

/// Unix only: on Windows `pwd` is known only from OSC 7, which `cmd.exe`
/// does not send, and `cmd.exe` sets a title.
#[cfg(unix)]
#[test]
fn terminal_methods_read_the_block() {
    let socket = start_server();
    let block = block(&socket);
    assert_eq!(ok(&socket, &["terminal.pwd", "--block", &block]), "/");
    assert_eq!(ok(&socket, &["terminal.title", "--block", &block]), "");
    let json = ok(&socket, &["--json", "terminal.pwd", "--block", &block]);
    assert!(json.contains("\"text\":\"/\""), "{json}");
}

/// An agent's view of a job: wait until it has the terminal, then read
/// what runs. Linux only: Windows reports no foreground process.
#[cfg(target_os = "linux")]
#[test]
fn wait_for_a_job_then_read_the_process() {
    let socket = start_server();
    let block = block(&socket);
    let waiter = ctl_command(
        &socket,
        &[
            "wait",
            "--event",
            "process-changed",
            "--block",
            &block,
            "--name",
            "sleep",
            "--timeout",
            "20",
        ],
    )
    .stdout(Stdio::piped())
    .spawn()
    .expect("spawn");
    ok(
        &socket,
        &[
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
            "sleep 30\r",
        ],
    );
    let output = waiter.wait_with_output().expect("wait");
    assert_eq!(output.status.code(), Some(0));
    assert!(
        stdout(&output).contains(" name=\"sleep\""),
        "{}",
        stdout(&output)
    );
    let process = ok(&socket, &["terminal.process", "--block", &block]);
    let names: Vec<&str> = process
        .lines()
        .filter_map(|l| l.split('\t').nth(1))
        .collect();
    assert_eq!(names, ["sleep", "sh"], "{process}");
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

/// The ids of the clients `list` shows.
fn client_ids(socket: &Path) -> Vec<String> {
    ok(socket, &["list"])
        .lines()
        .filter_map(|l| l.strip_prefix("client "))
        .filter_map(|l| l.split(' ').next())
        .map(str::to_owned)
        .collect()
}

#[test]
fn rename_then_disconnect_a_client_by_its_name() {
    let socket = start_server();
    let block = block(&socket);
    ok(
        &socket,
        &["rename-block", "--block", &block, "--name", "Build"],
    );
    ok(
        &socket,
        &["terminal.format", "--block", "build", "--history", "0"],
    );

    // A follower is a client that stays: the one in both listings.
    let mut follower = ctl_command(&socket, &["events", "--follow"])
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn");
    let mut lines = BufReader::new(follower.stdout.take().expect("stdout")).lines();
    let first = lines.next().expect("a line").expect("read");
    assert!(first.starts_with("cursor "), "{first}");
    let (one, two) = (client_ids(&socket), client_ids(&socket));
    let stays: Vec<&String> = one.iter().filter(|id| two.contains(id)).collect();
    let [id] = stays.as_slice() else {
        panic!("one lasting client: {one:?} {two:?}");
    };
    ok(
        &socket,
        &[
            "rename-client",
            "--client",
            &id[id.len() - 6..],
            "--name",
            "Follower",
        ],
    );
    let renamed = lines
        .by_ref()
        .map(|l| l.expect("read"))
        .find(|l| l.contains("client-renamed"));
    assert!(
        renamed.is_some_and(|l| l.contains(id.as_str()) && l.contains("Follower")),
        "the follower hears of its new name"
    );

    let own = ctl(&socket, &["disconnect-client", "--client", "nobody"]);
    assert_eq!(
        own.status.code(),
        Some(3),
        "a missing client is a missing target"
    );
    ok(&socket, &["disconnect-client", "--client", "follower"]);
    let deadline = std::time::Instant::now() + PATIENCE;
    while follower.try_wait().expect("wait").is_none() {
        assert!(
            std::time::Instant::now() < deadline,
            "follower still connected"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!client_ids(&socket).contains(id));
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

#[test]
fn split_beside_here_then_zoom_and_read_the_layout() {
    let socket = start_server();
    let block = block(&socket);
    let output = ctl_command(&socket, &["split-block", "--side", "down"])
        .env("RUSTY_MUX_BLOCK", &block)
        .output()
        .expect("run");
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let new = stdout(&output).trim().to_owned();
    ok(&socket, &["zoom-block", "--block", &new]);
    let listed = ok(&socket, &["list"]);
    let window = listed
        .lines()
        .find(|l| l.starts_with("window "))
        .expect("a window line");
    assert!(
        window.contains(&format!("layout=\"rows50({block},{new})\" zoomed={new}")),
        "{window}"
    );
    let json = ok(&socket, &["--json", "list"]);
    assert!(json.contains(&format!("\"zoomed\":\"{new}\"")), "{json}");
    let refused = ctl(
        &socket,
        &["split-block", "--block", &block, "--side", "north"],
    );
    assert_eq!(refused.status.code(), Some(2));
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
        NOTHING,
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
                        features: Features::CHECKPOINT_V6,
                        client: ClientId::from_uuid(rusty_uuid::Uuid::nil()),
                        cursor: None,
                        token: None,
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

/// A server without `LAUNCH` would ignore a launch argument and start its
/// shell, so ctl refuses before sending anything, typed, scripted or run.
#[test]
fn launch_arguments_are_refused_by_a_server_that_would_ignore_them() {
    let window = "00000000-0000-0000-0000-000000000001";
    let create = [
        "create-block",
        "--window",
        window,
        "--cols",
        "80",
        "--rows",
        "24",
    ];
    let launches: [&[&str]; 3] = [
        &["--command", "true"],
        &["--command", "sh", "--arg", "-c", "--arg", "exit 3"],
        &["--cwd", "/tmp"],
    ];
    for launch in launches {
        let (path, frames) = legacy_server();
        let args: Vec<&str> = create.iter().chain(launch).copied().collect();
        let output = ctl(&path, &args);
        assert_eq!(output.status.code(), Some(1), "{launch:?}");
        let reason = String::from_utf8_lossy(&output.stderr);
        assert!(
            reason.contains("does not support create-block with --"),
            "{reason}"
        );
        assert_eq!(
            frames.recv_timeout(PATIENCE).expect("count"),
            0,
            "{launch:?}"
        );
    }

    let (path, frames) = legacy_server();
    let output = ctl(&path, &["run", "--window", window, "--", "true"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(frames.recv_timeout(PATIENCE).expect("count"), 0, "run");

    let config = config_file(
        "launch",
        &format!(
            r#"{{"actions": [{{"name": "go", "run": [{{"call": "create-block", "args": {{"window": "{window}", "cols": 80, "rows": 24, "command": "true"}}}}]}}]}}"#
        ),
    );
    let (path, frames) = legacy_server();
    let output = with_config(&path, &config, &["do", "go"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(frames.recv_timeout(PATIENCE).expect("count"), 0, "do");
    std::fs::remove_file(&config).expect("cleanup");
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

/// Unix only: it sends `SIGINT` and checks the signal in the exit status.
#[cfg(unix)]
#[test]
fn an_interrupted_follow_ends_with_sigint() {
    use std::os::unix::process::ExitStatusExt;

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
    assert_eq!(
        stdout(&checked).trim(),
        "ok: 1 actions, 1 bindings, 0 rules"
    );
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

const RULES: &str = r#"{
  "on": [
    {"event": "session-created", "action": "terminal.title", "args": {"block": "nothing"}},
    {"event": "pwd-changed", "action": "create-session", "args": {"name": "at ${pwd}"}}
  ]
}"#;

/// Unix only: it reports the directory with `printf` and OSC 7, which
/// `cmd.exe` cannot print and ConPTY may not pass through.
#[cfg(unix)]
#[test]
fn watch_runs_rules_for_events_and_survives_a_failing_one() {
    let socket = start_server();
    let config = config_file("watch", RULES);
    let mut child = ctl_command(&socket, &["watch"])
        .env("RUSTY_MUX_CONFIG", &config)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let mut lines = BufReader::new(child.stdout.take().expect("stdout")).lines();
    let first = lines.next().expect("a line").expect("read");
    assert!(first.starts_with("cursor "), "{first}");

    // Creating the block's session fails rule 0; the watch goes on.
    let block = block(&socket);
    let report = "printf '\\033]7;file://host/tmp\\007'\r";
    ok(
        &socket,
        &[
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
            report,
        ],
    );
    let deadline = std::time::Instant::now() + PATIENCE;
    while !ok(&socket, &["list"]).contains("at /tmp") {
        assert!(std::time::Instant::now() < deadline, "rule 1 never ran");
        std::thread::sleep(Duration::from_millis(50));
    }

    child.kill().expect("kill");
    child.wait().expect("wait");
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("stderr")
        .read_to_string(&mut stderr)
        .expect("read");
    assert!(stderr.contains("on[0] failed for event"), "{stderr}");
    let printed: Vec<String> = lines.map(|l| l.expect("read")).collect();
    assert!(
        printed.iter().any(|l| l.contains("pwd-changed")),
        "{printed:?}"
    );
    std::fs::remove_file(&config).expect("cleanup");
}

#[test]
fn watch_hands_a_rule_the_exit_code() {
    let socket = start_server();
    let config = config_file(
        "exit",
        r#"{"on": [{"event": "program-exited", "action": "create-session", "args": {"name": "exit ${code}"}}]}"#,
    );
    let mut child = ctl_command(&socket, &["watch"])
        .env("RUSTY_MUX_CONFIG", &config)
        .stdout(Stdio::null())
        .spawn()
        .expect("spawn");
    let block = block(&socket);
    ok(
        &socket,
        &[
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
            "exit 4\r",
        ],
    );
    let deadline = std::time::Instant::now() + PATIENCE;
    while !ok(&socket, &["list"]).contains("exit 4") {
        assert!(std::time::Instant::now() < deadline, "the rule never ran");
        std::thread::sleep(Duration::from_millis(50));
    }
    child.kill().expect("kill");
    child.wait().expect("wait");
    std::fs::remove_file(&config).expect("cleanup");
}

#[test]
fn watch_needs_rules_and_takes_only_a_cursor() {
    let socket = start_server();
    let none = config_file("norules", WORKSPACE);
    let refused = with_config(&socket, &none, &["watch"]);
    assert_eq!(refused.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&refused.stderr).contains("no `on` rules"));
    assert_eq!(
        with_config(&socket, &none, &["watch", "--follow"])
            .status
            .code(),
        Some(2)
    );
    std::fs::remove_file(&none).expect("cleanup");
}

/// A server that numbers events but offers only `features`: it welcomes
/// the client with a cursor, then stays silent until it hangs up.
fn events_server(features: rusty_mux_proto::Features) -> PathBuf {
    use rusty_mux_core::ClientId;
    use rusty_mux_proto::{Cursor, Message, decode, encode};

    let path = socket_path("old-events");
    let listener = StdListener::bind(&path).expect("bind");
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        while !matches!(decode(&buf), Ok(Some((Message::Hello { .. }, _)))) {
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => return,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        }
        let welcome = Message::Welcome {
            features,
            client: ClientId::from_uuid(rusty_uuid::Uuid::nil()),
            cursor: Some(Cursor {
                epoch: rusty_uuid::Uuid::nil(),
                seq: 0,
            }),
            token: None,
        };
        stream
            .write_all(&encode(&welcome).expect("encode"))
            .expect("write");
        std::thread::sleep(PATIENCE);
    });
    path
}

#[test]
fn watch_refuses_rules_for_events_the_server_never_sends() {
    use rusty_mux_proto::Features;

    let old = Features::EVENT_SEQ
        .union(Features::SYNC)
        .union(Features::STATUS);
    let pwd = config_file("needs-pwd", RULES);
    let refused = with_config(&events_server(old), &pwd, &["watch"]);
    assert_eq!(refused.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&refused.stderr).into_owned();
    assert!(
        stderr.contains("does not send `pwd-changed` events"),
        "{stderr}"
    );

    let bell = config_file(
        "needs-bell",
        r#"{"on": [{"event": "bell", "action": "list"}]}"#,
    );
    let pwd_only = old.union(Features::TERMINAL_EVENTS);
    let refused = with_config(&events_server(pwd_only), &bell, &["watch"]);
    assert_eq!(refused.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&refused.stderr).into_owned();
    assert!(stderr.contains("does not send `bell` events"), "{stderr}");
    std::fs::remove_file(&bell).expect("cleanup");

    // Rules for events every numbering server sends still run.
    let generic = config_file(
        "generic",
        r#"{"on": [{"event": "session-created", "action": "list"}]}"#,
    );
    let mut child = ctl_command(&events_server(old), &["watch"])
        .env("RUSTY_MUX_CONFIG", &generic)
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn");
    let mut lines = BufReader::new(child.stdout.take().expect("stdout")).lines();
    let first = lines.next().expect("a line").expect("read");
    assert!(first.starts_with("cursor "), "{first}");
    child.kill().expect("kill");
    child.wait().expect("wait");
    for path in [pwd, generic] {
        std::fs::remove_file(path).expect("cleanup");
    }
}

/// Creates a session and a window; returns the window id.
fn window(socket: &Path) -> String {
    let session = ok(socket, &["create-session", "--name", "s"]);
    ok(
        socket,
        &["create-window", "--session", &session, "--name", "w"],
    )
}

/// A program, as `run` takes it, that exits with `code`.
fn exits_with(code: u8) -> Vec<String> {
    let line = format!("exit {code}");
    match cfg!(windows) {
        true => vec!["cmd.exe".into(), "/c".into(), line],
        false => vec!["sh".into(), "-c".into(), line],
    }
}

/// What to type at the block's shell so it waits a second, long enough
/// for a waiter to be listening, then exits with `code`.
fn pause_then_exit(code: u8) -> String {
    match cfg!(windows) {
        true => format!("ping -n 2 127.0.0.1 >nul & exit {code}\r"),
        false => format!("sleep 1; exit {code}\r"),
    }
}

#[test]
fn run_wait_exits_with_the_programs_status() {
    let socket = start_server();
    let window = window(&socket);
    let program = exits_with(3);
    let mut args = vec!["run", "--window", &window, "--wait", "--"];
    args.extend(program.iter().map(String::as_str));
    let output = ctl(&socket, &args);
    assert_eq!(
        output.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = stdout(&output);
    let mut lines = text.lines();
    let block = lines.next().expect("the new block's id");
    assert!(block.parse::<rusty_uuid::Uuid>().is_ok(), "{text}");
    let exited = lines.next().expect("the exit");
    assert!(
        exited.contains(&format!("program-exited block={block} code=3")),
        "{text}"
    );

    // Without --wait it prints the block and returns at once.
    let program = exits_with(0);
    let mut args = vec!["run", "--window", &window, "--"];
    args.extend(program.iter().map(String::as_str));
    assert!(ok(&socket, &args).parse::<rusty_uuid::Uuid>().is_ok());
}

#[cfg(target_os = "linux")]
#[test]
fn run_starts_in_ctls_directory_and_reports_a_signal() {
    let socket = start_server();
    let window = window(&socket);
    let check = r#"[ "$(pwd)" = /tmp ]"#;
    let output = ctl_command(
        &socket,
        &["run", "--window", &window, "--wait", "sh", "-c", check],
    )
    .current_dir("/tmp")
    .output()
    .expect("run");
    assert_eq!(output.status.code(), Some(0), "the server's dir is /");
    let killed = ctl(
        &socket,
        &[
            "run",
            "--window",
            &window,
            "--wait",
            "--",
            "sh",
            "-c",
            "kill -9 $$",
        ],
    );
    assert_eq!(killed.status.code(), Some(128 + 9));
    assert!(stdout(&killed).contains("signal=9"));
    let missing = ctl(
        &socket,
        &[
            "run",
            "--window",
            &window,
            "--wait",
            "--",
            "/no/such/program",
        ],
    );
    assert_eq!(missing.status.code(), Some(1), "refused, not run");
}

#[test]
fn wait_follows_an_existing_block_until_its_program_exits() {
    let socket = start_server();
    let block = block(&socket);
    let waiter = ctl_command(&socket, &["wait", "--block", &block, "--timeout", "20"])
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn");
    // Long enough for the waiter to be listening first.
    ok(
        &socket,
        &[
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
            &pause_then_exit(5),
        ],
    );
    let output = waiter.wait_with_output().expect("wait");
    assert_eq!(output.status.code(), Some(5));
    assert!(stdout(&output).contains("code=5"), "{}", stdout(&output));

    // The block is gone now.
    assert_eq!(
        ctl(&socket, &["wait", "--block", &block]).status.code(),
        Some(3)
    );
}

#[test]
fn wait_gives_up_at_its_timeout_and_fails_on_a_closed_block() {
    let socket = start_server();
    let block = block(&socket);
    let started = std::time::Instant::now();
    let output = ctl(&socket, &["wait", "--block", &block, "--timeout", "0.3"]);
    assert_eq!(output.status.code(), Some(124));
    assert!(started.elapsed() < PATIENCE);

    let waiter = ctl_command(&socket, &["wait", "--block", &block, "--timeout", "20"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    std::thread::sleep(Duration::from_millis(500));
    ok(&socket, &["close-block", "--block", &block]);
    let output = waiter.wait_with_output().expect("wait");
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).contains("block-closed"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("before its program exited"));
}

#[test]
fn wait_for_an_event_whose_fields_match() {
    let socket = start_server();
    let block = block(&socket);
    let waiter = ctl_command(
        &socket,
        &[
            "--json",
            "wait",
            "--event",
            "program-exited",
            "--block",
            &block,
            "--code",
            "7",
            "--timeout",
            "20",
        ],
    )
    .stdout(Stdio::piped())
    .spawn()
    .expect("spawn");
    ok(
        &socket,
        &[
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
            &pause_then_exit(7),
        ],
    );
    let output = waiter.wait_with_output().expect("wait");
    assert_eq!(output.status.code(), Some(0));
    let line = stdout(&output);
    let event = rusty_json::Value::from_json_str(line.trim()).expect("json");
    let rusty_json::Value::Object(map) = event else {
        panic!("{line}")
    };
    assert_eq!(
        map.get("kind"),
        Some(&rusty_json::Value::from("program-exited"))
    );

    let output = ctl(&socket, &["wait", "--event", "bell", "--timeout", "0.3"]);
    assert_eq!(output.status.code(), Some(124));
    for args in [&["wait"][..], &["wait", "--event", "nope"], &["run"]] {
        assert_eq!(ctl(&socket, args).status.code(), Some(2), "{args:?}");
    }
}

/// Types at `block`'s shell, once leased, what makes it exit with `code`
/// after a pause.
fn exit_later(socket: &Path, block: &str, code: u8) {
    ok(
        socket,
        &[
            "attach",
            "--block",
            block,
            "+",
            "acquire-lease",
            "--block",
            block,
            "+",
            "input",
            "--block",
            block,
            "--data",
            &pause_then_exit(code),
        ],
    );
}

/// Returns once a second client, the waiter, is connected besides the
/// one `list` itself uses. Its stream is open by then; whatever ends
/// before its listing still counts.
fn connected(socket: &Path) {
    let started = std::time::Instant::now();
    while client_ids(socket).len() < 2 {
        assert!(started.elapsed() < PATIENCE, "the waiter never connected");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn shell_block(socket: &Path, window: &str) -> String {
    ok(
        socket,
        &[
            "create-block",
            "--window",
            window,
            "--cols",
            "80",
            "--rows",
            "24",
        ],
    )
}

#[test]
fn wait_on_a_window_follows_every_block_until_the_last_ends() {
    let socket = start_server();
    let window = window(&socket);
    let (first, second) = (shell_block(&socket, &window), shell_block(&socket, &window));
    let waiter = ctl_command(&socket, &["wait", "--window", &window, "--timeout", "20"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    connected(&socket);
    // A block made after the wait began joins it.
    let late = shell_block(&socket, &window);
    exit_later(&socket, &first, 0);
    exit_later(&socket, &second, 4);
    exit_later(&socket, &late, 0);
    let output = waiter.wait_with_output().expect("wait");
    let text = stdout(&output);
    assert_eq!(
        output.status.code(),
        Some(4),
        "{text}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for (block, code) in [(&first, 0), (&second, 4), (&late, 0)] {
        let line = format!("program-exited block={block} code={code}");
        assert!(text.contains(&line), "{line} in {text}");
    }

    // Nothing is left in it, so a new wait ends at once, quietly.
    let output = ctl(&socket, &["wait", "--window", &window, "--timeout", "5"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "");
}

#[test]
fn wait_on_a_session_fails_when_a_block_closes_unfinished() {
    let socket = start_server();
    let session = ok(&socket, &["create-session", "--name", "s"]);
    let window = ok(
        &socket,
        &["create-window", "--session", &session, "--name", "w"],
    );
    let closed = shell_block(&socket, &window);
    let waiter = ctl_command(&socket, &["wait", "--session", "s", "--timeout", "20"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    connected(&socket);
    // A window and block made during the wait join it.
    let other = ok(
        &socket,
        &["create-window", "--session", &session, "--name", "x"],
    );
    let finished = shell_block(&socket, &other);
    exit_later(&socket, &finished, 0);
    ok(&socket, &["close-block", "--block", &closed]);
    let output = waiter.wait_with_output().expect("wait");
    let text = stdout(&output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(
        text.contains(&format!("block-closed block={closed}")),
        "{text}"
    );
    assert!(
        text.contains(&format!("program-exited block={finished} code=0")),
        "{text}"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("before its program exited"));

    assert_eq!(
        ctl(&socket, &["wait", "--session", "nope"]).status.code(),
        Some(3)
    );
}

/// What a client running `ssh host rusty-mux-ctl bridge` sees: its frames
/// reach the server through stdin and the answers come back on stdout. The
/// end of stdin ends the bridge.
#[test]
fn bridge_carries_the_protocol_over_stdin_and_stdout() {
    use rusty_mux_proto::{Features, Message, decode, encode};

    let socket = start_server();
    let mut bridge = ctl_command(&socket, &["bridge"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("run rusty-mux-ctl bridge");
    let mut input = bridge.stdin.take().expect("stdin");
    let hello = Message::Hello {
        features: Features::SYNC,
        resume: None,
        takeover: None,
    };
    let sync = Message::Sync { req: 1 };
    for message in [&hello, &sync] {
        input.write_all(&encode(message).expect("encode")).unwrap();
    }
    input.flush().unwrap();

    let mut output = bridge.stdout.take().expect("stdout");
    let (mut buf, mut got) = (Vec::new(), Vec::new());
    while !matches!(got.last(), Some(Message::Done { req: 1, .. })) {
        if let Some((message, used)) = decode(&buf).expect("decode") {
            buf.drain(..used);
            got.push(message);
            continue;
        }
        let mut chunk = [0u8; 4096];
        let n = output.read(&mut chunk).expect("read");
        assert!(n > 0, "the bridge ended early");
        buf.extend_from_slice(&chunk[..n]);
    }
    assert!(matches!(got[0], Message::Welcome { .. }), "{got:?}");

    drop(input);
    let status = bridge.wait().expect("wait");
    assert!(status.success(), "{status}");
}

#[test]
fn bridge_to_no_server_is_unreachable() {
    let output = ctl(&socket_path("bridge-none"), &["bridge"]);
    assert_eq!(output.status.code(), Some(4), "{output:?}");
}
