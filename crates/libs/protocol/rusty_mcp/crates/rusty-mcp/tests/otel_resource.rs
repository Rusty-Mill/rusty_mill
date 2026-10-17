//! Metrics carry the same resource identity as spans: the environment's
//! `OTEL_RESOURCE_ATTRIBUTES` (instance id, namespace, deployment environment),
//! with the config's `service.name` and `service.version` over it.
//!
//! The process environment cannot be changed from a test (`unsafe_code` is
//! forbidden and `set_var` is unsafe), so the test runs itself again as a child
//! with the variable set, and the child does the asserting.

#![cfg(feature = "otel")]

use std::{
    io::{Read as _, Write as _},
    net::TcpListener,
    process::Command,
    sync::mpsc,
    thread,
    time::Duration,
};

use rusty_json::Value;
use rusty_mcp::otel::{OtelConfig, metrics::Outcome};

const CHILD_MARKER: &str = "RUSTY_MCP_RESOURCE_CHILD";

#[test]
fn metrics_carry_the_environment_resource_on_the_wire() {
    let exe = std::env::current_exe().expect("test binary");
    let output = Command::new(exe)
        .args(["--exact", "child_pushes_and_checks", "--nocapture"])
        .env(CHILD_MARKER, "1")
        .env(
            "OTEL_RESOURCE_ATTRIBUTES",
            "service.instance.id=inst-7, service.namespace=ns, deployment.environment=staging, service.name=from-env",
        )
        .output()
        .expect("run the child");
    assert!(
        output.status.success(),
        "child failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Only does anything when run by the test above.
#[tokio::test(flavor = "multi_thread")]
async fn child_pushes_and_checks() {
    if std::env::var_os(CHILD_MARKER).is_none() {
        return;
    }

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (bodies, received) = mpsc::channel();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut raw = Vec::new();
            let mut buf = [0u8; 8192];
            while let Ok(n) = stream.read(&mut buf) {
                raw.extend_from_slice(&buf[..n]);
                if n == 0 || body_is_complete(&raw) {
                    break;
                }
            }
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            let _ = bodies.send(raw);
        }
    });

    let (guard, _tracer) = rusty_mcp::otel::pipeline(
        OtelConfig::new("configured-name")
            .with_service_version("9.9.9")
            .with_endpoint("http://127.0.0.1:1")
            .with_metrics_endpoint(format!("http://{addr}/v1/metrics"))
            .with_metrics_interval(Duration::from_secs(3600)),
    )
    .expect("pipeline");
    let instruments = guard.instruments().expect("metrics on");
    instruments.request_started("ping");
    instruments.request_finished("ping", None, Outcome::Ok, 0.001);
    guard.shutdown();

    let raw = received
        .recv_timeout(Duration::from_secs(5))
        .expect("a push");
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n").expect("head");
    let body = Value::parse(std::str::from_utf8(&raw[end + 4..]).expect("utf8")).expect("json");
    let attributes = body["resourceMetrics"][0]["resource"]["attributes"]
        .as_array()
        .expect("attributes");
    let get = |key: &str| {
        attributes
            .iter()
            .find(|a| a["key"].as_str() == Some(key))
            .and_then(|a| a["value"]["stringValue"].as_str().map(str::to_owned))
    };

    assert_eq!(get("service.instance.id").as_deref(), Some("inst-7"));
    assert_eq!(get("service.namespace").as_deref(), Some("ns"));
    assert_eq!(get("deployment.environment").as_deref(), Some("staging"));
    // The config wins over the environment, as it does for spans.
    assert_eq!(get("service.name").as_deref(), Some("configured-name"));
    assert_eq!(get("service.version").as_deref(), Some("9.9.9"));
    let names: Vec<_> = attributes
        .iter()
        .filter_map(|a| a["key"].as_str())
        .collect();
    assert_eq!(
        names.iter().filter(|n| **n == "service.name").count(),
        1,
        "service.name twice: {names:?}"
    );
}

fn body_is_complete(raw: &[u8]) -> bool {
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
