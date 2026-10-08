//! Version negotiation and handshake fallback rules.

use rusty_json::Value;
use rusty_mcp_proto::{
    falls_back_to_initialize, negotiate_classic, pick_common, DiscoverParams, ErrorCode, ErrorData,
    Implementation, ProtocolVersion, RequestMeta, Wire,
};

fn v(s: &str) -> ProtocolVersion {
    ProtocolVersion::new(s)
}

#[test]
fn classic_answers_with_the_requested_version_when_served() {
    let served = [v("2024-11-05"), v("2025-03-26"), v("2025-06-18")];
    assert_eq!(
        negotiate_classic(&v("2025-03-26"), &served),
        Some(v("2025-03-26"))
    );
}

#[test]
fn classic_offers_its_newest_when_the_request_is_unknown() {
    let served = [v("2025-06-18"), v("2024-11-05"), v("2025-03-26")];
    // Newer than anything served, older than anything served, and garbage.
    for requested in ["2026-07-28", "2023-01-01", "draft"] {
        let got = negotiate_classic(&v(requested), &served);
        assert_eq!(got, Some(v("2025-06-18")), "requested {requested}");
    }
    assert_eq!(negotiate_classic(&v("2025-06-18"), &[]), None);
}

#[test]
fn stateless_picks_the_newest_common_revision() {
    let client = [v("2026-07-28"), v("2025-11-25"), v("2025-06-18")];
    let server = [v("2025-06-18"), v("2025-11-25")];
    assert_eq!(pick_common(&client, &server), Some(v("2025-11-25")));
    assert_eq!(pick_common(&client, &[v("2024-11-05")]), None);
    assert_eq!(pick_common(&[], &server), None);
}

#[test]
fn only_two_errors_trigger_the_initialize_fallback() {
    for (code, expected) in [
        (ErrorCode::METHOD_NOT_FOUND, true),
        (ErrorCode::UNSUPPORTED_PROTOCOL_VERSION, true),
        (ErrorCode::INTERNAL_ERROR, false),
        (ErrorCode::INVALID_PARAMS, false),
        (ErrorCode::HEADER_MISMATCH, false),
    ] {
        assert_eq!(
            falls_back_to_initialize(&ErrorData::new(code, "x")),
            expected
        );
    }
}

#[test]
fn request_meta_refuses_mistyped_known_keys_and_non_objects() {
    for text in [
        r#"{"io.modelcontextprotocol/protocolVersion":5}"#,
        r#"{"io.modelcontextprotocol/clientInfo":{"name":"c"}}"#,
        r#"{"io.modelcontextprotocol/clientCapabilities":[]}"#,
        r#"{"io.modelcontextprotocol/logLevel":3}"#,
        r#"{"progressToken":true}"#,
        r#"[]"#,
    ] {
        assert!(RequestMeta::from_json(text).is_err(), "accepted {text}");
    }
}

#[test]
fn request_meta_builds_and_keeps_unknown_keys() {
    let mut meta = RequestMeta::new();
    meta.protocol_version = Some(v("2026-07-28"));
    meta.client_info = Some(Implementation::new("c", "1"));
    meta.extra.insert("traceparent", "00-a-b-01");
    let back = RequestMeta::from_json(&meta.to_json()).expect("round trip");
    assert_eq!(back, meta);
    assert_eq!(
        RequestMeta::from_json("{}").expect("empty"),
        RequestMeta::new()
    );
}

#[test]
fn discover_params_are_empty_or_just_meta() {
    assert_eq!(
        DiscoverParams::from_json("{}").expect("empty"),
        DiscoverParams::default()
    );
    let p = DiscoverParams::from_json(r#"{"_meta":{"k":1}}"#).expect("meta");
    assert_eq!(
        p.meta.and_then(|m| m.get("k").cloned()),
        Some(Value::from(1))
    );
    assert!(DiscoverParams::from_json("[]").is_err());
}
