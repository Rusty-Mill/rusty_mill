//! The OTLP/HTTP JSON encoding of metrics.
//!
//! Built from the [`rusty_json::Value`] tree, following the OTLP spec's JSON
//! mapping of the protobuf messages rather than any SDK's types:
//!
//! - 64-bit integers (`asInt`, `intValue`, `count`, `bucketCounts`, the
//!   `*UnixNano` times) are decimal **strings**, since JSON numbers cannot hold
//!   them exactly;
//! - trace and span ids are lowercase **hex**, not base64;
//! - enums are their integer values (`aggregationTemporality` is `2`, for
//!   cumulative);
//! - field names are lowerCamelCase.
//!
//! These are the bodies of `POST /v1/metrics` and `POST /v1/traces` with
//! `Content-Type: application/json`.

use rusty_json::{Value, json};

use super::instrument::{Data, HistogramPoint, Int, Metric};
use super::labels::Label;
use super::span::{Attribute, AttributeValue, Event, FinishedSpan, hex};

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

/// An `ExportTraceServiceRequest` carrying `spans`.
///
/// `resource` describes the process and `scope` names the instrumentation.
pub(crate) fn traces_request(
    resource: &[(String, String)],
    scope: &str,
    spans: &[FinishedSpan],
) -> Value {
    let spans: Vec<Value> = spans.iter().map(encode_span).collect();
    json!({
        "resourceSpans": [{
            "resource": { "attributes": attributes(resource.iter().map(|(k, v)| (k.as_str(), v.as_str()))) },
            "scopeSpans": [{
                "scope": { "name": scope },
                "spans": spans,
            }],
        }],
    })
}

fn encode_span(span: &FinishedSpan) -> Value {
    let mut encoded = json!({
        "traceId": hex(&span.trace_id),
        "spanId": hex(&span.span_id),
        "name": span.name.as_str(),
        "kind": span.kind.code(),
        "startTimeUnixNano": span.start_nanos.to_string(),
        "endTimeUnixNano": span.end_nanos.to_string(),
        "attributes": typed_attributes(&span.attributes),
        "events": span.events.iter().map(encode_event).collect::<Vec<Value>>(),
    });
    let Some(object) = encoded.as_object_mut() else {
        return encoded;
    };
    if let Some(parent) = &span.parent_span_id {
        object.insert("parentSpanId".to_owned(), Value::from(hex(parent)));
    }
    if let Some(state) = &span.trace_state {
        object.insert("traceState".to_owned(), Value::from(state.as_str()));
    }
    if span.status.code() != 0 {
        let mut status = json!({ "code": span.status.code() });
        if let (Some(message), Some(map)) = (&span.status_message, status.as_object_mut()) {
            map.insert("message".to_owned(), Value::from(message.as_str()));
        }
        object.insert("status".to_owned(), status);
    }
    encoded
}

fn encode_event(event: &Event) -> Value {
    json!({
        "timeUnixNano": event.time_nanos.to_string(),
        "name": event.name.as_str(),
        "attributes": typed_attributes(&event.attributes),
    })
}

fn typed_attributes(attributes: &[Attribute]) -> Value {
    let list: Vec<Value> = attributes
        .iter()
        .map(|(key, value)| json!({ "key": key.as_str(), "value": any_value(value) }))
        .collect();
    Value::from(list)
}

