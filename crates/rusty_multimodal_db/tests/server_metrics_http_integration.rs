//! Real HTTP and binary-wire traffic against one server (`MHTTP-FR-001`
//! through `006`, ADR-0069). Mirrors `server_metrics_integration`'s
//! loopback-listener and fixture conventions; no process environment is
//! mutated to configure an in-process server.

use rusty_multimodal_db::record::DogRecord;
use rusty_multimodal_db::server::client::SchemaDrivenClient;
use rusty_multimodal_db::server::dog::DogConnectionStore;
use rusty_multimodal_db::server::{serve, ServeOptions};
use rusty_multimodal_db::ProductionStore;
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use uuid::Uuid;

fn unique_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("{label}_{}_{n}", std::process::id()))
}

fn start_server(options: ServeOptions) -> SocketAddr {
    let dir = unique_dir("metrics_http_integration");
    std::fs::create_dir_all(&dir).unwrap();
    let store = ProductionStore::create(
        vec![DogRecord::new(Uuid::from_u128(1), "labrador", 3)],
        vec![],
        &dir.join("dogs.mmap"),
    )
    .unwrap();
    let connection_store = Arc::new(DogConnectionStore::new(store));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || serve(listener, connection_store, options));
    addr
}

fn start_http_server() -> (SocketAddr, SocketAddr) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let http_addr = listener.local_addr().unwrap();
    let wire_addr = start_server(ServeOptions::default().with_metrics_http(listener));
    (wire_addr, http_addr)
}

fn exchange(addr: SocketAddr, request: &[u8]) -> String {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream.write_all(request).unwrap();
    stream.shutdown(Shutdown::Write).unwrap();
    let mut response = String::new();
    if let Err(error) = stream.read_to_string(&mut response) {
        assert_eq!(error.kind(), std::io::ErrorKind::ConnectionReset);
    }
    response
}

fn scrape(addr: SocketAddr) -> String {
    let response = exchange(addr, b"GET /metrics HTTP/1.1\r\nHost: x\r\n\r\n");
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    assert!(head.starts_with("HTTP/1.1 200 OK\r\n"), "{head}");
    assert!(head
        .lines()
        .any(|line| line == "Content-Type: text/plain; version=0.0.4"));
    assert!(head.lines().any(|line| line == "Connection: close"));
    assert!(head
        .lines()
        .any(|line| line == format!("Content-Length: {}", body.len())));
    body.to_string()
}

fn metric_value(text: &str, name: &str) -> u64 {
    text.lines()
        .find_map(|line| line.strip_prefix(&format!("{name} ")))
        .unwrap_or_else(|| panic!("{name} not found in:\n{text}"))
        .parse()
        .unwrap()
}

/// `MHTTP-FR-001`: the unset-variable path passes unchanged options to
/// `serve`, just as before. No HTTP listener is bound or supplied here;
/// the existing wire handshake, reads, and Metrics still work.
#[test]
fn default_options_keep_existing_wire_behavior() {
    let addr = start_server(ServeOptions::default());
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    assert!(client.get(Uuid::from_u128(1)).unwrap().is_some());
    assert!(client.get(Uuid::from_u128(999)).unwrap().is_none());
    let metrics = client.metrics().unwrap();
    assert!(metric_value(&metrics, "dogserver_requests_total") >= 2);
    assert_eq!(metric_value(&metrics, "dogserver_connections_total"), 1);
}

