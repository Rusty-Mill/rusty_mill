//! End to end over a real socket: framing, keep-alive, limits, shutdown.

use rusty_tick::api::Api;
use rusty_tick::server::{Server, ShutdownHandle, MAX_BODY_BYTES};
use rusty_tick::service::{system_clock, Service};
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
        let service = Service::open(dir.path(), system_clock()).unwrap();
        let mut server = Server::bind(
            "127.0.0.1:0".parse().unwrap(),
            Api::new(TOKEN.into()).unwrap(),
            service,
        )
        .unwrap();
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
