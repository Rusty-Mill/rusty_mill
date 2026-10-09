//! The sans-IO dispatcher, driven with decoded messages.

use rusty_json::Value;
use rusty_mcp_proto::jsonrpc::code;
use rusty_mcp_proto::schema::Kind;
use rusty_mcp_proto::{
    CallToolResult, ErrorObject, InitializeResult, ListToolsResult, Message, RequestId, Schema,
    Tool,
};
use rusty_mcp_server::{Context, Dispatcher, Server, ToolError};

fn dispatcher() -> Dispatcher<Server> {
    Dispatcher::new(
        Server::new("test", "1.0.0")
            .instructions("hello")
            .tool(
                Tool::new(
                    "echo",
                    "Echo text.",
                    Schema::object()
                        .field("text", Kind::String, "Text", true)
                        .build(),
                ),
                |_, args| {
                    let text = args
                        .get("text")
                        .and_then(Value::as_str)
                        .ok_or_else(|| ToolError::from("text must be a string"))?;
                    Ok(CallToolResult::text(text))
                },
            )
            .tool(
                Tool::new("boom", "Panics.", Schema::object().build()),
                |_, _| panic!("handler bug"),
            ),
    )
}

fn request(id: i64, method: &str, params: Option<&str>) -> Message {
    Message::Request {
        id: RequestId::Number(id),
        method: method.to_string(),
        params: params.map(|p| Value::parse(p).expect("fixture is JSON")),
    }
}

fn ok(response: Option<Message>) -> Value {
    match response {
        Some(Message::Response { result, .. }) => result,
        other => panic!("expected a response, got {other:?}"),
    }
}

fn err(response: Option<Message>) -> ErrorObject {
    match response {
        Some(Message::Error { error, .. }) => error,
        other => panic!("expected an error, got {other:?}"),
    }
}

#[test]
fn initialize_echoes_a_supported_version_and_reports_what_is_offered() {
    let d = dispatcher();
    let params = r#"{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"c","version":"1"}}"#;
    let result = ok(d.handle(&Context::new(), request(1, "initialize", Some(params))));
    let init = InitializeResult::from_value(&result).expect("a valid result");
    assert_eq!(init.protocol_version, "2025-06-18");
    assert_eq!(init.server_info.name, "test");
    assert_eq!(init.instructions.as_deref(), Some("hello"));
    assert!(init.capabilities.tools.is_some());
    assert!(init.capabilities.resources.is_none());
}

#[test]
fn initialize_falls_back_to_the_newest_version_for_an_unknown_one() {
    let params = r#"{"protocolVersion":"1999-01-01","capabilities":{},"clientInfo":{"name":"c","version":"1"}}"#;
    let result = ok(dispatcher().handle(&Context::new(), request(1, "initialize", Some(params))));
    let init = InitializeResult::from_value(&result).expect("a valid result");
    assert_eq!(init.protocol_version, "2026-07-28");
}

#[test]
fn a_server_with_no_tools_offers_none() {
    let d = Dispatcher::new(Server::new("bare", "1"));
    let params = r#"{"protocolVersion":"2025-06-18","clientInfo":{"name":"c","version":"1"}}"#;
    let init = InitializeResult::from_value(&ok(
        d.handle(&Context::new(), request(1, "initialize", Some(params)))
    ))
    .expect("a valid result");
    assert!(init.capabilities.tools.is_none());
}

#[test]
fn ping_and_tools_list() {
    let d = dispatcher();
    assert_eq!(
        ok(d.handle(&Context::new(), request(1, "ping", None))),
        Value::object()
    );
    let listed = ListToolsResult::from_value(&ok(
        d.handle(&Context::new(), request(2, "tools/list", None))
    ))
    .expect("a valid result");
    let names: Vec<_> = listed.tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["echo", "boom"]);
}

#[test]
fn tools_call_runs_the_handler() {
    let result = ok(dispatcher().handle(
        &Context::new(),
        request(
            1,
            "tools/call",
            Some(r#"{"name":"echo","arguments":{"text":"hi"}}"#),
        ),
    ));
    assert_eq!(
        CallToolResult::from_value(&result).expect("valid"),
        CallToolResult::text("hi")
    );
}

#[test]
fn a_failing_tool_is_a_tool_result_not_a_protocol_error() {
    let result = ok(dispatcher().handle(
        &Context::new(),
        request(1, "tools/call", Some(r#"{"name":"echo","arguments":{}}"#)),
    ));
    let result = CallToolResult::from_value(&result).expect("valid");
    assert!(result.is_error);
}

#[test]
fn an_unknown_tool_is_invalid_params() {
    let error = err(dispatcher().handle(
        &Context::new(),
        request(1, "tools/call", Some(r#"{"name":"nope"}"#)),
    ));
    assert_eq!(error.code, code::INVALID_PARAMS);
    assert!(
        error.message.contains("Unknown tool: nope"),
        "{}",
        error.message
    );
}

#[test]
fn a_panicking_handler_becomes_an_internal_error() {
    let error = err(dispatcher().handle(
        &Context::new(),
        request(1, "tools/call", Some(r#"{"name":"boom"}"#)),
    ));
    assert_eq!(error.code, code::INTERNAL_ERROR);
}

#[test]
fn malformed_params_and_unknown_methods_are_refused() {
    let d = dispatcher();
    let bad = err(d.handle(
        &Context::new(),
        request(1, "tools/call", Some(r#"{"arguments":{}}"#)),
    ));
    assert_eq!(bad.code, code::INVALID_PARAMS);
    let unknown = err(d.handle(&Context::new(), request(2, "resources/list", None)));
    assert_eq!(unknown.code, code::METHOD_NOT_FOUND);
}

#[test]
fn notifications_and_responses_get_no_reply() {
    let d = dispatcher();
    let note = Message::Notification {
        method: "notifications/initialized".into(),
        params: None,
    };
    assert!(d.handle(&Context::new(), note).is_none());
    let stray = Message::Response {
        id: RequestId::Number(9),
        result: Value::object(),
    };
    assert!(d.handle(&Context::new(), stray).is_none());
}
