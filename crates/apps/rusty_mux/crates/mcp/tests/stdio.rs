//! `rusty-mux-mcp`'s tools over MCP stdio against a real server (C07).

#![cfg(target_os = "linux")]

use std::io::{Cursor, Read, Write};
use std::os::unix::net::{UnixListener as StdListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rusty_mcp_server::Server;
use rusty_mcp_server::json::Value;
use rusty_mcp_server::{StdioConfig, serve_lines};
use rusty_mux_ctl::Env;
use rusty_mux_mcp::DEFAULT_TIMEOUT;
use rusty_mux_proto::schema::CATALOG;
use rusty_mux_server::{Config, ConsoleHost, serve};
use rusty_tokio::io::UnixListener;
use rusty_tokio::{Builder, LocalOptions};

const PATIENCE: Duration = Duration::from_secs(20);
const INIT: &str = r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#;

fn socket_path() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("rusty-mux-mcp-{}-{n}.sock", std::process::id()))
}

fn start_server() -> PathBuf {
    let path = socket_path();
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
                console_host: ConsoleHost::default(),
            };
            serve(listener, config).await.expect("serve");
        });
    });
    ready_rx.recv_timeout(PATIENCE).expect("server bound");
    path
}

/// Collects what the MCP server writes.
#[derive(Clone, Default)]
struct Out(Arc<Mutex<Vec<u8>>>);

impl Write for Out {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("lock").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A server that accepts one connection and never answers. It reports
/// the accept, then the connection's end, on the returned channel.
fn start_stalled_server() -> (PathBuf, Receiver<&'static str>) {
    let path = socket_path();
    let listener = StdListener::bind(&path).expect("bind");
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let _ = tx.send("accepted");
        let mut sink = [0u8; 4096];
        while matches!(stream.read(&mut sink), Ok(n) if n > 0) {}
        let _ = tx.send("closed");
    });
    (path, rx)
}

/// A server from before `LAUNCH`: it welcomes with no features, then
/// reports how many requests it got before the client hung up.
fn start_server_without_launch() -> (PathBuf, Receiver<usize>) {
    use rusty_mux_proto::{Message, decode};

    // Welcome with no features and the nil client id, encoded by hand: the
    // client id type lives in a crate this one does not depend on. Length,
    // type, then features (field 10) = 0 and client (field 5) = 16 zeros.
    let mut welcome = vec![0, 0, 0, 21, 0x81, 10 << 1, 0, 5 << 1 | 1, 16];
    welcome.extend([0; 16]);

    let path = socket_path();
    let listener = StdListener::bind(&path).expect("bind");
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let (mut buf, mut chunk, mut requests) = (Vec::new(), [0u8; 4096], 0);
        loop {
            while let Ok(Some((message, used))) = decode(&buf) {
                buf.drain(..used);
                let Message::Hello { .. } = message else {
                    requests += 1;
                    continue;
                };
                stream.write_all(&welcome).expect("write");
            }
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        }
        let _ = tx.send(requests);
    });
    (path, rx)
}

/// A listening socket nobody accepts on, its one-slot backlog already
/// taken, so the next connect waits for room. Dropping it closes both.
struct FullBacklog {
    listener: i32,
    _queued: UnixStream,
}

impl Drop for FullBacklog {
    fn drop(&mut self) {
        let _ = rusty_libc::fd::close(self.listener);
    }
}

fn start_full_server() -> (PathBuf, FullBacklog) {
    use rusty_libc::socket::{
        AF_UNIX, SOCK_CLOEXEC, SOCK_STREAM, SockAddrUn, bind_un, listen, socket,
    };
    let path = socket_path();
    let addr = SockAddrUn::new(path.as_os_str().as_encoded_bytes()).expect("path fits");
    let listener = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0).expect("socket");
    bind_un(listener, &addr).expect("bind");
    listen(listener, 0).expect("listen");
    let queued = UnixStream::connect(&path).expect("the one queued connection");
    (
        path,
        FullBacklog {
            listener,
            _queued: queued,
        },
    )
}

fn mcp(socket: &Path, timeout: Duration) -> Arc<Server> {
    let env = Env {
        socket: Some(socket.to_path_buf()),
        ..Env::default()
    };
    Arc::new(rusty_mux_mcp::server(env, timeout).expect("server"))
}

fn opening() -> Vec<String> {
    vec![
        INIT.to_owned(),
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_owned(),
    ]
}

fn tool_call(id: u32, tool: &str, args: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"{tool}","arguments":{args}}}}}"#
    )
}