/// `MHTTP-FR-002`: nonzero wire traffic is visible on the second port.
/// Repeated scrapes leave every request/connection counter unchanged;
/// the immediately following wire Metrics renders before counting itself.
#[test]
fn http_reports_live_wire_counters_without_counting_scrapes() {
    let (wire_addr, http_addr) = start_http_server();
    let mut client = SchemaDrivenClient::connect(wire_addr).unwrap();
    for _ in 0..3 {
        assert!(client.get(Uuid::from_u128(1)).unwrap().is_some());
    }
    let first = scrape(http_addr);
    let second = scrape(http_addr);
    let wire = client.metrics().unwrap();
    for name in [
        "dogserver_requests_total",
        "dogserver_requests_ok_total",
        "dogserver_requests_err_total",
        "dogserver_connections_total",
        "dogserver_connections_active",
        "dogserver_uptime_seconds",
    ] {
        assert!(first.contains(&format!("# HELP {name} ")));
        assert!(first.contains(&format!("# TYPE {name} ")));
        let value = metric_value(&first, name);
        if name != "dogserver_uptime_seconds" {
            assert_eq!(value, metric_value(&second, name), "{name}");
            assert_eq!(value, metric_value(&wire, name), "{name}");
        }
    }
    assert!(metric_value(&first, "dogserver_requests_total") >= 3);
    assert!(metric_value(&first, "dogserver_requests_ok_total") >= 3);
    assert_eq!(metric_value(&first, "dogserver_connections_total"), 1);
    assert_eq!(metric_value(&first, "dogserver_connections_active"), 1);
}

/// `MHTTP-FR-002`: other paths and methods close with an empty 404.
#[test]
fn other_paths_and_methods_return_not_found() {
    let (_, http_addr) = start_http_server();
    for request in ["GET / HTTP/1.1\r\n\r\n", "POST /metrics HTTP/1.1\r\n\r\n"] {
        let response = exchange(http_addr, request.as_bytes());
        let (head, body) = response.split_once("\r\n\r\n").unwrap();
        assert!(head.starts_with("HTTP/1.1 404 Not Found\r\n"));
        assert!(head.lines().any(|line| line == "Connection: close"));
        assert!(head.lines().any(|line| line == "Content-Length: 0"));
        assert!(body.is_empty());
    }
}

/// `MHTTP-FR-004`: bad clients receive nothing; the accept loop and wire
/// listener still answer subsequent requests after every failure case.
#[test]
fn bad_heads_leave_both_listeners_serving() {
    let (wire_addr, http_addr) = start_http_server();
    for request in [
        b"garbage\r\n\r\n".to_vec(),
        b"GET /metrics HTTP/1.1\r\nHost:".to_vec(),
        vec![b'x'; rusty_http::head::DEFAULT_MAX_HEAD_LEN + 1],
    ] {
        assert!(exchange(http_addr, &request).is_empty());
        let metrics = scrape(http_addr);
        assert_eq!(metric_value(&metrics, "dogserver_requests_total"), 0);
        assert_eq!(metric_value(&metrics, "dogserver_connections_total"), 0);
        assert_eq!(metric_value(&metrics, "dogserver_connections_active"), 0);
    }
    let mut client = SchemaDrivenClient::connect(wire_addr).unwrap();
    assert!(client.get(Uuid::from_u128(1)).unwrap().is_some());
}

/// `MHTTP-FR-006`: each shipped binary treats an invalid configured
/// address as a fatal startup error naming the variable. Child-only env
/// changes avoid races with the other tests' process environment.
#[test]
fn invalid_http_bind_is_fatal_in_every_binary() {
    for binary in [
        env!("CARGO_BIN_EXE_dog_server"),
        env!("CARGO_BIN_EXE_memory_server"),
        env!("CARGO_BIN_EXE_reminder_server"),
        env!("CARGO_BIN_EXE_entity_server"),
    ] {
        let mut command = std::process::Command::new(binary);
        for (name, _) in std::env::vars_os() {
            if name.to_string_lossy().starts_with("SERVER_") {
                command.env_remove(name);
            }
        }
        let output = command
            .arg("127.0.0.1:0")
            .env("SERVER_METRICS_HTTP_ADDR", "invalid socket address")
            .output()
            .unwrap();
        assert!(!output.status.success(), "{binary}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("SERVER_METRICS_HTTP_ADDR"),
            "{binary}: {output:?}"
        );
    }
}
