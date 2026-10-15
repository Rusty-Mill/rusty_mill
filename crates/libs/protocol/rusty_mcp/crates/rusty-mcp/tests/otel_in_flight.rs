//! The `mcp.server.requests.in_flight` up/down counter.
//!
//! `otel_metrics.rs` scans raw OTLP bytes, which cannot tell a gauge that
//! returns to zero from one that only ever climbs. This reads the value back
//! through the SDK's in-memory exporter instead.

#![cfg(feature = "otel")]

use opentelemetry::metrics::MeterProvider as _;
use opentelemetry_sdk::metrics::{
    InMemoryMetricExporter, PeriodicReader, SdkMeterProvider,
    data::{AggregatedMetrics, MetricData},
};
use rusty_mcp::otel::metrics::{Instruments, Outcome};

const IN_FLIGHT: &str = "mcp.server.requests.in_flight";

/// Sum of every `in_flight` data point, as of the latest export.
fn in_flight(provider: &SdkMeterProvider, exporter: &InMemoryMetricExporter) -> i64 {
    provider.force_flush().expect("flush");
    let exports = exporter.get_finished_metrics().expect("metrics");
    let latest = exports.last().expect("at least one export");

    latest
        .scope_metrics()
        .flat_map(|scope| scope.metrics())
        .filter(|metric| metric.name() == IN_FLIGHT)
        .map(|metric| match metric.data() {
            AggregatedMetrics::I64(MetricData::Sum(sum)) => {
                sum.data_points().map(|point| point.value()).sum::<i64>()
            }
            other => panic!("unexpected aggregation: {other:?}"),
        })
        .sum()
}

#[test]
fn in_flight_is_one_while_a_request_is_open_and_zero_after() {
    let exporter = InMemoryMetricExporter::default();
    let reader = PeriodicReader::builder(exporter.clone()).build();
    let provider = SdkMeterProvider::builder().with_reader(reader).build();
    let instruments = Instruments::new(&provider.meter("test"));

    instruments.request_started("tools/call");
    assert_eq!(in_flight(&provider, &exporter), 1);

    instruments.request_finished("tools/call", None, Outcome::Ok, 0.01);
    assert_eq!(in_flight(&provider, &exporter), 0);
}
