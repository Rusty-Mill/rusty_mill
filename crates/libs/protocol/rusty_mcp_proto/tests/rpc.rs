//! JSON-RPC 2.0 envelope and version handling.

use rusty_json::Value;
use rusty_mcp_proto::rpc::params;
use rusty_mcp_proto::tool::method;
use rusty_mcp_proto::{
    CallToolParams, Error, ErrorCode, ErrorData, Message, PaginatedParams, ProtocolVersion,
    RequestId, Wire,
};

fn roundtrip(text: &str) -> Message {
    let m = Message::from_json(text).unwrap_or_else(|e| panic!("{text}: {e}"));
    let again = Value::from_json_str(&m.to_json()).expect("encodes JSON");
    assert_eq!(again, Value::from_json_str(text).expect("fixture is JSON"));
    m
}

#[test]
fn request_with_number_and_string_ids() {
    let m = roundtrip(r#"{"jsonrpc":"2.0","id":7,"method":"tools/list","params":{"cursor":"c"}}"#);
    let Message::Request {
        id,
        method,
        params: p,
    } = m
    else {
        panic!("not a request")
    };
    assert_eq!((id, method.as_str()), (RequestId::Number(7), method::LIST));
    assert_eq!(
        params::<PaginatedParams>(&p)
            .expect("params")
            .cursor
            .as_deref(),
        Some("c")
    );

    let m = roundtrip(r#"{"jsonrpc":"2.0","id":"a-1","method":"ping"}"#);
    assert!(
        matches!(m, Message::Request { id: RequestId::String(s), params: None, .. } if s == "a-1")
    );
}

#[test]
fn notification_has_no_id() {
    let m = roundtrip(
        r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}"#,
    );
    assert!(matches!(m, Message::Notification { .. }));
}

#[test]
fn response_and_error_response() {
    let m = roundtrip(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#);
    assert!(matches!(
        m,
        Message::Response {
            id: RequestId::Number(1),
            ..
        }
    ));

    let m = roundtrip(
        r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32601,"message":"no","data":{"m":"x"}}}"#,
    );
    let Message::Error { id, error } = m else {
        panic!("not an error")
    };
    assert_eq!(id, Some(RequestId::Number(2)));
    assert_eq!(error.code, ErrorCode::METHOD_NOT_FOUND);
    assert!(error.data.is_some());
}

#[test]
fn error_response_with_null_id() {
    let m = Message::from_json(
        r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"parse"}}"#,
    )
    .expect("decodes");
    assert!(matches!(&m, Message::Error { id: None, .. }));
    // Re-encoding keeps the explicit null the spec requires.
    let v = Value::from_json_str(&m.to_json()).expect("json");
    assert_eq!(v.get("id"), Some(&Value::Null));
}

#[test]
fn constructors_build_the_expected_wire_form() {
    let req = Message::request(
        RequestId::Number(1),
        method::CALL,
        &CallToolParams::new("add"),
    );
    assert_eq!(
        Value::from_json_str(&req.to_json()).expect("json"),
        Value::from_json_str(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"add"}}"#
        )
        .expect("json")
    );
    let err = Message::error(None, ErrorData::new(ErrorCode::INVALID_REQUEST, "bad"));
    assert!(matches!(err, Message::Error { id: None, .. }));
}

#[test]
fn malformed_messages_are_refused() {
    for text in [
        r#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#, // batch
        r#"{"id":1,"method":"ping"}"#,                   // no jsonrpc
        r#"{"jsonrpc":"1.0","id":1,"method":"ping"}"#,   // wrong version
        r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#, // request with null id
        r#"{"jsonrpc":"2.0","id":1.5,"method":"ping"}"#, // fractional id
        r#"{"jsonrpc":"2.0","id":true,"method":"ping"}"#, // boolean id
        r#"{"jsonrpc":"2.0","result":{}}"#,              // response without id
        r#"{"jsonrpc":"2.0","id":1}"#,                   // nothing to say
        r#"{"jsonrpc":"2.0","id":1,"error":{"code":1.5,"message":"x"}}"#,
        r#"{"jsonrpc":"2.0","id":1,"error":{"code":99999999999,"message":"x"}}"#,
        r#"{"jsonrpc":"2.0","id":1,"error":{"code":-1}}"#,
        r#"42"#,
    ] {
        assert!(Message::from_json(text).is_err(), "accepted {text}");
    }
    assert!(matches!(
        Message::from_json("{not json"),
        Err(Error::Json(_))
    ));
}

#[test]
fn absent_params_decode_as_an_empty_object() {
    let p: PaginatedParams = params(&None).expect("empty params");
    assert_eq!(p, PaginatedParams::default());
    // A type with a required member refuses absent params.
    assert!(params::<CallToolParams>(&None).is_err());
}

#[test]
fn protocol_versions_order_by_date() {
    let classic = ProtocolVersion::new(ProtocolVersion::V_2025_11_25);
    let stateless = ProtocolVersion::new(ProtocolVersion::V_2026_07_28);
    assert!(!classic.is_stateless());
    assert!(stateless.is_stateless());
    assert!(ProtocolVersion::new("2027-01-01").is_stateless());
    assert!(classic < stateless);
    // A revision this crate has never heard of still round-trips.
    let odd = ProtocolVersion::from_json(r#""draft""#).expect("decodes");
    assert_eq!(odd.as_str(), "draft");
    assert!(ProtocolVersion::from_json("5").is_err());
}
