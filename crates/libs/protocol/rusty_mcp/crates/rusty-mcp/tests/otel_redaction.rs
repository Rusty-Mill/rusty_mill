//! A metrics endpoint can carry a credential, in its query, its path or its
//! userinfo. Nothing the exporter says about it, in a log line or an error, may
//! repeat it.
//!
//! The subscriber is installed process-wide because the push thread logs from a
//! thread of its own; every test uses secrets that are unique to it.

#![cfg(feature = "otel")]

use std::{
    io::{Read as _, Write},
    net::{SocketAddr, TcpListener},
    sync::{Arc, Mutex, OnceLock},
    thread,
    time::Duration,
};

use rusty_mcp::otel::{OtelConfig, OtelError, metrics::Outcome};
use tracing_subscriber::fmt::MakeWriter;

/// Everything logged so far, as text.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("lock").extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Captured {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

impl Captured {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("lock")).into_owned()
    }
}

fn logs() -> &'static Captured {
    static LOGS: OnceLock<Captured> = OnceLock::new();
    LOGS.get_or_init(|| {
        let captured = Captured::default();
        tracing_subscriber::fmt()
            .with_writer(captured.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::WARN)
            .init();
        captured
    })
}

/// A collector that answers every POST with `status`.
fn collector(status: u16) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            thread::spawn(move || {
                let mut raw = Vec::new();
                let mut buf = [0u8; 8192];
                while let Ok(n) = stream.read(&mut buf) {
                    raw.extend_from_slice(&buf[..n]);
                    if n == 0 || complete(&raw) {
                        break;
                    }
                }
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
            });
        }
    });
    addr
}

fn complete(raw: &[u8]) -> bool {
    let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") else {
        return false;
    };
    let head = String::from_utf8_lossy(&raw[..end]).to_ascii_lowercase();
    let length: usize = head
        .lines()
        .find_map(|l| l.strip_prefix("content-length:"))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);
    raw.len() >= end + 4 + length
}

/// An address nothing is listening on.
fn dead_address() -> SocketAddr {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr")
}

/// Push once to `endpoint`, which fails, and return what was logged.
async fn push_that_fails(endpoint: String) -> String {
    let logs = logs();
    let (guard, _tracer) = rusty_mcp::otel::pipeline(
        OtelConfig::new("test-server")
            .with_endpoint("http://127.0.0.1:1")
            .with_metrics_endpoint(endpoint)
            .with_metrics_interval(Duration::from_secs(3600)),
    )
    .expect("pipeline");
    let instruments = guard.instruments().expect("metrics on");
    instruments.request_started("ping");
    instruments.request_finished("ping", None, Outcome::Ok, 0.001);
    guard.flush();
    guard.shutdown();
    logs.text()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rejected_push_does_not_log_the_query_or_the_path() {
    let addr = collector(500);
    let said = push_that_fails(format!(
        "http://{addr}/v1/PATHSECRET1/metrics?token=QUERYSECRET1"
    ))
    .await;

    assert!(
        said.contains("failed to push metrics"),
        "nothing logged: {said}"
    );
    assert!(
        said.contains(&format!("http://{addr}")),
        "no origin: {said}"
    );
    for secret in ["QUERYSECRET1", "PATHSECRET1"] {
        assert!(!said.contains(secret), "{secret} leaked: {said}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unreachable_collector_does_not_log_the_query_or_the_path() {
    let addr = dead_address();
    let said = push_that_fails(format!(
        "http://{addr}/v1/PATHSECRET2/metrics?token=QUERYSECRET2"
    ))
    .await;

    assert!(
        said.contains("failed to push metrics"),
        "nothing logged: {said}"
    );
    for secret in ["QUERYSECRET2", "PATHSECRET2"] {
        assert!(!said.contains(secret), "{secret} leaked: {said}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_metrics_endpoint_with_userinfo_is_refused_without_echoing_it() {
    for endpoint in [
        "http://user:USERSECRET@127.0.0.1:1/v1/metrics",
        "https://USERSECRET@127.0.0.1:1/v1/metrics?token=USERSECRET",
        "USERSECRET",
    ] {
        let result = rusty_mcp::otel::pipeline(
            OtelConfig::new("test-server")
                .with_endpoint("http://127.0.0.1:1")
                .with_metrics_endpoint(endpoint),
        );
        let Err(err) = result else {
            panic!("{endpoint} should be refused");
        };
        assert!(matches!(err, OtelError::MetricsEndpoint), "{err:?}");
        for text in [err.to_string(), format!("{err:?}")] {
            assert!(!text.contains("USERSECRET"), "leaked: {text}");
        }
    }
    assert!(!logs().text().contains("USERSECRET"));
}

#[test]
fn logging_a_config_does_not_print_an_endpoint_credential() {
    let config = OtelConfig::new("test-server")
        .with_endpoint("http://collector:4317/?key=SPANSECRET")
        .with_metrics_endpoint("https://collector:4318/v1/metrics?token=CONFIGSECRET");
    let shown = format!("{config:?}");
    assert!(
        !shown.contains("SPANSECRET") && !shown.contains("CONFIGSECRET"),
        "{shown}"
    );
    assert!(shown.contains("https://collector:4318"), "{shown}");
}
