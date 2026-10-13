//! The HTTP surface over a real socket: guard, API status mapping, UI, MCP.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;

use rusty_mcp_server::{HttpConfig, HttpHandler};
use rusty_mill_board::api::Shared;
use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::store::MemStore;
use rusty_mill_board::{mcp, web::Web};

fn start(token: Option<&str>) -> SocketAddr {
    let board: Arc<Shared> = Arc::new(Board::new(Box::new(MemStore::default()), Roles::default()));
    let config = HttpConfig {
        allowed_hosts: vec!["127.0.0.1".into()],
        ..HttpConfig::default()
    };
    let path = config.path.clone();
    let mcp = HttpHandler::new(mcp::server(board.clone()).expect("server"), config);
    let web = Web::new(
        board,
        mcp,
        path,
        vec!["127.0.0.1".into()],
        token.map(str::to_owned),
    );
    let server =
        rusty_serve::Server::bind_shared("127.0.0.1:0".parse().expect("addr"), web).expect("bind");
    let addr = server.local_addr().expect("addr");
    std::thread::spawn(move || {
        let _ = server.run();
    });
    addr
}

/// (status, body) of one request.
fn send(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> (u16, String) {
    let mut raw = format!(
        "{method} {path} HTTP/1.1\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("host")) {
        raw.push_str(&format!("Host: {addr}\r\n"));
    }
    for (k, v) in headers {
        raw.push_str(&format!("{k}: {v}\r\n"));
    }
    raw.push_str("\r\n");
    raw.push_str(body);
    let mut sock = TcpStream::connect(addr).expect("connect");
    sock.write_all(raw.as_bytes()).expect("write");
    let mut out = String::new();
    sock.read_to_string(&mut out).expect("read");
    let status = out
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .expect("status");
    (
        status,
        out.split("\r\n\r\n").nth(1).unwrap_or("").to_owned(),
    )
}

const JSON: (&str, &str) = ("Content-Type", "application/json");

#[test]
fn guard_and_api() {
    let addr = start(Some("tok"));
    let auth = ("Authorization", "Bearer tok");
    assert_eq!(send(addr, "POST", "/api/board_list", &[JSON], "{}").0, 401);
    assert_eq!(
        send(
            addr,
            "POST",
            "/api/board_list",
            &[JSON, auth, ("Host", "evil.com")],
            "{}"
        )
        .0,
        403
    );
    assert_eq!(
        send(addr, "POST", "/api/board_list", &[auth], "{}").0,
        400,
        "non-JSON write refused"
    );
    assert_eq!(send(addr, "POST", "/api/nope", &[JSON, auth], "{}").0, 404);
    assert_eq!(
        send(
            addr,
            "POST",
            "/api/card_add",
            &[JSON, auth],
            r#"{"title":1}"#
        )
        .0,
        400
    );

    let (status, body) = send(
        addr,
        "POST",
        "/api/card_add",
        &[JSON, auth],
        r#"{"actor":"a","role":"human","title":"t"}"#,
    );
    assert_eq!(status, 200);
    let id = serde_json::from_str::<serde_json::Value>(&body).expect("json")["card"]["id"]
        .as_str()
        .expect("id")
        .to_owned();

    let mv = |role: &str, to: &str| {
        let args = format!(r#"{{"actor":"a","role":"{role}","id":"{id}","to":"{to}"}}"#);
        send(addr, "POST", "/api/card_move", &[JSON, auth], &args).0
    };
    assert_eq!(mv("builder", "ready"), 403, "builder cannot triage");
    assert_eq!(mv("human", "done"), 422, "no such move");
    assert_eq!(mv("human", "ready"), 200);
}

#[test]
fn ui_is_served_without_token_but_requires_allowed_host() {
    let addr = start(Some("tok"));
    let (status, body) = send(addr, "GET", "/", &[], "");
    assert_eq!(status, 200);
    assert!(body.contains("Mill Board"));
    assert_eq!(send(addr, "GET", "/", &[("Host", "evil.com")], "").0, 403);
}

#[test]
fn mcp_requires_token_and_lists_tools() {
    let addr = start(Some("tok"));
    let call = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
    let accept = ("Accept", "application/json, text/event-stream");
    assert_eq!(send(addr, "POST", "/mcp", &[JSON, accept], call).0, 401);
    let (status, body) = send(
        addr,
        "POST",
        "/mcp",
        &[JSON, accept, ("Authorization", "Bearer tok")],
        call,
    );
    assert_eq!(status, 200);
    assert!(body.contains("card_move") && body.contains("board_events"));
}
