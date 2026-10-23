//! End to end over a real socket: framing, keep-alive, limits, shutdown.

use rusty_tick::backend::Backend;
use rusty_tick::server::{bind, ShutdownHandle, MAX_BODY_BYTES};
use rusty_tick::service::system_clock;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::thread::JoinHandle;
use std::time::Duration;

const TOKEN: &str = "test-token-0123456789";

struct Running {
    addr: SocketAddr,
    stop: ShutdownHandle,
    thread: Option<JoinHandle<()>>,
    _dir: tempfile::TempDir,
}

impl Running {
    fn start() -> Self {
        Self::start_with(None)
    }

    fn start_with(web_dir: Option<std::path::PathBuf>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::single(dir.path(), TOKEN.into(), system_clock()).unwrap();
        Self::serve(backend, dir, web_dir)
    }

    fn serve(
        backend: Backend,
        dir: tempfile::TempDir,
        web_dir: Option<std::path::PathBuf>,
    ) -> Self {
        let mut server = bind("127.0.0.1:0".parse().unwrap(), backend).unwrap();
        if let Some(web) = web_dir {
            server = server.with_web_dir(web);
        }
        let (addr, stop) = (
            server.local_addr().unwrap(),
            server.shutdown_handle().unwrap(),
        );
        let thread = std::thread::spawn(move || server.run().unwrap());
        Self {
            addr,
            stop,
            thread: Some(thread),
            _dir: dir,
        }
    }

