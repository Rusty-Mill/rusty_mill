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
    let unknown = err(d.handle(&Context::new(), request(2, "bogus/method", None)));
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

// ------------------------------------------------- resources, prompts, completion

mod content {
    use super::*;
    use rusty_mcp_proto::prompts::{GetPromptResult as GetResult, PromptMessage, Role};
    use rusty_mcp_proto::{
        CompleteResult, Completion, ListPromptsResult, ListResourceTemplatesResult,
        ListResourcesResult, Prompt, PromptArgument, ReadResourceResult, Resource,
        ResourceContents, ResourceTemplate,
    };

    fn rich() -> Dispatcher<Server> {
        let mut greet = Prompt::new("greet");
        let mut name = PromptArgument::new("name");
        name.required = Some(true);
        greet.arguments.push(name);
        Dispatcher::new(
            Server::new("rich", "1")
                .resource(Resource::new("mem://a", "a"), |_, uri| {
                    Ok(vec![ResourceContents::text(uri, "A")])
                })
                .resource(Resource::new("mem://bad", "bad"), |_, _| {
                    Err(ToolError::from("disk on fire"))
                })
                .resource_template(ResourceTemplate::new("mem://n/{id}", "n"))
                .read_other(|_, uri| {
                    uri.strip_prefix("mem://n/")
                        .map(|id| Ok(vec![ResourceContents::text(uri, id)]))
                })
                .prompt(greet, |_, args| {
                    Ok(GetResult {
                        description: None,
                        messages: vec![PromptMessage::text(Role::User, args["name"].clone())],
                    })
                })
                .completer(|_, _| Completion {
                    values: vec!["x".into()],
                    total: Some(1),
                    has_more: Some(false),
                }),
        )
    }

    #[test]
    fn capabilities_follow_what_is_registered() {
        let params = r#"{"protocolVersion":"2025-06-18","clientInfo":{"name":"c","version":"1"}}"#;
        let init = InitializeResult::from_value(&ok(
            rich().handle(&Context::new(), request(1, "initialize", Some(params)))
        ))
        .expect("valid");
        assert!(init.capabilities.resources.is_some());
        assert!(init.capabilities.prompts.is_some());
        assert!(init.capabilities.completions);
        assert!(init.capabilities.tools.is_none());
    }

    #[test]
    fn resources_list_templates_and_read() {
        let d = rich();
        let c = Context::new();
        let list =
            ListResourcesResult::from_value(&ok(d.handle(&c, request(1, "resources/list", None))))
                .expect("valid");
        assert_eq!(list.resources.len(), 2);
        let templates = ListResourceTemplatesResult::from_value(&ok(
            d.handle(&c, request(2, "resources/templates/list", None))
        ))
        .expect("valid");
        assert_eq!(templates.resource_templates[0].uri_template, "mem://n/{id}");
        let read = |uri: &str| {
            d.handle(
                &c,
                request(3, "resources/read", Some(&format!(r#"{{"uri":"{uri}"}}"#))),
            )
        };
        let fixed = ReadResourceResult::from_value(&ok(read("mem://a"))).expect("valid");
        assert_eq!(fixed.contents, vec![ResourceContents::text("mem://a", "A")]);
        let templated = ReadResourceResult::from_value(&ok(read("mem://n/42"))).expect("valid");
        assert_eq!(
            templated.contents,
            vec![ResourceContents::text("mem://n/42", "42")]
        );
        assert_eq!(err(read("mem://missing")).code, code::RESOURCE_NOT_FOUND);
        assert_eq!(err(read("mem://bad")).code, code::INTERNAL_ERROR);
    }

    #[test]
    fn prompts_check_required_arguments() {
        let d = rich();
        let c = Context::new();
        let list =
            ListPromptsResult::from_value(&ok(d.handle(&c, request(1, "prompts/list", None))))
                .expect("valid");
        assert_eq!(list.prompts[0].name, "greet");
        let got = GetResult::from_value(&ok(d.handle(
            &c,
            request(
                2,
                "prompts/get",
                Some(r#"{"name":"greet","arguments":{"name":"Ada"}}"#),
            ),
        )))
        .expect("valid");
        assert_eq!(got.messages[0], PromptMessage::text(Role::User, "Ada"));
        let missing = err(d.handle(&c, request(3, "prompts/get", Some(r#"{"name":"greet"}"#))));
        assert_eq!(missing.code, code::INVALID_PARAMS);
        assert!(
            missing.message.contains("Missing required argument: name"),
            "{}",
            missing.message
        );
        let unknown = err(d.handle(&c, request(4, "prompts/get", Some(r#"{"name":"nope"}"#))));
        assert!(
            unknown.message.contains("Unknown prompt"),
            "{}",
            unknown.message
        );
    }

    #[test]
    fn completion_and_the_absent_case() {
        let params = r#"{"ref":{"type":"ref/prompt","name":"greet"},"argument":{"name":"name","value":"a"}}"#;
        let done = CompleteResult::from_value(&ok(rich().handle(
            &Context::new(),
            request(1, "completion/complete", Some(params)),
        )))
        .expect("valid");
        assert_eq!(done.completion.values, ["x"]);
        // A server with no completer does not offer the method.
        let bare = Dispatcher::new(Server::new("bare", "1"));
        assert_eq!(
            err(bare.handle(
                &Context::new(),
                request(2, "completion/complete", Some(params))
            ))
            .code,
            code::METHOD_NOT_FOUND
        );
        // And empty lists, not errors, for the list methods it does not use.
        let empty = ListResourcesResult::from_value(&ok(
            bare.handle(&Context::new(), request(3, "resources/list", None))
        ))
        .expect("valid");
        assert!(empty.resources.is_empty());
    }
}
