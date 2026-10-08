//! Differential tests against `rmcp` 3.1's serde model, the wire format
//! the MCP crates speak today. For each fixture the three forms must agree:
//! the text itself, what this crate re-encodes, and what `rmcp` re-encodes.

use rmcp::model as rm;
use rusty_mcp_proto::{
    CallToolParams, CallToolResult, ContentBlock, ErrorData, Implementation, ListToolsResult,
    PaginatedParams, Resource, ResourceContents, Tool, Wire,
};
use serde::{de::DeserializeOwned, Serialize};

fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).expect("fixture is JSON")
}

/// `text` survives this crate unchanged and matches what `rmcp` makes of it.
fn agree<T: Wire, R: DeserializeOwned + Serialize>(text: &str) {
    let ours = T::from_json(text).unwrap_or_else(|e| panic!("decode {text}: {e}"));
    let encoded = json(&ours.to_json());
    assert_eq!(encoded, json(text), "re-encoding changed the fixture");
    let theirs: R = serde_json::from_str(text).unwrap_or_else(|e| panic!("rmcp {text}: {e}"));
    assert_eq!(
        encoded,
        serde_json::to_value(theirs).expect("rmcp encodes"),
        "rmcp encodes {text} differently"
    );
}

/// Both sides refuse `text`.
fn both_refuse<T: Wire, R: DeserializeOwned>(text: &str) {
    assert!(T::from_json(text).is_err(), "this crate accepted {text}");
    assert!(
        serde_json::from_str::<R>(text).is_err(),
        "rmcp accepted {text}"
    );
}

// Annotation priorities use binary-exact values (0.5, 0.25, 1.0): `rmcp` keeps
// `priority` as `f32`, so it re-encodes 0.2 as 0.20000000298. This crate keeps
// annotations as raw JSON and forwards 0.2 untouched, which is the safer side.

const SCHEMA: &str = r#"{"type":"object","properties":{"a":{"type":"integer"}},"required":["a"]}"#;

#[test]
fn tool_minimal_and_full() {
    agree::<Tool, rm::Tool>(&format!(r#"{{"name":"add","inputSchema":{SCHEMA}}}"#));
    agree::<Tool, rm::Tool>(&format!(
        r#"{{"name":"add","title":"Add","description":"Adds.","inputSchema":{SCHEMA},
        "outputSchema":{SCHEMA},"annotations":{{"readOnlyHint":true}},
        "icons":[{{"src":"https://x/i.png"}}],"_meta":{{"k":1}}}}"#
    ));
}

