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
            state_dir: None,
            grapheme_clustering: false,
            config_file: None,
        }
    } else {
        Config {
            program: "/bin/sh".into(),
            args: Vec::new(),
            cwd: "/".into(),
            console_host: ConsoleHost::default(),
            state_dir: None,
            grapheme_clustering: false,
            config_file: None,
        }
    }
}

/// Input that makes the block's shell print `OSC 7` naming `/tmp`. On
/// Windows `cmd.exe` has no `printf`: it takes ESC from a `prompt $E` a
/// child shell prints, then echoes the sequence ended by ST.
const REPORT_TMP: &str = if cfg!(windows) {
    "for /F %a in ('echo prompt $E ^| cmd') do @set \"ESC=%a\"\r\
     echo %ESC%]7;file://host/tmp%ESC%\\\r"
} else {
    "printf '\\033]7;file://host/tmp\\007'\r"
};

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
    start_server_with(shell())
}

fn start_server_with(config: Config) -> PathBuf {
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
    // A current server grants what ctl asks for: no older-server notice.
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("It lacks"), "{args:?}: {stderr}");
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
fn a_session_created_without_a_name_gets_a_generated_one() {
    let socket = start_server();
    let session = ok(&socket, &["create-session"]);
    let name = session_name(&socket, &session);
    assert!(rusty_mux_core::is_generated(&name), "{name}");
    let help = ok(&socket, &["help", "create-session"]);
    assert!(help.contains("[--name <text>]"), "{help}");
}

#[test]
fn the_naming_hook_asks_only_while_the_name_is_generated() {
    let socket = start_server();
    let session = ok(&socket, &["create-session"]);
    let block = "00000000-0000-0000-0000-00000000000b";
    let hook = |session: &str, flag: &[&str]| {
        let mut args = vec!["agent", "hook"];
        args.extend(flag);
        let output = ctl_command(&socket, &args)
            .env("RUSTY_MUX_SESSION", session)
            .env("RUSTY_MUX_BLOCK", block)
            .output()
            .expect("run rusty-mux-ctl");
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        stdout(&output)
    };
    let asked = hook(&session, &["--name-session"]);
    assert!(
        asked.contains(&format!("name-session --session {session} --name NAME")),
        "{asked}"
    );
    assert!(
        !hook(&session, &[]).contains("name-session"),
        "not opted in"
    );

    rename(&socket, &session, "fix-login");
    let named = hook(&session, &["--name-session"]);
    assert!(
        named.contains(block) && !named.contains("name-session"),
        "{named}"
    );

    // A name chosen in the generated shape is still chosen.
    let chosen = ok(&socket, &["create-session", "--name", "quiet-harbor"]);
    let text = hook(&chosen, &["--name-session"]);
    assert!(
        text.contains(block) && !text.contains("name-session"),
        "{text}"
    );
}

#[test]
fn an_event_hook_reports_status_silently_and_never_fails() {
    let socket = start_server();
    let block = block(&socket);
    let hook = |socket: &Path, block: &str, args: &[&str]| {
        let mut all = vec!["agent", "hook", "--event"];
        all.extend(args);
        let output = ctl_command(socket, &all)
            .env("RUSTY_MUX_BLOCK", block)
            .output()
            .expect("run rusty-mux-ctl");
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(
            output.stdout.is_empty() && output.stderr.is_empty(),
            "{output:?}"
        );
    };
    hook(
        &socket,
        &block,
        &[
            "blocked",
            "--kind",
            "permission",
            "--app",
            "pi",
            "--msg",
            "hi",
        ],
    );
    assert_eq!(
        ok(&socket, &["terminal.status", "--block", &block]),
        "state=blocked:kind=permission:app=pi:msg=aGk="
    );
    // A bad report, an unknown block and a missing server change nothing.
    hook(&socket, &block, &["busy"]);
    hook(&socket, "00000000-0000-0000-0000-00000000000b", &["idle"]);
    hook(Path::new("/nonexistent/rusty-mux.sock"), &block, &["idle"]);
    assert_eq!(
        ok(&socket, &["terminal.status", "--block", &block]),
        "state=blocked:kind=permission:app=pi:msg=aGk="
    );
}

