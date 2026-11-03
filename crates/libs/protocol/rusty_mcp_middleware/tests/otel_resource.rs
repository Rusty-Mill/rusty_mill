//! Spans and metrics carry the same resource identity: the environment's
//! `OTEL_RESOURCE_ATTRIBUTES` (instance id, namespace, deployment environment),
//! with the config's `service.name` and `service.version` over it.
//!
//! The process environment cannot be changed from a test (`unsafe_code` is
//! forbidden and `set_var` is unsafe), so the test runs itself again as a child
//! with the variable set, and the child does the asserting.

#![cfg(feature = "otel")]

mod otel_support;

use std::{process::Command, time::Duration};

use otel_support::Collector;
use rusty_json::Value;
use rusty_log::tracing_subscriber::prelude::*;
use rusty_mcp_middleware::otel::{OtelConfig, metrics::Outcome};

const CHILD_MARKER: &str = "RUSTY_MCP_RESOURCE_CHILD";

#[test]
fn spans_and_metrics_carry_the_environment_resource_on_the_wire() {
    let exe = std::env::current_exe().expect("test binary");
    let output = Command::new(exe)
        .args(["--exact", "child_exports_and_checks", "--nocapture"])
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
#[test]
fn child_exports_and_checks() {
    if std::env::var_os(CHILD_MARKER).is_none() {
        return;
    }

    let collector = Collector::start(200);
    let (guard, layer) = rusty_mcp_middleware::otel::pipeline(
        OtelConfig::new("configured-name")
            .with_service_version("9.9.9")
            .with_endpoint(collector.url())
            .with_metrics_interval(Duration::from_secs(3600)),
    )
    .expect("pipeline");
    rusty_log::subscriber::with_default(
        rusty_log::tracing_subscriber::registry().with(layer),
        || {
            drop(rusty_log::info_span!("work"));
        },
    );
    let instruments = guard.instruments().expect("metrics on");
    instruments.request_started("ping");
    instruments.request_finished("ping", None, Outcome::Ok, 0.001);
    guard.shutdown();

    let pushes = collector.pushes();
    for (path, root) in [
        ("/v1/traces", "resourceSpans"),
        ("/v1/metrics", "resourceMetrics"),
    ] {
        let push = pushes
            .iter()
            .find(|push| push.path == path)
            .unwrap_or_else(|| panic!("nothing was sent to {path}: {pushes:?}"));
        check_resource(path, &push.body[root][0]["resource"]["attributes"]);
    }
}

fn check_resource(signal: &str, attributes: &Value) {
    let attributes = attributes.as_array().expect("attributes");
    let get = |key: &str| {
        attributes
            .iter()
            .find(|a| a["key"].as_str() == Some(key))
            .and_then(|a| a["value"]["stringValue"].as_str().map(str::to_owned))
    };

    assert_eq!(
        get("service.instance.id").as_deref(),
        Some("inst-7"),
        "{signal}"
    );
    assert_eq!(get("service.namespace").as_deref(), Some("ns"), "{signal}");
    assert_eq!(
        get("deployment.environment").as_deref(),
        Some("staging"),
        "{signal}"
    );
    // The config wins over the environment.
    assert_eq!(
        get("service.name").as_deref(),
        Some("configured-name"),
        "{signal}"
    );
    assert_eq!(get("service.version").as_deref(), Some("9.9.9"), "{signal}");
    let names: Vec<_> = attributes
        .iter()
        .filter_map(|a| a["key"].as_str())
        .collect();
    assert_eq!(
        names.iter().filter(|n| **n == "service.name").count(),
        1,
        "service.name twice in {signal}: {names:?}"
    );
}
