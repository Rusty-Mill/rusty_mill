//! Differential check against `rmcp` (a dev-dependency used only as an
//! oracle, as ADR-0002 allows): what `rmcp` writes, this crate reads, and what
//! this crate writes, `rmcp` reads. Equality is on the JSON text re-parsed, so
//! member order does not matter.

use rmcp::model::{
    CallToolRequestParams, CallToolResult as RCallToolResult, ContentBlock,
    Implementation as RImpl, JsonObject, ListToolsResult as RListToolsResult,
    ServerCapabilities as RCaps, ServerInfo as RServerInfo, Tool as RTool,
};
use rusty_json::Value;
use rusty_mcp_proto::schema::Kind;
use rusty_mcp_proto::{
    CallToolParams, CallToolResult, InitializeResult, ListToolsResult, Schema, Tool,
};
use std::sync::Arc;

fn ours(text: &str) -> Value {
    Value::parse(text).expect("our JSON parses")
}

/// `rmcp` 3.1 writes two members this slice does not model yet, so they are
/// removed before comparing; any other difference still fails the test.
/// - `resultType: "complete"`: a 2026-07-28 member on every result (scope
///   doc, P2). Decoding already ignores it.
/// - `isError: false`: the default, which this crate leaves out.
fn theirs<T: serde::Serialize>(value: &T) -> Value {
    let mut wire = ours(&serde_json::to_string(value).expect("rmcp serializes"));
    if let Some(map) = wire.as_object_mut() {
        map.remove("resultType");
        if map.get("isError").and_then(Value::as_bool) == Some(false) {
            map.remove("isError");
        }
    }
    wire
}

fn schema_object() -> Arc<JsonObject> {
    let text = Schema::object()
        .field("city", Kind::String, "City name", true)
        .field("days", Kind::Integer, "", false)
        .build()
        .to_json_string();
    match serde_json::from_str::<serde_json::Value>(&text).expect("schema parses") {
        serde_json::Value::Object(map) => Arc::new(map),
        other => panic!("schema is not an object: {other}"),
    }
}

#[test]
fn rmcp_tool_reads_back_the_same() {
    let tool = RTool::new("forecast", "Get a forecast", schema_object());
    let wire = theirs(&tool);
    let decoded = Tool::from_value(&wire).expect("we decode rmcp's tool");
    assert_eq!(decoded.name, "forecast");
    assert_eq!(decoded.description.as_deref(), Some("Get a forecast"));
    assert_eq!(decoded.to_value(), wire, "and re-encode it identically");
}

#[test]
fn our_tool_reads_in_rmcp() {
    let ours_tool = Tool::new(
        "forecast",
        "Get a forecast",
        Schema::object()
            .field("city", Kind::String, "City name", true)
            .build(),
    );
    let text = ours_tool.to_value().to_json_string();
    let parsed: RTool = serde_json::from_str(&text).expect("rmcp reads our tool");
    assert_eq!(parsed.name, "forecast");
    assert_eq!(theirs(&parsed), ours_tool.to_value());
}

#[test]
fn call_results_agree_both_ways() {
    let ok = RCallToolResult::success(vec![ContentBlock::text("72F")]);
    let wire = theirs(&ok);
    assert_eq!(
        CallToolResult::from_value(&wire).expect("decode"),
        CallToolResult::text("72F")
    );
    assert_eq!(CallToolResult::text("72F").to_value(), wire);

    let err = RCallToolResult::error(vec![ContentBlock::text("boom")]);
    let wire = theirs(&err);
    assert_eq!(
        CallToolResult::from_value(&wire).expect("decode"),
        CallToolResult::error("boom")
    );
    assert_eq!(CallToolResult::error("boom").to_value(), wire);
}

#[test]
fn call_params_agree_both_ways() {
    let mut args = JsonObject::new();
    args.insert("city".into(), serde_json::Value::String("Oslo".into()));
    let params = CallToolRequestParams::new("forecast").with_arguments(args);
    let wire = theirs(&params);
    let decoded = CallToolParams::from_value(&wire).expect("decode");
    assert_eq!(decoded.name, "forecast");
    assert_eq!(decoded.to_value(), wire);
}

#[test]
fn list_tools_agree_both_ways() {
    let result = RListToolsResult {
        tools: vec![RTool::new("a", "first", schema_object())],
        ..Default::default()
    };
    let wire = theirs(&result);
    let decoded = ListToolsResult::from_value(&wire).expect("decode");
    assert_eq!(decoded.tools.len(), 1);
    assert_eq!(decoded.to_value(), wire);
}

#[test]
fn initialize_result_agrees() {
    let info = RServerInfo::new(RCaps::builder().enable_tools().build())
        .with_server_info(RImpl::new("srv", "1.2.3"))
        .with_instructions("be nice");
    let wire = theirs(&info);
    let decoded = InitializeResult::from_value(&wire).expect("decode");
    assert_eq!(decoded.server_info.name, "srv");
    assert_eq!(decoded.instructions.as_deref(), Some("be nice"));
    assert!(decoded.capabilities.tools.is_some());
    assert_eq!(decoded.to_value(), wire);
}