/// O2 part 2 (D89): Claude Code's `PreToolUse` and `PermissionRequest`
/// hooks name the tool from the event's JSON on stdin, and a hook whose
/// stdin holds no usable name still reports its state.
#[test]
fn an_event_hook_takes_the_tool_name_from_stdin() {
    let socket = start_server();
    let block = block(&socket);
    let hook = |stdin: &[u8], args: &[&str]| {
        let mut all = vec!["agent", "hook", "--event"];
        all.extend(args);
        let mut child = ctl_command(&socket, &all)
            .env("RUSTY_MUX_BLOCK", &block)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("run rusty-mux-ctl");
        let mut input = child.stdin.take().expect("stdin");
        let _ = input.write_all(stdin);
        drop(input);
        let output = child.wait_with_output().expect("wait");
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(
            output.stdout.is_empty() && output.stderr.is_empty(),
            "{output:?}"
        );
        ok(&socket, &["terminal.status", "--block", &block])
    };
    let event = br#"{"hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{"command":"rm -rf build"}}"#;
    let args = [
        "blocked",
        "--kind",
        "permission",
        "--app",
        "claude",
        "--tool-from-stdin",
    ];
    // "Bash", base64: the tool name only, never its input.
    assert_eq!(
        hook(event, &args),
        "state=blocked:kind=permission:app=claude:msg=QmFzaA=="
    );
    let working = ["working", "--app", "claude", "--tool-from-stdin"];
    assert_eq!(hook(b"not json", &working), "state=working:app=claude");
    let control = br#"{"tool_name":"a\u0007b"}"#;
    assert_eq!(hook(control, &working), "state=working:app=claude");
    let huge = vec![b' '; (1 << 20) + 10];
    assert_eq!(hook(&huge, &working), "state=working:app=claude");
}

#[test]
fn an_event_hook_whose_stdin_never_ends_still_reports_in_time() {
    let socket = start_server();
    let block = block(&socket);
    let args = [
        "agent",
        "hook",
        "--event",
        "working",
        "--app",
        "codex",
        "--tool-from-stdin",
    ];
    let started = std::time::Instant::now();
    let mut child = ctl_command(&socket, &args)
        .env("RUSTY_MUX_BLOCK", &block)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run rusty-mux-ctl");
    // Written but never closed: the writer holds the pipe open throughout.
    let mut input = child.stdin.take().expect("stdin");
    input.write_all(br#"{"tool_name":"Bash""#).expect("write");
    let output = child.wait_with_output().expect("wait");
    let took = started.elapsed();
    drop(input);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(
        output.stdout.is_empty() && output.stderr.is_empty(),
        "{output:?}"
    );
    assert!(took < std::time::Duration::from_secs(3), "{took:?}");
    assert_eq!(
        ok(&socket, &["terminal.status", "--block", &block]),
        "state=working:app=codex"
    );
}

fn rename(socket: &Path, session: &str, name: &str) {
    ok(
        socket,
        &["rename-session", "--session", session, "--name", name],
    );
}

/// Runs the naming hook's instruction; true if the name was taken.
fn name_session(socket: &Path, session: &str, name: &str) -> bool {
    let output = ctl(
        socket,
        &["name-session", "--session", session, "--name", name],
    );
    match output.status.code() {
        Some(0) => true,
        Some(1) => {
            let err = String::from_utf8_lossy(&output.stderr);
            assert!(err.contains("chosen name"), "{err}");
            false
        }
        other => panic!("{other:?}: {output:?}"),
    }
}

/// The name `list` reports for `session`.
fn session_name(socket: &Path, session: &str) -> String {
    ok(socket, &["--json", "list"])
        .lines()
        .find_map(|line| {
            let rusty_json::Value::Object(map) =
                rusty_json::Value::from_json_str(line).expect("json")
            else {
                panic!("object")
            };
            let is_it = map.get("session") == Some(&rusty_json::Value::String(session.into()));
            match (is_it, map.get("name")) {
                (true, Some(rusty_json::Value::String(name))) => Some(name.clone()),
                _ => None,
            }
        })
        .expect("the session is listed")
}

#[test]
fn a_stale_naming_instruction_keeps_a_name_chosen_since() {
    let socket = start_server();
    // The hook told the agent to name this session; then a person did.
    let session = ok(&socket, &["create-session"]);
    rename(&socket, &session, "release-notes");
    assert!(!name_session(&socket, &session, "fix-login"));
    assert_eq!(session_name(&socket, &session), "release-notes");

    // Two agents told to name one session: only the first does.
    let shared = ok(&socket, &["create-session"]);
    assert!(name_session(&socket, &shared, "fix-login"));
    assert!(!name_session(&socket, &shared, "write-docs"));
    assert_eq!(session_name(&socket, &shared), "fix-login");
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
/// does not send, and `cmd.exe` sets a title. Its Windows twin follows.
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

/// Types `data` into `block` under a lease.
fn type_into(socket: &Path, block: &str, data: &str) {
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
            data,
        ],
    );
}