/// Runs one MCP session of `requests` (after initializing) and returns the
/// reply to each, by id. Requests in a session run concurrently, so a test
/// that needs an order runs one session per step.
fn session(socket: &Path, requests: &[String]) -> Vec<Value> {
    session_within(socket, DEFAULT_TIMEOUT, requests)
}

fn session_within(socket: &Path, timeout: Duration, requests: &[String]) -> Vec<Value> {
    let server = mcp(socket, timeout);
    let mut lines = opening();
    lines.extend(requests.iter().cloned());
    let out = Out::default();
    serve_lines(
        server,
        Cursor::new(lines.join("\n").into_bytes()),
        out.clone(),
        StdioConfig::default(),
    )
    .expect("serve");
    let bytes = out.0.lock().expect("lock").clone();
    let replies: Vec<Value> = String::from_utf8(bytes)
        .expect("utf-8")
        .lines()
        .map(|l| Value::from_json_str(l).expect("JSON"))
        .collect();
    (1..=requests.len() as i64)
        .map(|id| {
            replies
                .iter()
                .find(|r| r.get("id").and_then(Value::as_i64) == Some(id))
                .cloned()
                .unwrap_or_else(|| panic!("no reply {id}: {replies:?}"))
        })
        .collect()
}

/// Calls one tool; returns its text and whether it failed.
fn call(socket: &Path, tool: &str, args: &str) -> (String, bool) {
    call_within(socket, DEFAULT_TIMEOUT, tool, args)
}

fn call_within(socket: &Path, timeout: Duration, tool: &str, args: &str) -> (String, bool) {
    let reply = session_within(socket, timeout, &[tool_call(1, tool, args)]).remove(0);
    let result = reply.get("result").unwrap_or_else(|| panic!("{reply:?}"));
    let text = result
        .get("content")
        .and_then(|c| c.get_index(0))
        .and_then(|c| c.get("text"))
        .and_then(Value::as_str)
        .expect("text")
        .to_owned();
    let failed = result.get("isError").and_then(Value::as_bool) == Some(true);
    (text, failed)
}

fn ok(socket: &Path, tool: &str, args: &str) -> String {
    let (text, failed) = call(socket, tool, args);
    assert!(!failed, "{tool} {args}: {text}");
    text
}

#[test]
fn tools_list_is_the_catalog_without_connection_methods() {
    let socket = start_server();
    let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#.to_owned();
    let reply = session(&socket, &[request]).remove(0);
    let tools = reply
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(Value::as_array)
        .expect("tools");
    let names: Vec<&str> = tools
        .iter()
        .filter_map(|t| t.get("name").and_then(Value::as_str))
        .collect();
    let connection = ["attach", "detach", "acquire-lease", "release-lease", "sync"];
    let expected: Vec<&str> = CATALOG
        .iter()
        .map(|m| m.name)
        .filter(|n| !connection.contains(n))
        .collect();
    assert_eq!(names, expected);
    for tool in tools {
        assert!(tool.get("inputSchema").is_some() && tool.get("description").is_some());
    }
}