#[test]
fn tool_refusals() {
    both_refuse::<Tool, rm::Tool>(r#"{"name":"add"}"#);
    both_refuse::<Tool, rm::Tool>(&format!(r#"{{"inputSchema":{SCHEMA}}}"#));
}

#[test]
fn list_tools_with_paging_and_cache_hints() {
    let tool = format!(r#"{{"name":"add","inputSchema":{SCHEMA}}}"#);
    agree::<ListToolsResult, rm::ListToolsResult>(&format!(r#"{{"tools":[{tool}]}}"#));
    agree::<ListToolsResult, rm::ListToolsResult>(&format!(
        r#"{{"resultType":"complete","tools":[{tool}],"nextCursor":"abc","ttlMs":0,"cacheScope":"public","_meta":{{"a":1}}}}"#
    ));
    both_refuse::<ListToolsResult, rm::ListToolsResult>(r#"{"nextCursor":"abc"}"#);
}

#[test]
fn negative_ttl_is_clamped_like_rmcp() {
    let text = r#"{"tools":[],"ttlMs":-5}"#;
    let ours = ListToolsResult::from_json(text).expect("decodes");
    let theirs: rm::ListToolsResult = serde_json::from_str(text).expect("rmcp decodes");
    assert_eq!(ours.paging.ttl_ms, Some(0));
    assert_eq!(theirs.ttl_ms, Some(0));
}

#[test]
fn call_tool_params() {
    agree::<CallToolParams, rm::CallToolRequestParams>(r#"{"name":"add"}"#);
    agree::<CallToolParams, rm::CallToolRequestParams>(
        r#"{"name":"add","arguments":{"a":1,"b":2},"inputResponses":{"q":{"action":"accept"}},
        "requestState":"opaque","_meta":{"progressToken":"t1"}}"#,
    );
    both_refuse::<CallToolParams, rm::CallToolRequestParams>(r#"{"arguments":{}}"#);
    both_refuse::<CallToolParams, rm::CallToolRequestParams>(r#"{"name":"add","arguments":[1]}"#);
}

#[test]
fn call_tool_result_variants() {
    agree::<CallToolResult, rm::CallToolResult>(r#"{"content":[{"type":"text","text":"3"}]}"#);
    agree::<CallToolResult, rm::CallToolResult>(
        r#"{"resultType":"complete","content":[{"type":"text","text":"3"}],
        "structuredContent":{"sum":3},"isError":false,"_meta":{"k":"v"}}"#,
    );
    agree::<CallToolResult, rm::CallToolResult>(
        r#"{"content":[{"type":"text","text":"boom"}],"isError":true}"#,
    );
}

#[test]
fn content_blocks() {
    for text in [
        r#"{"type":"text","text":"hi"}"#,
        r#"{"type":"text","text":"hi","annotations":{"audience":["user"],"priority":0.5},"_meta":{"a":1}}"#,
        r#"{"type":"image","data":"aGk=","mimeType":"image/png"}"#,
        r#"{"type":"audio","data":"aGk=","mimeType":"audio/wav","annotations":{"priority":1.0}}"#,
        r#"{"type":"resource","resource":{"uri":"file:///a","mimeType":"text/plain","text":"x"}}"#,
        r#"{"type":"resource","resource":{"uri":"file:///a","blob":"aGk="},"_meta":{"a":1}}"#,
        r#"{"type":"resource_link","uri":"file:///a","name":"a","mimeType":"text/plain","size":3}"#,
    ] {
        agree::<ContentBlock, rm::ContentBlock>(text);
    }
    both_refuse::<ContentBlock, rm::ContentBlock>(r#"{"type":"video","data":"x"}"#);
    both_refuse::<ContentBlock, rm::ContentBlock>(r#"{"type":"text"}"#);
    both_refuse::<ContentBlock, rm::ContentBlock>(r#"{"type":"image","data":"x"}"#);
}

#[test]
fn resources() {
    agree::<Resource, rm::Resource>(r#"{"uri":"file:///a","name":"a"}"#);
    agree::<Resource, rm::Resource>(
        r#"{"uri":"file:///a","name":"a","title":"A","description":"d","mimeType":"text/plain",
        "size":10,"icons":[{"src":"x"}],"_meta":{"a":1},"annotations":{"priority":0.25}}"#,
    );
    agree::<ResourceContents, rm::ResourceContents>(
        r#"{"uri":"u","mimeType":"text/plain","text":"t"}"#,
    );
    agree::<ResourceContents, rm::ResourceContents>(r#"{"uri":"u","blob":"aGk=","_meta":{"a":1}}"#);
    both_refuse::<ResourceContents, rm::ResourceContents>(r#"{"uri":"u"}"#);
    both_refuse::<Resource, rm::Resource>(r#"{"uri":"u"}"#);
}

#[test]
fn error_data() {
    agree::<ErrorData, rm::ErrorData>(r#"{"code":-32602,"message":"bad"}"#);
    agree::<ErrorData, rm::ErrorData>(r#"{"code":-32002,"message":"gone","data":{"uri":"x"}}"#);
    both_refuse::<ErrorData, rm::ErrorData>(r#"{"code":-32602}"#);
}

#[test]
fn implementation_and_pagination() {
    agree::<Implementation, rm::Implementation>(r#"{"name":"s","version":"1.0"}"#);
    agree::<Implementation, rm::Implementation>(
        r#"{"name":"s","title":"S","version":"1.0","description":"d","icons":[{"src":"x"}],"websiteUrl":"https://x"}"#,
    );
    both_refuse::<Implementation, rm::Implementation>(r#"{"name":"s"}"#);
    agree::<PaginatedParams, rm::PaginatedRequestParams>(r#"{}"#);
    agree::<PaginatedParams, rm::PaginatedRequestParams>(
        r#"{"cursor":"c","_meta":{"progressToken":1}}"#,
    );
}
