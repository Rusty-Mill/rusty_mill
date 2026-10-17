//! The OTLP/HTTP JSON encoding of metrics.
//!
//! Built from the [`rusty_json::Value`] tree, following the OTLP spec's JSON
//! mapping of the protobuf messages rather than any SDK's types:
//!
//! - 64-bit integers (`asInt`, `count`, `bucketCounts`, the `*UnixNano` times)
//!   are decimal **strings**, since JSON numbers cannot hold them exactly;
//! - enums are their integer values (`aggregationTemporality` is `2`, for
//!   cumulative);
//! - field names are lowerCamelCase.
//!
//! This is the body of a `POST /v1/metrics` with `Content-Type:
//! application/json`.

use rusty_json::{Value, json};

use super::instrument::{Data, HistogramPoint, Int, Metric};
use super::labels::Label;

/// `AGGREGATION_TEMPORALITY_CUMULATIVE`.
const CUMULATIVE: u32 = 2;

/// An `ExportMetricsServiceRequest` carrying `metrics` as of `now_nanos`.
///
/// `resource` describes the process, `scope` names the instrumentation, and
/// `start_nanos` is when the cumulative counts began.
pub(crate) fn metrics_request(
    resource: &[(String, String)],
    scope: &str,
    start_nanos: u64,
    now_nanos: u64,
    metrics: &[Metric],
) -> Value {
    let metrics: Vec<Value> = metrics
        .iter()
        .map(|metric| encode_metric(metric, start_nanos, now_nanos))
        .collect();
    json!({
        "resourceMetrics": [{
            "resource": { "attributes": attributes(resource.iter().map(|(k, v)| (k.as_str(), v.as_str()))) },
            "scopeMetrics": [{
                "scope": { "name": scope },
                "metrics": metrics,
            }],
        }],
    })
}

fn encode_metric(metric: &Metric, start_nanos: u64, now_nanos: u64) -> Value {
    let mut encoded = json!({
        "name": metric.name,
        "description": metric.description,
        "unit": metric.unit,
    });
    let (key, body) = match &metric.data {
        Data::Sum { monotonic, points } => {
            let points: Vec<Value> = points
                .iter()
                .map(|(attrs, value)| number_point(attrs, *value, start_nanos, now_nanos))
                .collect();
            (
                "sum",
                json!({
                    "dataPoints": points,
                    "aggregationTemporality": CUMULATIVE,
                    "isMonotonic": *monotonic,
                }),
            )
        }
        Data::Histogram { bounds, points } => {
            let points: Vec<Value> = points
                .iter()
                .map(|point| histogram_point(point, bounds, start_nanos, now_nanos))
                .collect();
            (
                "histogram",
                json!({
                    "dataPoints": points,
                    "aggregationTemporality": CUMULATIVE,
                }),
            )
        }
    };
    if let Some(object) = encoded.as_object_mut() {
        object.insert(key.to_owned(), body);
    }
    encoded
}

fn number_point(attrs: &[Label], value: Int, start_nanos: u64, now_nanos: u64) -> Value {
    json!({
        "attributes": labels(attrs),
        "startTimeUnixNano": start_nanos.to_string(),
        "timeUnixNano": now_nanos.to_string(),
        "asInt": value.to_string(),
    })
}

fn histogram_point(
    point: &HistogramPoint,
    bounds: &[f64],
    start_nanos: u64,
    now_nanos: u64,
) -> Value {
    let buckets: Vec<Value> = point
        .buckets
        .iter()
        .map(|count| Value::from(count.to_string()))
        .collect();
    let bounds: Vec<Value> = bounds.iter().map(|bound| Value::from(*bound)).collect();
    json!({
        "attributes": labels(&point.attributes),
        "startTimeUnixNano": start_nanos.to_string(),
        "timeUnixNano": now_nanos.to_string(),
        "count": point.count.to_string(),
        // JSON has no infinity, and a sum that overflowed is still "very large".
        "sum": point.sum.clamp(f64::MIN, f64::MAX),
        "bucketCounts": buckets,
        "explicitBounds": bounds,
    })
}

fn labels(labels: &[Label]) -> Value {
    attributes(labels.iter().map(|(key, value)| (*key, value.as_str())))
}

