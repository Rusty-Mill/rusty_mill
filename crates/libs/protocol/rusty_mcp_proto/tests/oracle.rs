//! Differential tests against `rmcp` 3.1's serde model, the wire format
//! the MCP crates speak today. For each fixture the three forms must agree:
//! the text itself, what this crate re-encodes, and what `rmcp` re-encodes.

use rmcp::model as rm;
use rusty_mcp_proto::{
    AcknowledgedParams, CallToolParams, CallToolResponse, CallToolResult, CancelledParams,
    ClientCapabilities, CompleteParams, CompleteResult, ContentBlock, CreateTaskResult,
    DiscoverResult, ElicitParams, ElicitResult, ErrorData, GetPromptParams, GetPromptResult,
    GetTaskResult, Implementation, InitializeParams, InitializeResult, InputRequiredResult,
    ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult, ListToolsResult,
    ListenParams, ListenResult, Outcome, PaginatedParams, ProgressParams, Prompt,
    ReadResourceParams, ReadResourceResult, RequestMeta, Resource, ResourceContents,
    ResourceTemplate, ResourceUpdatedParams, ServerCapabilities, SubscribeParams, TaskAckResult,
    TaskIdParams, TaskStatusParams, Tool, UpdateTaskParams, Wire,
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

#[test]
fn prompts() {
    agree::<Prompt, rm::Prompt>(r#"{"name":"p"}"#);
    agree::<Prompt, rm::Prompt>(
        r#"{"name":"p","title":"P","description":"d","arguments":[{"name":"a"},
        {"name":"b","title":"B","description":"x","required":true}],"icons":[{"src":"x"}],"_meta":{"k":1}}"#,
    );
    both_refuse::<Prompt, rm::Prompt>(r#"{"title":"P"}"#);
    both_refuse::<Prompt, rm::Prompt>(r#"{"name":"p","arguments":[{"title":"no name"}]}"#);

    agree::<ListPromptsResult, rm::ListPromptsResult>(r#"{"prompts":[{"name":"p"}]}"#);
    agree::<ListPromptsResult, rm::ListPromptsResult>(
        r#"{"prompts":[],"nextCursor":"n","ttlMs":60000,"cacheScope":"private"}"#,
    );
    both_refuse::<ListPromptsResult, rm::ListPromptsResult>(r#"{}"#);
}

#[test]
fn get_prompt() {
    agree::<GetPromptParams, rm::GetPromptRequestParams>(r#"{"name":"p"}"#);
    agree::<GetPromptParams, rm::GetPromptRequestParams>(
        r#"{"name":"p","arguments":{"a":"1"},"inputResponses":{"q":{}},"requestState":"s","_meta":{"k":1}}"#,
    );
    both_refuse::<GetPromptParams, rm::GetPromptRequestParams>(r#"{"arguments":{}}"#);

    agree::<GetPromptResult, rm::GetPromptResult>(
        r#"{"messages":[{"role":"user","content":{"type":"text","text":"hi"}}]}"#,
    );
    agree::<GetPromptResult, rm::GetPromptResult>(
        r#"{"resultType":"complete","description":"d","messages":[
        {"role":"assistant","content":{"type":"image","data":"aGk=","mimeType":"image/png"}},
        {"role":"user","content":{"type":"resource","resource":{"uri":"u","text":"t"}}}],"_meta":{"k":1}}"#,
    );
    both_refuse::<GetPromptResult, rm::GetPromptResult>(
        r#"{"messages":[{"role":"system","content":{"type":"text","text":"x"}}]}"#,
    );
    both_refuse::<GetPromptResult, rm::GetPromptResult>(r#"{"description":"d"}"#);
}

#[test]
fn resource_methods() {
    agree::<ResourceTemplate, rm::ResourceTemplate>(r#"{"uriTemplate":"file:///{p}","name":"f"}"#);
    agree::<ResourceTemplate, rm::ResourceTemplate>(
        r#"{"uriTemplate":"file:///{p}","name":"f","title":"F","description":"d","mimeType":"text/plain",
        "icons":[{"src":"x"}],"_meta":{"k":1},"annotations":{"priority":0.5}}"#,
    );
    both_refuse::<ResourceTemplate, rm::ResourceTemplate>(r#"{"name":"f"}"#);

    agree::<ListResourcesResult, rm::ListResourcesResult>(
        r#"{"resources":[{"uri":"u","name":"n"}],"nextCursor":"c","ttlMs":0,"cacheScope":"public"}"#,
    );
    agree::<ListResourceTemplatesResult, rm::ListResourceTemplatesResult>(
        r#"{"resourceTemplates":[{"uriTemplate":"u/{a}","name":"n"}]}"#,
    );
    both_refuse::<ListResourcesResult, rm::ListResourcesResult>(r#"{"nextCursor":"c"}"#);
    both_refuse::<ListResourceTemplatesResult, rm::ListResourceTemplatesResult>(
        r#"{"resources":[]}"#,
    );

    agree::<ReadResourceParams, rm::ReadResourceRequestParams>(r#"{"uri":"u"}"#);
    agree::<ReadResourceParams, rm::ReadResourceRequestParams>(
        r#"{"uri":"u","inputResponses":{"q":{}},"requestState":"s","_meta":{"k":1}}"#,
    );
    both_refuse::<ReadResourceParams, rm::ReadResourceRequestParams>(r#"{}"#);

    agree::<ReadResourceResult, rm::ReadResourceResult>(r#"{"contents":[{"uri":"u","text":"t"}]}"#);
    agree::<ReadResourceResult, rm::ReadResourceResult>(
        r#"{"resultType":"complete","ttlMs":1000,"cacheScope":"private",
        "contents":[{"uri":"u","text":"t"},{"uri":"v","blob":"aGk=","mimeType":"image/png"}],"_meta":{"k":1}}"#,
    );
    both_refuse::<ReadResourceResult, rm::ReadResourceResult>(r#"{"ttlMs":1}"#);
}

#[test]
fn completion() {
    agree::<CompleteParams, rm::CompleteRequestParams>(
        r#"{"ref":{"type":"ref/prompt","name":"p"},"argument":{"name":"a","value":"x"}}"#,
    );
    agree::<CompleteParams, rm::CompleteRequestParams>(
        r#"{"ref":{"type":"ref/resource","uri":"file:///{p}"},"argument":{"name":"p","value":""},
        "context":{"arguments":{"q":"1"}},"_meta":{"k":1}}"#,
    );
    both_refuse::<CompleteParams, rm::CompleteRequestParams>(
        r#"{"ref":{"type":"ref/other","name":"p"},"argument":{"name":"a","value":"x"}}"#,
    );
    both_refuse::<CompleteParams, rm::CompleteRequestParams>(
        r#"{"ref":{"type":"ref/prompt","name":"p"},"argument":{"name":"a"}}"#,
    );

    agree::<CompleteResult, rm::CompleteResult>(r#"{"completion":{"values":[]}}"#);
    agree::<CompleteResult, rm::CompleteResult>(
        r#"{"resultType":"complete","completion":{"values":["a","b"],"total":10,"hasMore":true},"_meta":{"k":1}}"#,
    );
    both_refuse::<CompleteResult, rm::CompleteResult>(r#"{"completion":{"values":[1]}}"#);
    both_refuse::<CompleteResult, rm::CompleteResult>(r#"{}"#);
}

#[test]
fn cancel_and_progress() {
    agree::<CancelledParams, rm::CancelledNotificationParam>(r#"{}"#);
    agree::<CancelledParams, rm::CancelledNotificationParam>(
        r#"{"requestId":7,"reason":"user","_meta":{"k":1}}"#,
    );
    agree::<CancelledParams, rm::CancelledNotificationParam>(r#"{"requestId":"abc"}"#);
    both_refuse::<CancelledParams, rm::CancelledNotificationParam>(r#"{"requestId":true}"#);

    agree::<ProgressParams, rm::ProgressNotificationParam>(
        r#"{"progressToken":"t","progress":0.5}"#,
    );
    agree::<ProgressParams, rm::ProgressNotificationParam>(
        r#"{"progressToken":3,"progress":1.5,"total":4.5,"message":"m","_meta":{"k":1}}"#,
    );
    both_refuse::<ProgressParams, rm::ProgressNotificationParam>(r#"{"progress":0.5}"#);
    both_refuse::<ProgressParams, rm::ProgressNotificationParam>(r#"{"progressToken":"t"}"#);
    both_refuse::<ProgressParams, rm::ProgressNotificationParam>(
        r#"{"progressToken":"t","progress":"x"}"#,
    );
}

const CLIENT_CAPS: &str =
    r#"{"roots":{"listChanged":true},"sampling":{},"elicitation":{"form":{}}}"#;

#[test]
fn capabilities() {
    agree::<ServerCapabilities, rm::ServerCapabilities>(r#"{}"#);
    agree::<ServerCapabilities, rm::ServerCapabilities>(
        r#"{"experimental":{"x":{"a":1}},"extensions":{"io.modelcontextprotocol/tasks":{}},
        "logging":{},"completions":{},"prompts":{"listChanged":true},
        "resources":{"subscribe":true,"listChanged":false},"tools":{"listChanged":true}}"#,
    );
    agree::<ServerCapabilities, rm::ServerCapabilities>(
        r#"{"tools":{},"prompts":{},"resources":{}}"#,
    );
    both_refuse::<ServerCapabilities, rm::ServerCapabilities>(r#"{"tools":{"listChanged":"yes"}}"#);
    agree::<ClientCapabilities, rm::ClientCapabilities>(r#"{}"#);
    agree::<ClientCapabilities, rm::ClientCapabilities>(CLIENT_CAPS);
    agree::<ClientCapabilities, rm::ClientCapabilities>(
        r#"{"experimental":{"x":{}},"extensions":{"io.modelcontextprotocol/ui":{}}}"#,
    );
}

#[test]
fn initialize() {
    let info = r#"{"name":"c","version":"1"}"#;
    agree::<InitializeParams, rm::InitializeRequestParams>(&format!(
        r#"{{"protocolVersion":"2025-06-18","capabilities":{{}},"clientInfo":{info}}}"#
    ));
    agree::<InitializeParams, rm::InitializeRequestParams>(&format!(
        r#"{{"protocolVersion":"2025-11-25","capabilities":{CLIENT_CAPS},"clientInfo":{info},"_meta":{{"k":1}}}}"#
    ));
    both_refuse::<InitializeParams, rm::InitializeRequestParams>(&format!(
        r#"{{"capabilities":{{}},"clientInfo":{info}}}"#
    ));
    both_refuse::<InitializeParams, rm::InitializeRequestParams>(
        r#"{"protocolVersion":"2025-06-18","capabilities":{}}"#,
    );

    let server = r#"{"name":"s","version":"2"}"#;
    agree::<InitializeResult, rm::InitializeResult>(&format!(
        r#"{{"protocolVersion":"2025-06-18","capabilities":{{"tools":{{}}}},"serverInfo":{server}}}"#
    ));
    agree::<InitializeResult, rm::InitializeResult>(&format!(
        r#"{{"protocolVersion":"2024-11-05","capabilities":{{"tools":{{"listChanged":true}}}},
        "serverInfo":{server},"instructions":"use me","_meta":{{"k":1}}}}"#
    ));
    both_refuse::<InitializeResult, rm::InitializeResult>(
        r#"{"protocolVersion":"2025-06-18","capabilities":{}}"#,
    );
}

#[test]
fn discover() {
    agree::<DiscoverResult, rm::DiscoverResult>(
        r#"{"resultType":"complete","supportedVersions":["2025-11-25","2026-07-28"],
        "capabilities":{"tools":{}},"ttlMs":0,"cacheScope":"private"}"#,
    );
    agree::<DiscoverResult, rm::DiscoverResult>(
        r#"{"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{},
        "instructions":"hi","ttlMs":60000,"cacheScope":"public",
        "_meta":{"io.modelcontextprotocol/serverInfo":{"name":"s","version":"1"}}}"#,
    );
    both_refuse::<DiscoverResult, rm::DiscoverResult>(
        r#"{"resultType":"complete","supportedVersions":[],"capabilities":{},"cacheScope":"private"}"#,
    );
    both_refuse::<DiscoverResult, rm::DiscoverResult>(
        r#"{"resultType":"complete","supportedVersions":[],"capabilities":{},"ttlMs":0}"#,
    );
    let r = DiscoverResult::from_json(
        r#"{"resultType":"complete","supportedVersions":[],"capabilities":{},"ttlMs":0,"cacheScope":"private",
        "_meta":{"io.modelcontextprotocol/serverInfo":{"name":"s","version":"1"}}}"#,
    )
    .expect("decodes");
    assert_eq!(r.server_info().map(|i| i.name), Some("s".to_owned()));
}

/// `RequestMeta` reads the same members `rmcp`'s `RequestMetaObject` does.
#[test]
fn request_meta_matches_rmcp_accessors() {
    let text = format!(
        r#"{{"progressToken":"p1","io.modelcontextprotocol/protocolVersion":"2026-07-28",
        "io.modelcontextprotocol/clientInfo":{{"name":"c","version":"1"}},
        "io.modelcontextprotocol/clientCapabilities":{CLIENT_CAPS},
        "io.modelcontextprotocol/logLevel":"info","traceparent":"00-abc-def-01","vendor/x":{{"a":1}}}}"#
    );
    let ours = RequestMeta::from_json(&text).expect("decodes");
    let theirs: rm::RequestMetaObject = serde_json::from_str(&text).expect("rmcp decodes");
    assert_eq!(
        ours.protocol_version.as_ref().map(|v| v.as_str()),
        Some(theirs.protocol_version().expect("version").as_str())
    );
    assert_eq!(
        ours.client_info.as_ref().map(|i| i.name.as_str()),
        theirs.client_info().as_ref().map(|i| i.name.as_ref())
    );
    assert_eq!(
        json(&ours.client_capabilities.as_ref().expect("caps").to_json()),
        serde_json::to_value(theirs.client_capabilities().expect("caps")).expect("json")
    );
    // Trace context and vendor keys survive a round trip.
    assert_eq!(json(&ours.to_json()), json(&text));
    assert_eq!(
        ours.extra.get("traceparent").and_then(|v| v.as_str()),
        Some("00-abc-def-01")
    );
}

const ELICIT_SCHEMA: &str =
    r#"{"type":"object","properties":{"name":{"type":"string"}},"required":["name"]}"#;

#[test]
fn subscriptions() {
    agree::<rusty_mcp_proto::SubscriptionFilter, rm::SubscriptionFilter>(r#"{}"#);
    agree::<rusty_mcp_proto::SubscriptionFilter, rm::SubscriptionFilter>(
        r#"{"toolsListChanged":true,"promptsListChanged":false,"resourcesListChanged":true,
        "resourceSubscriptions":["file:///a","file:///b"]}"#,
    );
    both_refuse::<rusty_mcp_proto::SubscriptionFilter, rm::SubscriptionFilter>(
        r#"{"resourceSubscriptions":"file:///a"}"#,
    );
    agree::<ListenParams, rm::SubscriptionsListenRequestParams>(
        r#"{"notifications":{"toolsListChanged":true}}"#,
    );
    agree::<ListenParams, rm::SubscriptionsListenRequestParams>(
        r#"{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"},"notifications":{}}"#,
    );
    both_refuse::<ListenParams, rm::SubscriptionsListenRequestParams>(r#"{"_meta":{}}"#);
    agree::<AcknowledgedParams, rm::SubscriptionsAcknowledgedNotificationParams>(
        r#"{"_meta":{"k":1},"notifications":{"toolsListChanged":true}}"#,
    );
    agree::<ListenResult, rm::SubscriptionsListenResult>(
        r#"{"resultType":"complete","_meta":{"io.modelcontextprotocol/subscriptionId":7}}"#,
    );
    agree::<ListenResult, rm::SubscriptionsListenResult>(
        r#"{"resultType":"complete","_meta":{"io.modelcontextprotocol/subscriptionId":"s-1",
        "io.modelcontextprotocol/serverInfo":{"name":"s","version":"1"}}}"#,
    );
    both_refuse::<ListenResult, rm::SubscriptionsListenResult>(
        r#"{"resultType":"complete","_meta":{}}"#,
    );
    both_refuse::<ListenResult, rm::SubscriptionsListenResult>(r#"{"resultType":"complete"}"#);
    agree::<ResourceUpdatedParams, rm::ResourceUpdatedNotificationParam>(r#"{"uri":"file:///a"}"#);
    agree::<ResourceUpdatedParams, rm::ResourceUpdatedNotificationParam>(
        r#"{"uri":"file:///a","_meta":{"k":1}}"#,
    );
    both_refuse::<ResourceUpdatedParams, rm::ResourceUpdatedNotificationParam>(r#"{}"#);
    agree::<SubscribeParams, rm::SubscribeRequestParams>(r#"{"uri":"file:///a","_meta":{"k":1}}"#);
    both_refuse::<SubscribeParams, rm::SubscribeRequestParams>(r#"{}"#);
}

const TASK: &str =
    r#""taskId":"t1","createdAt":"2026-07-28T10:00:00Z","lastUpdatedAt":"2026-07-28T10:00:01Z""#;

#[test]
fn tasks() {
    agree::<CreateTaskResult, rm::CreateTaskResult>(&format!(
        r#"{{"resultType":"task",{TASK},"status":"working","ttlMs":null}}"#
    ));
    agree::<CreateTaskResult, rm::CreateTaskResult>(&format!(
        r#"{{"resultType":"task",{TASK},"status":"working","statusMessage":"go","ttlMs":60000,
        "pollIntervalMs":500,"_meta":{{"k":1}}}}"#
    ));
    both_refuse::<CreateTaskResult, rm::CreateTaskResult>(&format!(
        r#"{{"resultType":"complete",{TASK},"status":"working","ttlMs":null}}"#
    ));
    both_refuse::<CreateTaskResult, rm::CreateTaskResult>(&format!(
        r#"{{"resultType":"task",{TASK},"status":"paused","ttlMs":null}}"#
    ));

    for (status, extra) in [
        ("working", ""),
        ("cancelled", ""),
        (
            "completed",
            r#","result":{"content":[{"type":"text","text":"ok"}]}"#,
        ),
        ("failed", r#","error":{"code":-32603,"message":"boom"}"#),
        (
            "input_required",
            r#","inputRequests":{"q":{"method":"elicitation/create","params":{"mode":"form","message":"?","requestedSchema":{"type":"object","properties":{}}}}}"#,
        ),
    ] {
        let text = format!(
            r#"{{"resultType":"complete",{TASK},"status":"{status}","ttlMs":null{extra}}}"#
        );
        agree::<GetTaskResult, rm::GetTaskResult>(&text);
        // The same state as a notification (no resultType).
        let note =
            format!(r#"{{{TASK},"status":"{status}","ttlMs":null{extra},"_meta":{{"k":1}}}}"#);
        agree::<TaskStatusParams, rm::TaskStatusNotificationParams>(&note);
    }
    // A status without its payload member is refused by both.
    for status in ["completed", "failed", "input_required"] {
        both_refuse::<GetTaskResult, rm::GetTaskResult>(&format!(
            r#"{{"resultType":"complete",{TASK},"status":"{status}","ttlMs":null}}"#
        ));
    }

    agree::<TaskIdParams, rm::GetTaskParams>(r#"{"taskId":"t1"}"#);
    agree::<TaskIdParams, rm::CancelTaskParams>(r#"{"taskId":"t1","_meta":{"k":1}}"#);
    both_refuse::<TaskIdParams, rm::GetTaskParams>(r#"{}"#);
    agree::<UpdateTaskParams, rm::UpdateTaskParams>(
        r#"{"taskId":"t1","inputResponses":{"q":{"action":"accept","content":{"name":"x"}}}}"#,
    );
    both_refuse::<UpdateTaskParams, rm::UpdateTaskParams>(r#"{"taskId":"t1"}"#);
    agree::<TaskAckResult, rm::TaskAckResult>(r#"{"resultType":"complete"}"#);
    agree::<TaskAckResult, rm::TaskAckResult>(r#"{"resultType":"complete","_meta":{"k":1}}"#);
    both_refuse::<TaskAckResult, rm::TaskAckResult>(r#"{"resultType":"complete","extra":1}"#);
    both_refuse::<TaskAckResult, rm::TaskAckResult>(r#"{"resultType":"task"}"#);
}

#[test]
fn multi_round_trip_input() {
    let request = format!(
        r#"{{"method":"elicitation/create","params":{{"mode":"form","message":"Name?","requestedSchema":{ELICIT_SCHEMA}}}}}"#
    );
    agree::<InputRequiredResult, rm::InputRequiredResult>(&format!(
        r#"{{"resultType":"input_required","inputRequests":{{"who":{request}}},"requestState":"opaque","_meta":{{"k":1}}}}"#
    ));
    agree::<InputRequiredResult, rm::InputRequiredResult>(
        r#"{"resultType":"input_required","requestState":"only-state"}"#,
    );
    agree::<InputRequiredResult, rm::InputRequiredResult>(
        r#"{"resultType":"input_required","inputRequests":{"roots":{"method":"roots/list"}}}"#,
    );
    // Needs one of inputRequests or requestState; wrong resultType; unknown method.
    both_refuse::<InputRequiredResult, rm::InputRequiredResult>(
        r#"{"resultType":"input_required"}"#,
    );
    both_refuse::<InputRequiredResult, rm::InputRequiredResult>(
        r#"{"resultType":"complete","requestState":"s"}"#,
    );
    both_refuse::<InputRequiredResult, rm::InputRequiredResult>(
        r#"{"resultType":"input_required","inputRequests":{"x":{"method":"tools/call","params":{}}}}"#,
    );

    agree::<ElicitParams, rm::ElicitRequestParams>(&format!(
        r#"{{"mode":"form","message":"Name?","requestedSchema":{ELICIT_SCHEMA},"_meta":{{"k":1}}}}"#
    ));
    agree::<ElicitParams, rm::ElicitRequestParams>(
        r#"{"mode":"url","message":"Sign in","url":"https://x/auth","elicitationId":"e1"}"#,
    );
    both_refuse::<ElicitParams, rm::ElicitRequestParams>(r#"{"mode":"form","message":"x"}"#);
    both_refuse::<ElicitParams, rm::ElicitRequestParams>(r#"{"mode":"other","message":"x"}"#);
    agree::<ElicitResult, rm::ElicitResult>(r#"{"action":"accept","content":{"name":"x"}}"#);
    agree::<ElicitResult, rm::ElicitResult>(r#"{"action":"decline","_meta":{"k":1}}"#);
    both_refuse::<ElicitResult, rm::ElicitResult>(r#"{"action":"maybe"}"#);
}

/// A form without `mode` is how older peers send it; both sides read it as a form.
#[test]
fn elicit_without_mode_is_a_form() {
    let text = format!(r#"{{"message":"Name?","requestedSchema":{ELICIT_SCHEMA}}}"#);
    assert!(matches!(
        ElicitParams::from_json(&text).expect("decodes"),
        ElicitParams::Form { .. }
    ));
    assert!(serde_json::from_str::<rm::ElicitRequestParams>(&text).is_ok());
}

/// The outcome enums pick their variant from `resultType`, as `rmcp`'s do.
#[test]
fn responses_dispatch_on_result_type() {
    let complete = r#"{"content":[{"type":"text","text":"ok"}]}"#;
    let input = r#"{"resultType":"input_required","requestState":"s"}"#;
    let task = format!(r#"{{"resultType":"task",{TASK},"status":"working","ttlMs":null}}"#);

    assert!(matches!(
        CallToolResponse::from_json(complete),
        Ok(CallToolResponse::Complete(_))
    ));
    assert!(matches!(
        CallToolResponse::from_json(input),
        Ok(CallToolResponse::InputRequired(_))
    ));
    assert!(matches!(
        CallToolResponse::from_json(&task),
        Ok(CallToolResponse::Task(_))
    ));
    for text in [complete, input, &task] {
        // Each form re-encodes to itself.
        let ours = CallToolResponse::from_json(text).expect("decodes");
        assert_eq!(json(&ours.to_json()), json(text));
    }

    type Prompt = Outcome<GetPromptResult>;
    type Read = Outcome<ReadResourceResult>;
    let prompt = r#"{"messages":[{"role":"user","content":{"type":"text","text":"hi"}}]}"#;
    assert!(matches!(
        Prompt::from_json(prompt),
        Ok(Outcome::Complete(_))
    ));
    assert!(matches!(
        Prompt::from_json(input),
        Ok(Outcome::InputRequired(_))
    ));
    assert!(matches!(
        Read::from_json(r#"{"contents":[]}"#),
        Ok(Outcome::Complete(_))
    ));
    assert!(matches!(
        Read::from_json(input),
        Ok(Outcome::InputRequired(_))
    ));
    // A task is not a valid answer to prompts/get: it is read as a (bad) prompt result.
    assert!(Prompt::from_json(&task).is_err());
}
