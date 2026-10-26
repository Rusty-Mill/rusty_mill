//! `rusty-mux-mcp`'s tools over MCP stdio against a real server (C07).

#![cfg(target_os = "linux")]

use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rusty_mcp_server::json::Value;
use rusty_mcp_server::{StdioConfig, serve_lines};
use rusty_mux_ctl::Env;
use rusty_mux_proto::schema::CATALOG;
use rusty_mux_server::{Config, serve};
use rusty_tokio::io::UnixListener;
use rusty_tokio::{Builder, LocalOptions};

const PATIENCE: Duration = Duration::from_secs(20);
const INIT: &str = r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#;

fn start_server() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("rusty-mux-mcp-{}-{n}.sock", std::process::id()));
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

/// Runs one MCP session of `requests` (after initializing) and returns the
/// reply to each, by id. Requests in a session run concurrently, so a test
/// that needs an order runs one session per step.
fn session(socket: &Path, requests: &[String]) -> Vec<Value> {
    let env = Env {
        socket: Some(socket.to_path_buf()),
        here: Default::default(),
    };
    let server = Arc::new(rusty_mux_mcp::server(env).expect("server"));
    let mut lines = vec![
        INIT.to_owned(),
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_owned(),
    ];
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
    let request = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"{tool}","arguments":{args}}}}}"#
    );
    let reply = session(socket, &[request]).remove(0);
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
        if screen.lines().any(|l| l == "HI2") {
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