/// An OTLP `AnyValue`.
fn any_value(value: &AttributeValue) -> Value {
    match value {
        AttributeValue::String(text) => json!({ "stringValue": text.as_str() }),
        AttributeValue::Bool(flag) => json!({ "boolValue": *flag }),
        AttributeValue::Int(int) => json!({ "intValue": int.to_string() }),
        // JSON has no NaN or infinity; say what it was as text instead of null.
        AttributeValue::Float(float) if !float.is_finite() => {
            json!({ "stringValue": float.to_string() })
        }
        AttributeValue::Float(float) => json!({ "doubleValue": *float }),
    }
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
    use crate::otel::span::{Kind, Status};

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

    fn span() -> FinishedSpan {
        FinishedSpan {
            trace_id: [0xab; 16],
            span_id: [0xcd; 8],
            parent_span_id: Some([0xef; 8]),
            trace_state: Some("vendor=opaque".to_owned()),
            name: "tools/call".to_owned(),
            kind: Kind::Server,
            start_nanos: 10,
            end_nanos: 20,
            attributes: vec![
                ("s".to_owned(), "text".into()),
                ("b".to_owned(), true.into()),
                ("i".to_owned(), AttributeValue::Int(-7)),
                ("f".to_owned(), AttributeValue::Float(1.5)),
            ],
            events: vec![Event {
                time_nanos: 15,
                name: "retry".to_owned(),
                attributes: vec![("attempt".to_owned(), AttributeValue::Int(2))],
            }],
            status: Status::Error,
            status_message: Some("boom".to_owned()),
        }
    }

    fn encoded_span(span: &FinishedSpan) -> Value {
        let request = traces_request(&resource(), "rusty-mcp", std::slice::from_ref(span));
        request["resourceSpans"][0]["scopeSpans"][0]["spans"][0].clone()
    }

    #[test]
    fn a_span_has_hex_ids_string_times_and_an_integer_kind() {
        let request = traces_request(&resource(), "rusty-mcp", &[span()]);
        let resource = &request["resourceSpans"][0]["resource"]["attributes"][0];
        assert_eq!(resource["key"].as_str(), Some("service.name"));
        let scope = &request["resourceSpans"][0]["scopeSpans"][0]["scope"];
        assert_eq!(scope["name"].as_str(), Some("rusty-mcp"));

        let span = encoded_span(&span());
        assert_eq!(span["traceId"].as_str(), Some("ab".repeat(16).as_str()));
        assert_eq!(span["spanId"].as_str(), Some("cd".repeat(8).as_str()));
        assert_eq!(span["parentSpanId"].as_str(), Some("ef".repeat(8).as_str()));
        assert_eq!(span["traceState"].as_str(), Some("vendor=opaque"));
        assert_eq!(span["name"].as_str(), Some("tools/call"));
        assert_eq!(span["kind"].as_u64(), Some(2));
        assert_eq!(span["startTimeUnixNano"].as_str(), Some("10"));
        assert_eq!(span["endTimeUnixNano"].as_str(), Some("20"));
    }

    #[test]
    fn attribute_values_use_their_otlp_forms() {
        let span = encoded_span(&span());
        let attribute = |n: usize| &span["attributes"][n]["value"];
        assert_eq!(attribute(0)["stringValue"].as_str(), Some("text"));
        assert_eq!(attribute(1)["boolValue"].as_bool(), Some(true));
        // 64-bit integers are strings.
        assert_eq!(attribute(2)["intValue"].as_str(), Some("-7"));
        assert_eq!(attribute(3)["doubleValue"].as_f64(), Some(1.5));
    }

    #[test]
    fn a_non_finite_float_is_text_not_null() {
        let mut span = span();
        span.attributes = vec![("f".to_owned(), AttributeValue::Float(f64::NAN))];
        let encoded = encoded_span(&span);
        assert_eq!(
            encoded["attributes"][0]["value"]["stringValue"].as_str(),
            Some("NaN")
        );
    }

    #[test]
    fn events_and_status_are_encoded() {
        let span = encoded_span(&span());
        let event = &span["events"][0];
        assert_eq!(event["name"].as_str(), Some("retry"));
        assert_eq!(event["timeUnixNano"].as_str(), Some("15"));
        assert_eq!(
            event["attributes"][0]["value"]["intValue"].as_str(),
            Some("2")
        );
        assert_eq!(span["status"]["code"].as_u64(), Some(2));
        assert_eq!(span["status"]["message"].as_str(), Some("boom"));
    }

    #[test]
    fn a_root_span_has_no_parent_and_an_unset_status_is_omitted() {
        let mut root = span();
        root.parent_span_id = None;
        root.trace_state = None;
        root.status = Status::Unset;
        root.status_message = None;
        let encoded = encoded_span(&root);
        assert!(encoded.get("parentSpanId").is_none());
        assert!(encoded.get("traceState").is_none());
        assert!(encoded.get("status").is_none());
    }

    #[test]
    fn the_traces_request_round_trips_through_text() {
        let request = traces_request(&resource(), "rusty-mcp", &[span(), span()]);
        let parsed = Value::parse(&request.to_json_string()).expect("valid json");
        assert_eq!(parsed, request);
    }
}