/// A list of OTLP `KeyValue`s with string values.
fn attributes<'a>(pairs: impl Iterator<Item = (&'a str, &'a str)>) -> Value {
    let list: Vec<Value> = pairs
        .map(|(key, value)| json!({ "key": key, "value": { "stringValue": value } }))
        .collect();
    Value::from(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resource() -> Vec<(String, String)> {
        vec![("service.name".to_owned(), "svc".to_owned())]
    }

    fn label(key: &'static str, value: &str) -> Label {
        (key, value.to_owned())
    }

    fn encode(metrics: &[Metric]) -> Value {
        metrics_request(&resource(), "rusty-mcp", 1_000, 2_000, metrics)
    }

    fn metrics_of(request: &Value) -> &Vec<Value> {
        request
            .get("resourceMetrics")
            .and_then(|r| r.get_index(0))
            .and_then(|r| r.get("scopeMetrics"))
            .and_then(|s| s.get_index(0))
            .and_then(|s| s.get("metrics"))
            .and_then(Value::as_array)
            .expect("resourceMetrics[0].scopeMetrics[0].metrics")
    }

    fn counter() -> Metric {
        Metric {
            name: "mcp.server.requests",
            description: "Requests.",
            unit: "{request}",
            data: Data::Sum {
                monotonic: true,
                points: vec![(vec![label("mcp.method", "ping")], 7)],
            },
        }
    }

    fn histogram() -> Metric {
        Metric {
            name: "mcp.server.request.duration",
            description: "Duration.",
            unit: "s",
            data: Data::Histogram {
                bounds: vec![0.5, 1.0],
                points: vec![HistogramPoint {
                    attributes: vec![label("mcp.method", "ping")],
                    count: 3,
                    sum: 1.75,
                    buckets: vec![1, 1, 1],
                }],
            },
        }
    }

    #[test]
    fn the_envelope_names_the_resource_and_the_scope() {
        let request = encode(&[]);
        let resource = &request["resourceMetrics"][0]["resource"]["attributes"][0];
        assert_eq!(resource["key"].as_str(), Some("service.name"));
        assert_eq!(resource["value"]["stringValue"].as_str(), Some("svc"));
        let scope = &request["resourceMetrics"][0]["scopeMetrics"][0]["scope"];
        assert_eq!(scope["name"].as_str(), Some("rusty-mcp"));
    }

    #[test]
    fn a_counter_is_a_cumulative_monotonic_sum_with_string_integers() {
        let request = encode(&[counter()]);
        let metric = &metrics_of(&request)[0];
        assert_eq!(metric["name"].as_str(), Some("mcp.server.requests"));
        assert_eq!(metric["unit"].as_str(), Some("{request}"));

        let sum = &metric["sum"];
        assert_eq!(sum["isMonotonic"].as_bool(), Some(true));
        assert_eq!(sum["aggregationTemporality"].as_u64(), Some(2));

        let point = &sum["dataPoints"][0];
        // 64-bit integers travel as strings.
        assert_eq!(point["asInt"].as_str(), Some("7"));
        assert_eq!(point["startTimeUnixNano"].as_str(), Some("1000"));
        assert_eq!(point["timeUnixNano"].as_str(), Some("2000"));
        assert_eq!(point["attributes"][0]["key"].as_str(), Some("mcp.method"));
        assert_eq!(
            point["attributes"][0]["value"]["stringValue"].as_str(),
            Some("ping")
        );
    }

    #[test]
    fn an_up_down_counter_is_not_monotonic() {
        let metric = Metric {
            data: Data::Sum {
                monotonic: false,
                points: vec![(vec![], -2)],
            },
            ..counter()
        };
        let request = encode(&[metric]);
        let sum = &metrics_of(&request)[0]["sum"];
        assert_eq!(sum["isMonotonic"].as_bool(), Some(false));
        assert_eq!(sum["dataPoints"][0]["asInt"].as_str(), Some("-2"));
    }

    #[test]
    fn a_histogram_carries_bounds_as_numbers_and_counts_as_strings() {
        let request = encode(&[histogram()]);
        let histogram = &metrics_of(&request)[0]["histogram"];
        assert_eq!(histogram["aggregationTemporality"].as_u64(), Some(2));

        let point = &histogram["dataPoints"][0];
        assert_eq!(point["count"].as_str(), Some("3"));
        assert_eq!(point["sum"].as_f64(), Some(1.75));
        let counts: Vec<_> = point["bucketCounts"]
            .as_array()
            .expect("bucketCounts")
            .iter()
            .map(|c| c.as_str().expect("string"))
            .collect();
        assert_eq!(counts, ["1", "1", "1"]);
        let bounds: Vec<_> = point["explicitBounds"]
            .as_array()
            .expect("explicitBounds")
            .iter()
            .map(|b| b.as_f64().expect("number"))
            .collect();
        assert_eq!(bounds, [0.5, 1.0]);
    }

    #[test]
    fn one_more_bucket_count_than_bounds() {
        // The OTLP invariant a collector validates.
        let request = encode(&[histogram()]);
        let point = &metrics_of(&request)[0]["histogram"]["dataPoints"][0];
        assert_eq!(
            point["bucketCounts"].as_array().expect("counts").len(),
            point["explicitBounds"].as_array().expect("bounds").len() + 1
        );
    }

    #[test]
    fn an_overflowed_sum_is_clamped_rather_than_becoming_null() {
        let mut metric = histogram();
        if let Data::Histogram { points, .. } = &mut metric.data {
            points[0].sum = f64::INFINITY;
        }
        let request = encode(&[metric]);
        let sum = &metrics_of(&request)[0]["histogram"]["dataPoints"][0]["sum"];
        assert_eq!(sum.as_f64(), Some(f64::MAX));
    }

    #[test]
    fn the_request_round_trips_through_text() {
        let request = encode(&[counter(), histogram()]);
        let text = request.to_json_string();
        let parsed = Value::parse(&text).expect("valid json");
        assert_eq!(parsed, request);
        assert_eq!(metrics_of(&parsed).len(), 2);
    }

    #[test]
    fn metrics_keep_their_order() {
        let request = encode(&[counter(), histogram()]);
        let names: Vec<_> = metrics_of(&request)
            .iter()
            .map(|m| m["name"].as_str().expect("name"))
            .collect();
        assert_eq!(
            names,
            ["mcp.server.requests", "mcp.server.request.duration"]
        );
    }
}