/// Polls `args` until it succeeds with output that passes `want`, named
/// `what`. A failure counts as "not yet": until the shell's report lands,
/// `terminal.pwd` exits 1 with "the program's working directory is
/// unknown".
#[cfg(windows)]
fn eventually(socket: &Path, args: &[&str], what: &str, want: impl Fn(&str) -> bool) {
    let deadline = std::time::Instant::now() + PATIENCE;
    loop {
        let output = ctl(socket, args);
        let got = if output.status.success() {
            stdout(&output).trim().to_owned()
        } else {
            String::from_utf8_lossy(&output.stderr).trim().to_owned()
        };
        if output.status.success() && want(&got) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{args:?}: {got:?}, never {what}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// On Windows `pwd` comes only from an OSC 7 the shell prints, and the
/// title from `title`, which the console host mirrors as OSC 0.
#[cfg(windows)]
#[test]
fn terminal_methods_read_the_block() {
    let socket = start_server();
    let block = block(&socket);
    type_into(&socket, &block, REPORT_TMP);
    let pwd = ["terminal.pwd", "--block", &block];
    eventually(&socket, &pwd, "/tmp", |got| got == "/tmp");
    type_into(&socket, &block, "title rusty-mux-title\r");
    // An elevated cmd.exe puts "Administrator: " before every title.
    let title = ["terminal.title", "--block", &block];
    eventually(&socket, &title, "rusty-mux-title", |got| {
        got.ends_with("rusty-mux-title")
    });
    let json = ok(&socket, &["--json", "terminal.pwd", "--block", &block]);
    assert!(json.contains("\"text\":\"/tmp\""), "{json}");
}

#[test]
fn terminal_size_theme_and_reset() {
    let socket = start_server();
    let block = block(&socket);
    assert_eq!(ok(&socket, &["terminal.size", "--block", &block]), "80x24");
    let leased = [
        "attach",
        "--block",
        &block,
        "+",
        "acquire-lease",
        "--block",
        &block,
        "+",
    ];
    let theme = [
        "terminal.set-theme",
        "--block",
        &block,
        "--background",
        "#102030",
        "--palette",
        "1=#cc0000",
        "--palette",
        "2=default",
    ];
    ok(&socket, &[&leased[..], &theme].concat());
    ok(
        &socket,
        &[&leased[..], &["terminal.reset", "--block", &block]].concat(),
    );
    // Without the lease the server refuses; a bad colour never leaves ctl.
    let output = ctl(&socket, &["terminal.reset", "--block", &block]);
    assert_eq!(output.status.code(), Some(1));
    let bad = [
        "terminal.set-theme",
        "--block",
        &block,
        "--background",
        "red",
    ];
    let output = ctl(&socket, &[&leased[..], &bad].concat());
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("background"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
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
    let llms = ok(nowhere, &["api", "llms"]);
    assert!(llms.starts_with("# rusty_mux API\n"), "{llms}");
    assert!(llms.contains("\n### `terminal.format`\n"), "{llms}");
    for args in [
        &["api", "describe", "nope"][..],
        &["api"],
        &["api", "call"],
        &["api", "llms", "extra"],
    ] {
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

/// `whoami` prints the caller's own client: a fresh one each run, as
/// `list` would show it, with the server's user as its principal (C25).
#[cfg(target_os = "linux")]
#[test]
fn whoami_shows_the_callers_principal_and_transport() {
    let socket = start_server();
    let me = ok(&socket, &["whoami"]);
    let uid = rusty_libc::process::geteuid();
    assert!(me.starts_with("client "), "{me}");
    assert!(me.contains(&format!("principal=\"uid:{uid}\"")), "{me}");
    assert!(me.contains("transport=\"socket\""), "{me}");
    assert_eq!(me.lines().count(), 1, "{me}");
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
        &["create-window"],
        &["resize", "--block", "x"],
    ] {
        assert_eq!(ctl(&socket, args).status.code(), Some(2), "{args:?}");
    }
    let output = Command::new(env!("CARGO_BIN_EXE_rusty-mux-ctl"))
        .env_remove("RUSTY_MUX_SOCKET")
        .env_remove("XDG_RUNTIME_DIR")
        .env_remove("LOCALAPPDATA")
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
    let said = String::from_utf8_lossy(&output.stderr);
    assert!(said.contains("no server is running"), "{said}");
}

#[cfg(unix)]
#[test]
fn a_socket_path_that_cannot_be_looked_up_keeps_its_error() {
    // A directory that is a symlink to itself: looking the socket up fails
    // with a loop, not "not found", whatever the user's permissions.
    let dir = socket_path("loop");
    std::fs::create_dir(&dir).expect("dir");
    std::os::unix::fs::symlink("loop", dir.join("loop")).expect("symlink");
    let socket = dir.join("loop").join("s.sock");
    let output = ctl(&socket, &["create-session", "--name", "x"]);
    assert_eq!(output.status.code(), Some(4));
    let said = String::from_utf8_lossy(&output.stderr);
    assert!(!said.contains("no server is running"), "{said}");
    assert!(said.contains(&socket.display().to_string()), "{said}");
    let _ = std::fs::remove_dir_all(&dir);
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
    legacy_server_with(rusty_mux_proto::Features::CHECKPOINT_V6)
}

/// [`legacy_server`] offering `features`.
fn legacy_server_with(
    features: rusty_mux_proto::Features,
) -> (PathBuf, std::sync::mpsc::Receiver<usize>) {
    use rusty_mux_core::ClientId;
    use rusty_mux_proto::{Message, decode, encode};

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
                        version: None,
                        features,
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

/// A server from before C40 has block calls but not the terminal control
/// methods: ctl refuses them before sending anything, chained or not.
#[test]
fn terminal_control_is_refused_by_a_server_without_it() {
    use rusty_mux_proto::Features;
    let block = "00000000-0000-0000-0000-000000000001";
    let older = Features::CHECKPOINT_V6
        .union(Features::SYNC)
        .union(Features::INSPECT)
        .union(Features::BLOCK_CALL);
    let chains: [&[&str]; 3] = [
        &["terminal.size", "--block", block],
        &[
            "attach",
            "--block",
            block,
            "+",
            "acquire-lease",
            "--block",
            block,
            "+",
            "terminal.reset",
            "--block",
            block,
        ],
        &[
            "terminal.set-theme",
            "--block",
            block,
            "--background",
            "#102030",
        ],
    ];
    for chain in chains {
        let (path, frames) = legacy_server_with(older);
        let output = ctl(&path, chain);
        assert_eq!(output.status.code(), Some(1), "{chain:?}");
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

/// Windows' twin: Ctrl+Break, aimed at a follow in a process group of
/// its own, ends it as the default handler does, with
/// `STATUS_CONTROL_C_EXIT`.
#[cfg(windows)]
#[test]
fn a_ctrl_break_ends_a_follow() {
    use std::os::windows::process::CommandExt;

    /// `CREATE_NEW_PROCESS_GROUP` (winbase.h): the child leads a group of
    /// its own, whose id is its pid.
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const STATUS_CONTROL_C_EXIT: u32 = 0xC000_013A;

    let socket = start_server();
    let mut child = ctl_command(&socket, &["events", "--follow"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .creation_flags(CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .expect("spawn");
    let mut lines = BufReader::new(child.stdout.take().expect("stdout")).lines();
    let first = lines.next().expect("a line").expect("read");
    assert!(first.starts_with("cursor "), "{first}");

    // GenerateConsoleCtrlEvent through PowerShell, which shares this
    // console, so the test needs no Win32 binding of its own.
    let group = child.id();
    let script = format!(
        "$k = Add-Type -Name K -Namespace RustyMuxCtlTest -PassThru -MemberDefinition \
         '[DllImport(\"kernel32.dll\")] public static extern bool GenerateConsoleCtrlEvent(uint e, uint g);'; \
         if (-not $k::GenerateConsoleCtrlEvent(1, {group})) {{ exit 1 }}"
    );
    let sent = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .stdin(Stdio::null())
        .status()
        .expect("run powershell");
    assert!(sent.success(), "Ctrl+Break to group {group}: {sent:?}");
    let status = child.wait().expect("wait");
    assert_eq!(
        status.code().map(|c| c as u32),
        Some(STATUS_CONTROL_C_EXIT),
        "{status:?}"
    );
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

/// The shell reports the directory with OSC 7 ([`REPORT_TMP`]).
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
    type_into(&socket, &block, REPORT_TMP);
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
fn events_server(features: rusty_mux_proto::Features, version: Option<&str>) -> PathBuf {
    use rusty_mux_core::ClientId;
    use rusty_mux_proto::{Cursor, Message, decode, encode};

    let version = version.map(str::to_owned);
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
            version,
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
fn an_older_server_is_named_on_stderr() {
    use rusty_mux_proto::Features;

    let old = Features::EVENT_SEQ.union(Features::SYNC);
    let pwd = config_file("older-notice", RULES);
    let refused = with_config(&events_server(old, None), &pwd, &["watch"]);
    let stderr = String::from_utf8_lossy(&refused.stderr).into_owned();
    assert!(
        stderr.contains("the server runs an unknown rusty_mux version"),
        "{stderr}"
    );
    assert!(stderr.contains("It lacks "), "{stderr}");

    let refused = with_config(&events_server(old, Some("0.0.1")), &pwd, &["watch"]);
    let stderr = String::from_utf8_lossy(&refused.stderr).into_owned();
    let named = format!(
        "the server runs an older rusty_mux (0.0.1; this is {})",
        rusty_mux_proto::VERSION
    );
    assert!(stderr.contains(&named), "{stderr}");
}

#[test]
fn watch_refuses_rules_for_events_the_server_never_sends() {
    use rusty_mux_proto::Features;

    let old = Features::EVENT_SEQ
        .union(Features::SYNC)
        .union(Features::STATUS);
    let pwd = config_file("needs-pwd", RULES);
    let refused = with_config(&events_server(old, None), &pwd, &["watch"]);
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
    let refused = with_config(&events_server(pwd_only, None), &bell, &["watch"]);
    assert_eq!(refused.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&refused.stderr).into_owned();
    assert!(stderr.contains("does not send `bell` events"), "{stderr}");
    std::fs::remove_file(&bell).expect("cleanup");

    // Rules for events every numbering server sends still run.
    let generic = config_file(
        "generic",
        r#"{"on": [{"event": "session-created", "action": "list"}]}"#,
    );
    let mut child = ctl_command(&events_server(old, None), &["watch"])
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

/// An agent waits for the program to copy to the primary selection, and
/// then for its status record to end, without reading the screen.
#[cfg(unix)]
#[test]
fn wait_for_a_clipboard_write_and_an_ended_status() {
    let socket = start_server();
    let block = block(&socket);
    let wait = |kind: &str, field: &str, value: &str| {
        ctl_command(
            &socket,
            &[
                "wait",
                "--event",
                kind,
                "--block",
                &block,
                field,
                value,
                "--timeout",
                "20",
            ],
        )
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn")
    };
    let copied = wait("clipboard-written", "--selection", "primary");
    let ended = wait("status-removed", "--id", "job");
    std::thread::sleep(Duration::from_millis(500));
    // "aGk=" is "hi". The program pauses first, so both waiters are
    // listening, and again before the clear, so it lands in a later read
    // and ends the record.
    let typed = "sleep 1; printf '\\033]7501;state=working:id=job\\007\\033]52;p;aGk=\\007'; \
                 sleep 1; printf '\\033]7501;state=clear:id=job\\007'\r";
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
            typed,
        ],
    );
    let expected = [
        (
            copied,
            format!("clipboard-written block={block} selection=primary"),
        ),
        (ended, format!("status-removed block={block} id=\"job\"")),
    ];
    for (waiter, line) in expected {
        let output = waiter.wait_with_output().expect("wait");
        assert_eq!(output.status.code(), Some(0), "{line}");
        let printed = stdout(&output);
        assert!(printed.trim_end().ends_with(&line), "{printed}");
    }
}

/// An agent selects a window and sees the session's active window move,
/// both in the listing and as an event a waiter catches.
#[test]
fn select_window_moves_the_active_window() {
    let socket = start_server();
    let session = ok(&socket, &["create-session", "--name", "s"]);
    let first = ok(
        &socket,
        &["create-window", "--session", &session, "--name", "a"],
    );
    let second = ok(
        &socket,
        &["create-window", "--session", &session, "--name", "b"],
    );
    let line = |listing: &str| {
        listing
            .lines()
            .find(|l| l.starts_with("session "))
            .expect("a session line")
            .to_owned()
    };
    assert!(line(&ok(&socket, &["list"])).ends_with(&format!("active={first}")));

    let waiter = ctl_command(
        &socket,
        &[
            "wait",
            "--event",
            "active-window-changed",
            "--window",
            &second,
            "--timeout",
            "20",
        ],
    )
    .stdout(Stdio::piped())
    .spawn()
    .expect("spawn");
    // `wait` sees only events after it subscribes, so select back and
    // forth until it has caught one, rather than guess how long that takes.
    let mut waiter = waiter;
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while waiter.try_wait().expect("poll").is_none() {
        assert!(
            std::time::Instant::now() < deadline,
            "the waiter never saw it"
        );
        ok(&socket, &["select-window", "--window", &first]);
        ok(&socket, &["select-window", "--window", &second]);
        std::thread::sleep(Duration::from_millis(100));
    }
    let output = waiter.wait_with_output().expect("wait");
    assert_eq!(output.status.code(), Some(0));
    let printed = stdout(&output);
    assert!(
        printed.trim_end().ends_with(&format!(
            "active-window-changed session={session} window={second}"
        )),
        "{printed}"
    );
    assert!(line(&ok(&socket, &["list"])).ends_with(&format!("active={second}")));
    // A window that does not exist is a missing target: exit 3.
    let gone = "00000000-0000-4000-8000-000000000000";
    assert_eq!(
        ctl(&socket, &["select-window", "--window", gone])
            .status
            .code(),
        Some(3)
    );
}

/// An agent edits a window's layout and reorders windows from the command
/// line (C41).
#[test]
fn layout_edits_reshape_a_window_and_reorder_windows() {
    let socket = start_server();
    let session = ok(&socket, &["create-session", "--name", "s"]);
    let window = ok(
        &socket,
        &["create-window", "--session", &session, "--name", "a"],
    );
    let size = ["--cols", "80", "--rows", "24"];
    let block = |socket: &Path| {
        let mut args = vec!["create-block", "--window", &window];
        args.extend(size);
        ok(socket, &args)
    };
    let (a, b) = (block(&socket), block(&socket));
    let layout = |socket: &Path| {
        ok(socket, &["list"])
            .lines()
            .find(|l| l.starts_with(&format!("window {window}")))
            .and_then(|l| l.split(" layout=").nth(1))
            .map(|rest| {
                rest.split(' ')
                    .next()
                    .unwrap_or_default()
                    .trim_matches('"')
                    .to_owned()
            })
            .expect("a window line with a layout")
    };
    assert_eq!(layout(&socket), format!("cols50({a},{b})"));
    ok(
        &socket,
        &[
            "resize-split",
            "--block",
            &b,
            "--axis",
            "cols",
            "--percent",
            "25",
        ],
    );
    assert_eq!(layout(&socket), format!("cols75({a},{b})"));
    ok(&socket, &["swap-blocks", "--block", &a, "--other", &b]);
    assert_eq!(layout(&socket), format!("cols75({b},{a})"));
    ok(&socket, &["detach-block", "--block", &b]);
    assert_eq!(layout(&socket), a);
    ok(
        &socket,
        &[
            "move-block",
            "--block",
            &b,
            "--beside",
            &a,
            "--side",
            "down",
        ],
    );
    assert_eq!(layout(&socket), format!("rows50({a},{b})"));
    // A percent outside 1 to 99 is refused before it is sent.
    let bad = ctl(
        &socket,
        &[
            "resize-split",
            "--block",
            &b,
            "--axis",
            "rows",
            "--percent",
            "100",
        ],
    );
    assert_ne!(bad.status.code(), Some(0));

    let second = ok(
        &socket,
        &["create-window", "--session", &session, "--name", "b"],
    );
    ok(
        &socket,
        &["move-window", "--window", &second, "--index", "0"],
    );
    let order: Vec<String> = ok(&socket, &["list"])
        .lines()
        .filter(|l| l.starts_with("window "))
        .filter_map(|l| l.split(' ').nth(1).map(str::to_owned))
        .collect();
    assert_eq!(order, vec![second, window]);
}

/// An agent moves a window's focus from the command line, and `list`
/// shows it (C41).
#[test]
fn focus_moves_between_blocks_and_shows_in_the_listing() {
    let socket = start_server();
    let session = ok(&socket, &["create-session", "--name", "s"]);
    let window = ok(
        &socket,
        &["create-window", "--session", &session, "--name", "a"],
    );
    let block = |socket: &Path| {
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
    };
    let (a, b) = (block(&socket), block(&socket));
    let focus = |socket: &Path| {
        ok(socket, &["list"])
            .lines()
            .find(|l| l.starts_with(&format!("window {window}")))
            .and_then(|l| l.split(" focused=").nth(1))
            .map(str::to_owned)
            .expect("a window line with its focus")
    };
    assert_eq!(focus(&socket), a);
    ok(
        &socket,
        &["move-focus", "--window", &window, "--toward", "right"],
    );
    assert_eq!(focus(&socket), b);
    ok(&socket, &["focus-block", "--block", &a]);
    assert_eq!(focus(&socket), a);
    let bad = ctl(
        &socket,
        &["move-focus", "--window", &window, "--toward", "north"],
    );
    assert_ne!(bad.status.code(), Some(0));
}

#[test]
fn reload_config_rereads_the_file_and_says_so() {
    let file = socket_path("config").with_extension("json");
    std::fs::write(&file, "{}").expect("write config");
    let socket = start_server_with(Config {
        config_file: Some(file.clone()),
        ..shell()
    });
    let mut waiter = ctl_command(
        &socket,
        &["wait", "--event", "config-reloaded", "--timeout", "20"],
    )
    .stdout(Stdio::piped())
    .spawn()
    .expect("spawn");
    // As with select-window: reload until the waiter has caught one.
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while waiter.try_wait().expect("poll").is_none() {
        assert!(
            std::time::Instant::now() < deadline,
            "the waiter never saw it"
        );
        assert_eq!(ok(&socket, &["reload-config"]), "");
        std::thread::sleep(Duration::from_millis(100));
    }
    let output = waiter.wait_with_output().expect("wait");
    assert_eq!(output.status.code(), Some(0));
    let printed = stdout(&output);
    assert!(printed.trim_end().ends_with("config-reloaded"), "{printed}");

    std::fs::write(&file, r#"{"server": {"command": ""}}"#).expect("write config");
    let refused = ctl(&socket, &["reload-config"]);
    assert_ne!(refused.status.code(), Some(0));
    let err = String::from_utf8_lossy(&refused.stderr);
    assert!(err.contains("server.command"), "{err}");
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
        version: None,
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

/// C36b: `server stop` returns once the server has stopped, so a second
/// finds no server and exits 4.
#[test]
fn server_stop_waits_for_the_stop_and_then_finds_no_server() {
    let socket = start_server();
    ok(&socket, &["create-session", "--name", "kept"]);
    assert_eq!(ok(&socket, &["server", "stop"]), "");
    let again = ctl(&socket, &["server", "stop"]);
    assert_eq!(
        again.status.code(),
        Some(4),
        "{}",
        String::from_utf8_lossy(&again.stderr)
    );
    let usage = ctl(&socket, &["server", "start"]);
    assert_eq!(usage.status.code(), Some(2));
}

/// A stand-in `ssh` that runs the remote command here, with `HOME` as the
/// remote home, and records its arguments; or that fails as ssh does when
/// it cannot log in.
#[cfg(unix)]
fn fake_ssh(dir: &Path, reachable: bool) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("ssh");
    let body = match reachable {
        true => {
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$(dirname \"$0\")/argv\"\nshift 2\nexec sh -c \"$1\"\n"
        }
        false => "#!/bin/sh\necho 'ssh: connect to host box: refused' >&2\nexit 255\n",
    };
    std::fs::write(&path, body).expect("write fake ssh");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    path
}

#[cfg(unix)]
#[test]
fn terminfo_installs_checks_and_removes_the_entry_over_ssh() {
    if Command::new("tic").arg("-V").output().is_err() {
        eprintln!("skipped: no tic");
        return;
    }
    let dir = ssh_scratch("terminfo");
    let home = dir.join("remote-home");
    std::fs::create_dir_all(&home).expect("home");
    let ssh = fake_ssh(&dir, true);
    let terminfo = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_rusty-mux-ctl"))
            .env("RUSTY_MUX_SSH", &ssh)
            .env("HOME", &home)
            .env_remove("TERMINFO")
            .env_remove("TERMINFO_DIRS")
            .args(args)
            .output()
            .expect("run rusty-mux-ctl")
    };
    let entry = home.join(".terminfo").join("r").join("rusty_term");

    let missing = terminfo(&["terminfo", "check", "me@box"]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(
        stdout(&missing).contains("rusty_term: "),
        "{}",
        stdout(&missing)
    );
    let argv = std::fs::read_to_string(dir.join("argv")).expect("argv");
    assert!(argv.starts_with("--\nme@box\nsh -c '"), "{argv}");

    let dry = terminfo(&["terminfo", "setup", "me@box", "--dry-run"]);
    assert_eq!(dry.status.code(), Some(0));
    assert!(stdout(&dry).contains("tic -x -o"), "{}", stdout(&dry));
    assert!(!entry.exists(), "a dry run writes nothing");

    assert_eq!(
        terminfo(&["terminfo", "setup", "me@box"]).status.code(),
        Some(0)
    );
    assert!(entry.exists());
    let found = terminfo(&["terminfo", "check", "me@box"]);
    assert_eq!(found.status.code(), Some(0), "{}", stdout(&found));
    assert!(stdout(&found).contains("installed in ~/.terminfo"));

    assert_eq!(
        terminfo(&["terminfo", "remove", "me@box"]).status.code(),
        Some(0)
    );
    assert!(!entry.exists());
    let _gone = std::fs::remove_dir_all(dir);
}

#[cfg(unix)]
#[test]
fn terminfo_exits_4_when_ssh_cannot_log_in() {
    let dir = ssh_scratch("terminfo-down");
    std::fs::create_dir_all(&dir).expect("dir");
    let ssh = fake_ssh(&dir, false);
    let output = Command::new(env!("CARGO_BIN_EXE_rusty-mux-ctl"))
        .env("RUSTY_MUX_SSH", &ssh)
        .args(["terminfo", "check", "box"])
        .output()
        .expect("run rusty-mux-ctl");
    assert_eq!(output.status.code(), Some(4));
    for bad in [
        &["terminfo", "check", "-oProxyCommand=x"][..],
        &["terminfo", "check", "box", "--dry-run"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusty-mux-ctl"))
            .env("RUSTY_MUX_SSH", &ssh)
            .args(bad)
            .output()
            .expect("run rusty-mux-ctl");
        assert_eq!(output.status.code(), Some(2), "{bad:?}");
    }
    let _gone = std::fs::remove_dir_all(dir);
}

/// A directory of its own for one test.
#[cfg(unix)]
fn ssh_scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rusty-mux-ctl-ssh-{}-{tag}", std::process::id()));
    let _fresh = std::fs::remove_dir_all(&dir);
    dir
}

/// A directory of its own for one ssh test.
#[cfg(unix)]
fn ssh_config_dir(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("rusty-mux-ctl-sshcfg-{}-{tag}", std::process::id()));
    let _fresh = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    dir
}

#[cfg(unix)]
fn executable(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, body).expect("write script");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
}

/// `rusty-mux-ctl ARGS` with a stand-in `ssh` whose `-G` prints `resolved`
/// and an `ssh-keygen` that knows only `known`, under `home`.
#[cfg(unix)]
fn ssh_ctl(dir: &Path, resolved: &str, known: &str, args: &[&str]) -> Output {
    let ssh = dir.join("ssh");
    std::fs::write(dir.join("resolved"), resolved).expect("write");
    executable(
        &ssh,
        "#!/bin/sh\nd=$(dirname \"$0\")\nprintf '%s\\n' \"$@\" > \"$d/argv\"\n\
         case \"$1\" in -V) echo 'OpenSSH_9.6p1' >&2 ;; -G) cat \"$d/resolved\" ;; *) exit 255 ;; esac\n",
    );
    executable(
        &dir.join("ssh-keygen"),
        &format!("#!/bin/sh\n[ \"$2\" = '{known}' ]\n"),
    );
    Command::new(env!("CARGO_BIN_EXE_rusty-mux-ctl"))
        .env("RUSTY_MUX_SSH", &ssh)
        .env("HOME", dir)
        .env_remove("SSH_AUTH_SOCK")
        .args(args)
        .output()
        .expect("run rusty-mux-ctl")
}

#[cfg(unix)]
#[test]
fn ssh_check_reads_what_ssh_resolves_and_never_connects() {
    let dir = ssh_config_dir("check");
    std::fs::create_dir_all(dir.join(".ssh")).expect("dir");
    std::fs::write(dir.join(".ssh/id_ed25519"), "key").expect("key");
    std::fs::write(dir.join(".ssh/known_hosts"), "box.example ssh-ed25519 AAAA").expect("known");
    let resolved = "hostname box.example\nport 22\nidentityfile ~/.ssh/id_ed25519\n\
        clearallforwardings no\nuserknownhostsfile ~/.ssh/known_hosts\n";

    let fine = ssh_ctl(&dir, resolved, "box.example", &["ssh", "check", "box"]);
    assert_eq!(fine.status.code(), Some(0), "{}", stdout(&fine));
    assert!(stdout(&fine).starts_with("ok: "), "{}", stdout(&fine));
    let argv = std::fs::read_to_string(dir.join("argv")).expect("argv");
    assert!(
        argv.starts_with("-G\n") && argv.ends_with("--\nbox\n"),
        "{argv}"
    );
    assert!(argv.contains("ForkAfterAuthentication=no\n") && argv.contains("BatchMode=yes\n"));

    let unknown = ssh_ctl(&dir, resolved, "other", &["ssh", "diagnose", "box"]);
    assert_eq!(unknown.status.code(), Some(1));
    let said = stdout(&unknown);
    assert!(said.contains("hostname: box.example"), "{said}");
    assert!(
        said.contains("problem: no known-hosts file has box.example's key"),
        "{said}"
    );
    assert!(said.contains("  fix: "), "{said}");

    std::fs::write(dir.join("resolved"), "").expect("write");
    executable(
        &dir.join("ssh"),
        "#!/bin/sh\necho 'Bad configuration option: forkafterauthentication' >&2\nexit 255\n",
    );
    let old = Command::new(env!("CARGO_BIN_EXE_rusty-mux-ctl"))
        .env("RUSTY_MUX_SSH", dir.join("ssh"))
        .env("HOME", &dir)
        .args(["ssh", "check", "box"])
        .output()
        .expect("run rusty-mux-ctl");
    assert_eq!(old.status.code(), Some(1));
    assert!(stdout(&old).contains("OpenSSH 8.7"), "{}", stdout(&old));
    let _gone = std::fs::remove_dir_all(dir);
}

#[cfg(unix)]
#[test]
fn ssh_setup_prints_a_block_once() {
    let dir = ssh_config_dir("setup");
    let args = [
        "ssh",
        "setup",
        "box",
        "--hostname",
        "box.example",
        "--port",
        "2222",
    ];
    let printed = ssh_ctl(&dir, "", "", &args);
    assert_eq!(printed.status.code(), Some(0));
    let block = stdout(&printed);
    assert!(
        block.starts_with("# BEGIN rusty-mux-ctl ssh setup box\nHost box\n"),
        "{block}"
    );
    assert!(block.contains("    Port 2222\n"), "{block}");
    assert!(!dir.join(".ssh").exists(), "setup writes nothing");

    std::fs::create_dir_all(dir.join(".ssh")).expect("dir");
    std::fs::write(dir.join(".ssh/config"), block).expect("write");
    let again = ssh_ctl(&dir, "", "", &args);
    assert!(
        stdout(&again).contains("already has a Host entry for box"),
        "{}",
        stdout(&again)
    );
    for bad in [
        &["ssh", "setup", "box", "--port", "0"][..],
        &["ssh", "check", "-oProxyCommand=x"],
    ] {
        assert_eq!(ssh_ctl(&dir, "", "", bad).status.code(), Some(2), "{bad:?}");
    }
    let _gone = std::fs::remove_dir_all(dir);
}
