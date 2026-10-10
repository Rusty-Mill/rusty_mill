//! The `Schema` builder: exact output, replacement, and agreement with `rmcp`.

use rmcp::model as rm;
use rusty_json::Value;
use rusty_mcp_proto::{Kind, Schema, Tool, Wire};

fn parsed(text: &str) -> Value {
    Value::from_json_str(text).expect("fixture is JSON")
}

#[test]
fn builds_the_documented_shape() {
    let built = Schema::object()
        .field("city", Kind::String, "Where", true)
        .field("days", Kind::Integer, "", false)
        .field(
            "units",
            Kind::OneOf(vec!["c".into(), "f".into()]),
            "",
            false,
        )
        .field("tags", Kind::StringArray, "", false)
        .build();
    assert_eq!(
        built,
        parsed(
            r#"{"type":"object","properties":{
                "city":{"type":"string","description":"Where"},
                "days":{"type":"integer"},
                "units":{"type":"string","enum":["c","f"]},
                "tags":{"type":"array","items":{"type":"string"}}},
              "required":["city"]}"#
        )
    );
}

#[test]
fn empty_schema_has_no_required_key() {
    assert_eq!(
        Schema::object().build(),
        parsed(r#"{"type":"object","properties":{}}"#)
    );
}

#[test]
fn redefining_a_property_replaces_it_and_its_required_flag() {
    let built = Schema::object()
        .field("a", Kind::String, "", true)
        .field("a", Kind::Number, "", false)
        .build();
    assert_eq!(
        built,
        parsed(r#"{"type":"object","properties":{"a":{"type":"number"}}}"#)
    );
}

#[test]
fn hand_built_property_is_kept_verbatim() {
    let nested = parsed(r#"{"type":"object","properties":{"x":{"type":"boolean"}}}"#);
    let built = Schema::object()
        .property("opts", nested.clone(), true)
        .build();
    assert_eq!(
        built.get("properties").and_then(|p| p.get("opts")),
        Some(&nested)
    );
    assert_eq!(built.get("required"), Some(&parsed(r#"["opts"]"#)));
}

#[test]
fn a_tool_with_a_built_schema_decodes_in_rmcp() {
    let tool = Tool::new(
        "weather",
        Schema::object()
            .field("city", Kind::String, "Where", true)
            .build(),
    );
    let text = tool.to_json();
    let theirs: rm::Tool = serde_json::from_str(&text).expect("rmcp decodes");
    let encoded = serde_json::to_value(&theirs).expect("rmcp encodes");
    assert_eq!(
        encoded["inputSchema"]["required"],
        serde_json::json!(["city"])
    );
    assert_eq!(Tool::from_json(&text).expect("round trip"), tool);
}