#[test]
fn an_agent_builds_a_block_types_into_it_and_reads_it_back_by_name() {
    let socket = start_server();
    ok(&socket, "create-session", r#"{"name": "agents"}"#);
    ok(
        &socket,
        "create-window",
        r#"{"session": "agents", "name": "work"}"#,
    );
    let block = ok(
        &socket,
        "create-block",
        r#"{"window": "work", "cols": 80, "rows": 24}"#,
    );
    assert!(
        block.parse::<rusty_uuid::Uuid>().is_ok(),
        "the new id: {block}"
    );

    let typed = ok(
        &socket,
        "input",
        r#"{"block": "agents/work/1", "data": "echo HI$((1+1))\r"}"#,
    );
    assert_eq!(typed, "ok");
    let deadline = Instant::now() + PATIENCE;
    loop {
        let screen = ok(
            &socket,
            "terminal.format",
            r#"{"block": "agents/work/1", "history": 0}"#,
        );
        // A prompt printed after the echo can share the output's line.
        if screen.lines().any(|l| l.ends_with("HI2")) {
            break;
        }
        assert!(Instant::now() < deadline, "never saw the output: {screen}");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(ok(&socket, "list", "{}").contains("agents"));
    assert_eq!(
        ok(
            &socket,
            "close-block",
            &format!(r#"{{"block": "{block}"}}"#)
        ),
        "ok"
    );
}

#[test]
fn refusals_and_bad_arguments_are_tool_errors() {
    let socket = start_server();
    let (text, failed) = call(&socket, "terminal.title", r#"{"block": "nowhere"}"#);
    assert!(failed && text.contains("nowhere"), "{text}");
    let (text, failed) = call(&socket, "terminal.title", r#"{"block": "x", "extra": "y"}"#);
    assert!(
        failed && text.contains("unknown parameter `extra`"),
        "{text}"
    );

    let unreachable = std::env::temp_dir().join("rusty-mux-mcp-nothing-here.sock");
    let (text, failed) = call(&unreachable, "list", "{}");
    assert!(failed && text.contains("nothing-here"), "{text}");
}

#[test]
fn a_silent_server_fails_the_call_at_its_deadline() {
    let (socket, _events) = start_stalled_server();
    let started = Instant::now();
    let (text, failed) = call_within(&socket, Duration::from_millis(300), "list", "{}");
    assert!(failed && text.contains("deadline"), "{text}");
    assert!(started.elapsed() < PATIENCE, "{:?}", started.elapsed());
}

/// MCP input fed a line at a time, ending when the sender is dropped.
struct Feed {
    lines: Receiver<String>,
    pending: Vec<u8>,
}

impl Read for Feed {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.pending.is_empty() {
            match self.lines.recv() {
                Ok(line) => self.pending = format!("{line}\n").into_bytes(),
                Err(_) => return Ok(0),
            }
        }
        let n = buf.len().min(self.pending.len());
        buf[..n].copy_from_slice(&self.pending[..n]);
        self.pending.drain(..n);
        Ok(n)
    }
}

const CANCEL_1: &str =
    r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}"#;

/// Serves an MCP session fed line by line, already initialized and with a
/// `list` call (id 1) under way, that waits an hour for running calls once
/// its input ends. The receiver gets the session's end.
fn serve_list_call(socket: &Path) -> (Sender<String>, Receiver<std::io::Result<()>>) {
    let (feed, lines) = channel();
    let (done_tx, done) = channel();
    let server = mcp(socket, Duration::from_secs(3600));
    let config = StdioConfig {
        drain_timeout: Duration::from_secs(3600),
        ..StdioConfig::default()
    };
    std::thread::spawn(move || {
        let input = std::io::BufReader::new(Feed {
            lines,
            pending: Vec::new(),
        });
        let _ = done_tx.send(serve_lines(server, input, Out::default(), config));
    });
    for line in opening() {
        feed.send(line).expect("feed");
    }
    feed.send(tool_call(1, "list", "{}")).expect("feed");
    (feed, done)
}

#[test]
fn cancelling_a_call_to_a_silent_server_releases_its_connection() {
    let (socket, events) = start_stalled_server();
    let (feed, done) = serve_list_call(&socket);
    assert_eq!(events.recv_timeout(PATIENCE), Ok("accepted"));

    feed.send(CANCEL_1.to_owned()).expect("feed");
    assert_eq!(
        events.recv_timeout(Duration::from_secs(5)),
        Ok("closed"),
        "the call kept its connection after the cancel"
    );
    drop(feed);
    let ended = done
        .recv_timeout(Duration::from_secs(5))
        .expect("session ended");
    ended.expect("serve");
}

#[test]
fn a_full_backlog_fails_the_call_at_its_deadline() {
    let (socket, _full) = start_full_server();
    let started = Instant::now();
    let (text, failed) = call_within(&socket, Duration::from_millis(300), "list", "{}");
    assert!(failed && text.contains("deadline"), "{text}");
    assert!(started.elapsed() < PATIENCE, "{:?}", started.elapsed());
}

#[test]
fn cancelling_a_call_stuck_connecting_ends_it() {
    let (socket, _full) = start_full_server();
    let (feed, done) = serve_list_call(&socket);
    // Nothing reports a connect in progress; give the call time to start.
    std::thread::sleep(Duration::from_millis(300));
    feed.send(CANCEL_1.to_owned()).expect("feed");
    drop(feed);
    let ended = done
        .recv_timeout(Duration::from_secs(5))
        .expect("the cancelled call still held the session open");
    ended.expect("serve");
}

#[test]
fn a_launch_the_server_would_ignore_is_a_tool_error_and_sends_nothing() {
    let (socket, requests) = start_server_without_launch();
    let (text, failed) = call(
        &socket,
        "create-block",
        concat!(
            r#"{"window": "00000000-0000-0000-0000-000000000001", "cols": 80, "rows": 24, "#,
            r#""command": "sh", "arg": ["-c", "exit 3"]}"#
        ),
    );
    assert!(failed, "{text}");
    assert!(
        text.contains("does not support create-block with --"),
        "{text}"
    );
    assert_eq!(requests.recv_timeout(PATIENCE).expect("count"), 0);
}