    fn connect(&self) -> TcpStream {
        let stream = TcpStream::connect(self.addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop.shutdown();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn request(method: &str, path: &str, body: &str, extra: &str) -> String {
    format!(
        "{method} {path} HTTP/1.1\r\nHost: t\r\nAuthorization: Bearer {TOKEN}\r\n{extra}Content-Length: {}\r\n\r\n{body}",
        body.len()
    )
}

/// Send `raw` and read until the server closes (the request says `Connection: close`).
fn exchange(server: &Running, raw: &str) -> String {
    let mut stream = server.connect();
    stream.write_all(raw.as_bytes()).unwrap();
    let mut out = String::new();
    let _ = stream.read_to_string(&mut out);
    out
}

fn status(response: &str) -> u16 {
    response.split(' ').nth(1).unwrap().parse().unwrap()
}

fn body(response: &str) -> &str {
    response.split_once("\r\n\r\n").map_or("", |(_, b)| b)
}

/// Read exactly one response off a keep-alive connection.
fn read_one(stream: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        assert_eq!(
            stream.read(&mut byte).unwrap(),
            1,
            "connection closed early"
        );
        buf.push(byte[0]);
    }
    let head = String::from_utf8(buf).unwrap();
    let len: usize = head
        .lines()
        .find_map(|l| {
            l.strip_prefix("Content-Length: ")
                .or_else(|| l.strip_prefix("content-length: "))
        })
        .map_or(0, |v| v.trim().parse().unwrap());
    let mut rest = vec![0u8; len];
    stream.read_exact(&mut rest).unwrap();
    head + &String::from_utf8(rest).unwrap()
}

#[test]
fn round_trip_over_a_socket() {
    let server = Running::start();
    let health = exchange(
        &server,
        "GET /health HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n",
    );
    assert_eq!(status(&health), 200);
    assert_eq!(body(&health), r#"{"status":"ok"}"#);
    assert!(
        health.contains("Content-Type: application/json"),
        "{health}"
    );

    let created = exchange(
        &server,
        &request(
            "POST",
            "/api/v1/lists",
            r#"{"name":"Inbox"}"#,
            "Connection: close\r\n",
        ),
    );
    assert_eq!(status(&created), 201, "{created}");
    assert!(body(&created).contains(r#""name":"Inbox""#));

    let unauth = exchange(
        &server,
        "GET /api/v1/lists HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n",
    );
    assert_eq!(status(&unauth), 401);
    assert!(unauth.contains("WWW-Authenticate: Bearer"), "{unauth}");
}

#[test]
fn keep_alive_serves_several_requests_on_one_connection() {
    let server = Running::start();
    let mut stream = server.connect();
    for name in ["a", "b"] {
        let req = request(
            "POST",
            "/api/v1/lists",
            &format!(r#"{{"name":"{name}"}}"#),
            "",
        );
        stream.write_all(req.as_bytes()).unwrap();
        assert_eq!(status(&read_one(&mut stream)), 201);
    }
    stream
        .write_all(request("GET", "/api/v1/lists", "", "").as_bytes())
        .unwrap();
    let listing = read_one(&mut stream);
    assert!(body(&listing).contains(r#""name":"a""#) && body(&listing).contains(r#""name":"b""#));
}

#[test]
fn a_204_has_no_body_and_no_content_length() {
    let server = Running::start();
    let list = exchange(
        &server,
        &request(
            "POST",
            "/api/v1/lists",
            r#"{"name":"x"}"#,
            "Connection: close\r\n",
        ),
    );
    let id = body(&list)
        .split(r#""id":""#)
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap()
        .to_string();
    let deleted = exchange(
        &server,
        &request(
            "DELETE",
            &format!("/api/v1/lists/{id}"),
            "",
            "Connection: close\r\n",
        ),
    );
    assert_eq!(status(&deleted), 204);
    assert!(
        !deleted.to_ascii_lowercase().contains("content-length"),
        "{deleted}"
    );
    assert_eq!(body(&deleted), "");
}

#[test]
fn oversized_and_chunked_bodies_and_garbage_are_refused() {
    let server = Running::start();
    let huge = format!(
        "POST /api/v1/lists HTTP/1.1\r\nHost: t\r\nAuthorization: Bearer {TOKEN}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        MAX_BODY_BYTES + 1
    );
    assert_eq!(status(&exchange(&server, &huge)), 413);

    let chunked = format!(
        "POST /api/v1/lists HTTP/1.1\r\nHost: t\r\nAuthorization: Bearer {TOKEN}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n0\r\n\r\n"
    );
    assert_eq!(status(&exchange(&server, &chunked)), 411);

    assert_eq!(status(&exchange(&server, "NOT HTTP AT ALL\r\n\r\n")), 400);

    let long_header = format!("GET /health HTTP/1.1\r\nX: {}\r\n\r\n", "a".repeat(20_000));
    assert_eq!(
        status(&exchange(&server, &long_header)),
        400,
        "head over the size limit"
    );
}

#[test]
fn shutdown_stops_the_accept_loop() {
    let mut server = Running::start();
    server.stop.shutdown();
    server.thread.take().unwrap().join().unwrap();
}

#[test]
fn serves_the_web_ui_without_shadowing_the_api() {
    let web = tempfile::tempdir().unwrap();
    std::fs::write(
        web.path().join("index.html"),
        "<!doctype html><title>Tick Local</title>",
    )
    .unwrap();
    std::fs::create_dir(web.path().join("assets")).unwrap();
    std::fs::write(web.path().join("assets/app.js"), "console.log(1)").unwrap();
    let outside = web.path().parent().unwrap().join("outside-secret.txt");
    std::fs::write(&outside, "secret").unwrap();
    let server = Running::start_with(Some(web.path().to_path_buf()));
    let get = |path: &str| {
        exchange(
            &server,
            &format!("GET {path} HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n"),
        )
    };

    let page = get("/");
    assert_eq!(status(&page), 200);
    assert!(body(&page).contains("Tick Local"));
    assert!(
        page.contains("Content-Security-Policy: default-src 'self'"),
        "{page}"
    );
    assert!(page.contains("X-Frame-Options: DENY"));

    let js = get("/assets/app.js");
    assert!(
        js.contains("text/javascript") && body(&js) == "console.log(1)",
        "{js}"
    );
    assert!(
        !js.contains("Content-Security-Policy"),
        "the policy rides on pages, not scripts"
    );

    assert!(
        body(&get("/some/client/route")).contains("Tick Local"),
        "unknown paths get the app shell"
    );
    assert!(!body(&get("/../outside-secret.txt")).contains("secret"));
    assert!(!body(&get("/%2e%2e/outside-secret.txt")).contains("secret"));

    assert_eq!(
        body(&get("/health")),
        r#"{"status":"ok"}"#,
        "the API still wins over static files"
    );
    assert_eq!(
        status(&get("/api/v1/lists")),
        401,
        "API paths are never served from disk"
    );
    assert_eq!(status(&get("/api/v1/nope")), 401);
    std::fs::remove_file(outside).unwrap();
}

#[test]
fn without_a_web_dir_only_the_api_answers() {
    let server = Running::start();
    let root = exchange(
        &server,
        "GET / HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n",
    );
    assert_eq!(
        status(&root),
        401,
        "no UI is served, and the API needs a token"
    );
}

/// One JSON-RPC `message` to `/mcp`, as a Streamable HTTP client sends it; the whole response.
fn mcp(server: &Running, token: Option<&str>, session: Option<&str>, message: &str) -> String {
    let auth = token.map_or(String::new(), |t| format!("Authorization: Bearer {t}\r\n"));
    let sid = session.map_or(String::new(), |s| format!("Mcp-Session-Id: {s}\r\n"));
    exchange(
        server,
        &format!(
            "POST /mcp HTTP/1.1\r\nHost: t\r\n{auth}{sid}Accept: application/json, text/event-stream\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{message}",
            message.len()
        ),
    )
}

fn header<'a>(response: &'a str, name: &str) -> Option<&'a str> {
    let head = response.split_once("\r\n\r\n").map_or(response, |(h, _)| h);
    head.lines().find_map(|l| {
        let (n, v) = l.split_once(':')?;
        n.eq_ignore_ascii_case(name).then_some(v.trim())
    })
}

fn call(server: &Running, session: &str, id: u32, tool: &str, args: &str) -> String {
    call_as(server, TOKEN, session, id, tool, args)
}

fn call_as(
    server: &Running,
    token: &str,
    session: &str,
    id: u32,
    tool: &str,
    args: &str,
) -> String {
    let message = format!(
        r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"{tool}","arguments":{args}}}}}"#
    );
    let response = mcp(server, Some(token), Some(session), &message);
    assert_eq!(status(&response), 200, "{response}");
    body(&response).to_string()
}

#[test]
fn mcp_needs_a_token_and_runs_tools_against_the_callers_tasks() {
    let server = Running::start();
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#;
    assert_eq!(status(&mcp(&server, None, None, init)), 401);
    assert_eq!(
        status(&mcp(&server, Some("wrong-token-wrong-token"), None, init)),
        401
    );

    let opened = mcp(&server, Some(TOKEN), None, init);
    assert_eq!(status(&opened), 200, "{opened}");
    let session = header(&opened, "mcp-session-id")
        .expect("a session id")
        .to_string();
    let ready = mcp(
        &server,
        Some(TOKEN),
        Some(&session),
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    );
    assert_eq!(status(&ready), 202, "{ready}");

    let tools = mcp(
        &server,
        Some(TOKEN),
        Some(&session),
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
    );
    for name in [
        "list_lists",
        "add_task",
        "search_tasks",
        "complete_task",
        "delete_task",
    ] {
        assert!(
            body(&tools).contains(&format!(r#""name":"{name}""#)),
            "{name}: {tools}"
        );
    }

    let added = call(&server, &session, 3, "add_task", r#"{"title":"Buy milk"}"#);
    assert!(
        added.contains("Buy milk") && !added.contains(r#""isError":true"#),
        "{added}"
    );
    let found = call(&server, &session, 4, "search_tasks", r#"{"query":"milk"}"#);
    assert!(found.contains("Buy milk"), "{found}");
    let api = exchange(
        &server,
        &request("GET", "/api/v1/search?q=milk", "", "Connection: close\r\n"),
    );
    assert!(body(&api).contains("Buy milk"), "{api}");

    let unknown = call(
        &server,
        &session,
        5,
        "add_task",
        r#"{"title":"x","colour":"red"}"#,
    );
    assert!(
        unknown.contains(r#""isError":true"#),
        "the API refuses an unknown field: {unknown}"
    );
    let reshaped = call(&server, &session, 6, "get_task", r#"{"id":"../trash"}"#);
    assert!(
        reshaped.contains(r#""isError":true"#) && reshaped.contains("not a task or list id"),
        "{reshaped}"
    );
}

/// An MCP session opened with `token`: its id.
fn open_session(server: &Running, token: &str) -> String {
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#;
    let opened = mcp(server, Some(token), None, init);
    assert_eq!(status(&opened), 200, "{opened}");
    let session = header(&opened, "mcp-session-id")
        .expect("a session id")
        .to_string();
    let ready = mcp(
        server,
        Some(token),
        Some(&session),
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    );
    assert_eq!(status(&ready), 202, "{ready}");
    session
}

/// The id of the one task titled `title`, read through the API.
fn task_id(server: &Running, token: &str, title: &str) -> String {
    let word = title.split(' ').next().unwrap_or("");
    let raw = format!("GET /api/v1/search?q={word} HTTP/1.1\r\nHost: t\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n");
    let found: rusty_json::Value = rusty_json::from_str(body(&exchange(server, &raw))).unwrap();
    let tasks = found["tasks"].as_array().unwrap();
    let task = tasks
        .iter()
        .find(|t| t["title"].as_str() == Some(title))
        .expect("the task");
    task["id"].as_str().unwrap().to_string()
}

#[test]
fn mcp_changes_tasks_through_each_tool() {
    let server = Running::start();
    let session = open_session(&server, TOKEN);
    let ok = |id: u32, tool: &str, args: &str| {
        let out = call(&server, &session, id, tool, args);
        assert!(!out.contains(r#""isError":true"#), "{tool}: {out}");
        out
    };
    ok(2, "add_task", r#"{"title":"Pay rent","dueMs":1}"#);
    let id = task_id(&server, TOKEN, "Pay rent");
    assert!(ok(3, "due_tasks", r#"{"view":"overdue"}"#).contains("Pay rent"));
    assert!(ok(4, "list_lists", "{}").contains("Inbox"));
    ok(
        5,
        "update_task",
        &format!(r#"{{"id":"{id}","title":"Pay the rent"}}"#),
    );
    assert!(ok(6, "get_task", &format!(r#"{{"id":"{id}"}}"#)).contains("Pay the rent"));
    ok(7, "complete_task", &format!(r#"{{"id":"{id}"}}"#));
    let inbox = "00000000-0000-7000-8000-000000000001";
    assert!(ok(
        8,
        "list_tasks",
        &format!(r#"{{"listId":"{inbox}","status":"done"}}"#)
    )
    .contains("Pay the rent"));
    ok(9, "reopen_task", &format!(r#"{{"id":"{id}"}}"#));
    assert!(ok(10, "list_tasks", &format!(r#"{{"listId":"{inbox}"}}"#)).contains("Pay the rent"));
    ok(11, "delete_task", &format!(r#"{{"id":"{id}"}}"#));
    assert!(!ok(12, "list_tasks", &format!(r#"{{"listId":"{inbox}"}}"#)).contains("Pay the rent"));
}

#[test]
fn mcp_keeps_each_users_tasks_to_their_own_token() {
    use rusty_tick::backend::USERS_FILE;
    use rusty_tick::users::{Registry, UserKey};
    let dir = tempfile::tempdir().unwrap();
    let mut registry = Registry::default();
    let (alice, bob) = (
        UserKey::parse("alice").unwrap(),
        UserKey::parse("bob").unwrap(),
    );
    registry.add_user(&alice).unwrap();
    registry.add_user(&bob).unwrap();
    let alice_token = registry.add_token(&alice, "phone", 1).unwrap();
    let bob_token = registry.add_token(&bob, "laptop", 1).unwrap();
    registry.save(&dir.path().join(USERS_FILE)).unwrap();
    let backend = Backend::multi(
        dir.path(),
        std::num::NonZeroUsize::new(4).unwrap(),
        Box::new(system_clock),
    )
    .unwrap();
    let server = Running::serve(backend, dir, None);

    let alice_session = open_session(&server, &alice_token);
    call_as(
        &server,
        &alice_token,
        &alice_session,
        2,
        "add_task",
        r#"{"title":"Alice secret"}"#,
    );
    let id = task_id(&server, &alice_token, "Alice secret");

    let bob_session = open_session(&server, &bob_token);
    let found = call_as(
        &server,
        &bob_token,
        &bob_session,
        3,
        "search_tasks",
        r#"{"query":"secret"}"#,
    );
    assert!(!found.contains("Alice secret"), "{found}");
    let fetched = call_as(
        &server,
        &bob_token,
        &bob_session,
        4,
        "get_task",
        &format!(r#"{{"id":"{id}"}}"#),
    );
    assert!(
        fetched.contains(r#""isError":true"#) && !fetched.contains("Alice secret"),
        "{fetched}"
    );
    // A session id is no credential: Bob's token on Alice's session still reaches only Bob's store.
    let borrowed = call_as(
        &server,
        &bob_token,
        &alice_session,
        5,
        "get_task",
        &format!(r#"{{"id":"{id}"}}"#),
    );
    assert!(!borrowed.contains("Alice secret"), "{borrowed}");
}
