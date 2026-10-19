//! A telemetry endpoint can carry a credential, in its query, its path or its
//! userinfo. Nothing the exporter says about it, in a log line or an error, may
//! repeat it.
//!
//! The subscriber is installed process-wide because the export threads log from
//! threads of their own; every test uses secrets that are unique to it.

#![cfg(feature = "otel")]

mod otel_support;

use std::{
    io::Write,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

use otel_support::{Collector, dead_address};
use rusty_log::tracing_subscriber::{fmt::MakeWriter, prelude::*};
use rusty_mcp::otel::{OtelConfig, OtelError, Signal, metrics::Outcome};

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
        rusty_log::tracing_subscriber::fmt()
            .with_writer(captured.clone())
            .with_ansi(false)
            .with_max_level(rusty_log::Level::WARN)
            .init();
        captured
    })
}

/// Send one span batch to `base` and one metric push to `metrics_endpoint`,
/// both of which fail, and return everything logged.
fn send_that_fails(base: String, metrics_endpoint: String) -> String {
    let logs = logs();
    let (guard, layer) = rusty_mcp::otel::pipeline(
        OtelConfig::new("test-server")
            .with_endpoint(base)
            .with_metrics_endpoint(metrics_endpoint)
            .with_metrics_interval(Duration::from_secs(3600)),
    )
    .expect("pipeline");
    // Scoped to this thread, so the global capture subscriber is not replaced.
    rusty_log::subscriber::with_default(
        rusty_log::tracing_subscriber::registry().with(layer),
        || {
            drop(rusty_log::info_span!("work"));
        },
    );
    let instruments = guard.instruments().expect("metrics on");
    instruments.request_started("ping");
    instruments.request_finished("ping", None, Outcome::Ok, 0.001);
    guard.flush();
    guard.shutdown();
    logs.text()
}

/// How many failures `said` reports against `addr`. The log is shared by every
/// test in the process, so a test counts only the lines about its own collector.
fn failures_for(said: &str, addr: std::net::SocketAddr) -> usize {
    said.lines()
        .filter(|line| {
            line.contains("failed to send telemetry") && line.contains(&addr.to_string())
        })
        .count()
}

#[test]
fn a_rejected_send_does_not_log_the_query_or_the_path() {
    let addr = Collector::start(500).addr;
    let said = send_that_fails(
        format!("http://{addr}/PATHSECRET1?token=QUERYSECRET1"),
        format!("http://{addr}/v1/PATHSECRET1/metrics?token=QUERYSECRET1"),
    );

    // Both signals failed, and said so without the secrets.
    // The span batch once, and the metrics at the flush and again at shutdown.
    assert_eq!(failures_for(&said, addr), 3, "{said}");
    assert!(
        said.contains(&format!("http://{addr}")),
        "no origin: {said}"
    );
    for secret in ["QUERYSECRET1", "PATHSECRET1"] {
        assert!(!said.contains(secret), "{secret} leaked: {said}");
    }
}

#[test]
fn an_unreachable_collector_does_not_log_the_query_or_the_path() {
    let addr = dead_address();
    let said = send_that_fails(
        format!("http://{addr}/PATHSECRET2?token=QUERYSECRET2"),
        format!("http://{addr}/v1/PATHSECRET2/metrics?token=QUERYSECRET2"),
    );

    // The span batch once, and the metrics at the flush and again at shutdown.
    assert_eq!(failures_for(&said, addr), 3, "{said}");
    for secret in ["QUERYSECRET2", "PATHSECRET2"] {
        assert!(!said.contains(secret), "{secret} leaked: {said}");
    }
}

#[test]
fn an_endpoint_with_userinfo_is_refused_without_echoing_it() {
    let bad = [
        "http://user:USERSECRET@127.0.0.1:1/v1/metrics",
        "https://USERSECRET@127.0.0.1:1/v1/metrics?token=USERSECRET",
        "USERSECRET",
    ];
    for endpoint in bad {
        // As the metrics URL, and as the base both signals are derived from.
        let cases = [
            (
                Signal::Metrics,
                OtelConfig::new("s").with_metrics_endpoint(endpoint),
            ),
            (Signal::Traces, OtelConfig::new("s").with_endpoint(endpoint)),
        ];
        for (signal, config) in cases {
            let Err(err) = rusty_mcp::otel::pipeline(config) else {
                panic!("{endpoint} should be refused");
            };
            assert!(
                matches!(err, OtelError::Endpoint(s) if s == signal),
                "{err:?}"
            );
            for text in [err.to_string(), format!("{err:?}")] {
                assert!(!text.contains("USERSECRET"), "leaked: {text}");
            }
        }
    }
    assert!(!logs().text().contains("USERSECRET"));
}

#[test]
fn logging_a_config_does_not_print_an_endpoint_credential() {
    let config = OtelConfig::new("test-server")
        .with_endpoint("http://collector:4318/?key=BASESECRET")
        .with_metrics_endpoint("https://collector:4318/v1/metrics?token=CONFIGSECRET");
    let shown = format!("{config:?}");
    assert!(
        !shown.contains("BASESECRET") && !shown.contains("CONFIGSECRET"),
        "{shown}"
    );
    assert!(shown.contains("https://collector:4318"), "{shown}");
}
