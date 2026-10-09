//! Spec-shaped messages decode and re-encode to themselves, and malformed
//! ones are refused with an error that names what was wrong.

use rusty_json::Value;
use rusty_mcp_proto::jsonrpc::code;
use rusty_mcp_proto::types::{CallToolParams, CallToolResult, Content};
use rusty_mcp_proto::{
    Error, ErrorObject, InitializeParams, InitializeResult, ListToolsResult, Message, RequestId,
    Schema, ServerCapabilities, Tool,
};

fn parse(text: &str) -> Value {
    Value::parse(text).expect("fixture is JSON")
}

#[test]
fn initialize_request_round_trips() {
    let text = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{"elicitation":{}},"clientInfo":{"name":"ExampleClient","title":"Example Client Display Name","version":"1.0.0"}}}"#;
    let message = Message::from_json(text).expect("decodes");
    let Message::Request { id, method, params } = &message else {
        panic!("not a request: {message:?}");
    };
    assert_eq!(*id, RequestId::Number(1));
    assert_eq!(method, "initialize");
    let params = InitializeParams::from_value(params.as_ref().expect("params")).expect("params");
    assert_eq!(params.protocol_version, "2025-06-18");
    assert_eq!(
        params.client_info.title.as_deref(),
        Some("Example Client Display Name")
    );
    assert_eq!(message.to_value(), parse(text));
}

#[test]
fn initialize_result_round_trips() {
    let text = r#"{"protocolVersion":"2025-06-18","capabilities":{"logging":{},"prompts":{"listChanged":true},"resources":{"listChanged":true},"tools":{"listChanged":true}},"serverInfo":{"name":"ExampleServer","title":"Example Server Display Name","version":"1.0.0"},"instructions":"Optional instructions for the client"}"#;
    let result = InitializeResult::from_value(&parse(text)).expect("decodes");
    assert!(result.capabilities.logging);
    assert!(result.capabilities.tools.expect("tools").list_changed);
    assert_eq!(result.to_value(), parse(text));
}

#[test]
fn tools_list_result_round_trips() {
    let text = r#"{"tools":[{"name":"get_weather","title":"Weather Information Provider","description":"Get current weather information for a location","inputSchema":{"type":"object","properties":{"location":{"type":"string","description":"City name or zip code"}},"required":["location"]}}],"nextCursor":"next-page-cursor"}"#;
    let result = ListToolsResult::from_value(&parse(text)).expect("decodes");
    assert_eq!(result.tools[0].name, "get_weather");
    assert_eq!(result.next_cursor.as_deref(), Some("next-page-cursor"));
    assert_eq!(result.to_value(), parse(text));
}

#[test]
fn schema_builder_matches_the_spec_example() {
    let schema = Schema::object()
        .field(
            "location",
            rusty_mcp_proto::schema::Kind::String,
            "City name or zip code",
            true,
        )
        .build();
    let tool = Tool::new(
        "get_weather",
        "Get current weather information for a location",
        schema,
    );
    let expected = parse(
        r#"{"name":"get_weather","description":"Get current weather information for a location","inputSchema":{"type":"object","properties":{"location":{"type":"string","description":"City name or zip code"}},"required":["location"]}}"#,
    );
    assert_eq!(tool.to_value(), expected);
}

#[test]
fn call_tool_params_and_results() {
    let params = CallToolParams::from_value(&parse(
        r#"{"name":"get_weather","arguments":{"location":"New York"}}"#,
    ))
    .expect("decodes");
    assert_eq!(params.name, "get_weather");
    assert!(params.arguments.expect("arguments").is_object());

    let ok = CallToolResult::from_value(&parse(
        r#"{"content":[{"type":"text","text":"72F"}],"isError":false}"#,
    ))
    .expect("decodes");
    assert_eq!(ok, CallToolResult::text("72F"));
    // `isError: false` is the default, so it is not written back.
    assert_eq!(
        ok.to_value(),
        parse(r#"{"content":[{"type":"text","text":"72F"}]}"#)
    );

    let failed = CallToolResult::error("boom");
    assert!(
        CallToolResult::from_value(&failed.to_value())
            .expect("decodes")
            .is_error
    );
}

#[test]
fn unknown_content_types_are_kept_not_dropped() {
    let image = r#"{"type":"image","data":"AAAA","mimeType":"image/png"}"#;
    let result = CallToolResult::from_value(&parse(&format!(r#"{{"content":[{image}]}}"#)))
        .expect("decodes");
    assert_eq!(result.content, vec![Content::Other(parse(image))]);
    assert_eq!(
        result
            .to_value()
            .get("content")
            .and_then(|c| c.get_index(0)),
        Some(&parse(image))
    );
}

#[test]
fn error_response_with_and_without_an_id() {
    let text = r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32602,"message":"Unknown tool: invalid_tool_name"}}"#;
    let message = Message::from_json(text).expect("decodes");
    assert_eq!(
        message,
        Message::Error {
            id: Some(RequestId::Number(2)),
            error: ErrorObject::new(code::INVALID_PARAMS, "Unknown tool: invalid_tool_name"),
        }
    );
    assert_eq!(message.to_value(), parse(text));

    let no_id = Message::from_json(
        r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}"#,
    )
    .expect("decodes");
    assert!(matches!(no_id, Message::Error { id: None, .. }));
}

#[test]
fn notification_has_no_id_and_string_ids_work() {
    let n = Message::from_json(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
        .expect("decodes");
    assert_eq!(
        n,
        Message::Notification {
            method: "notifications/initialized".into(),
            params: None
        }
    );
    let r = Message::from_json(r#"{"jsonrpc":"2.0","id":"abc","result":{}}"#).expect("decodes");
    assert!(matches!(r, Message::Response { id: RequestId::String(ref s), .. } if s == "abc"));
}

#[test]
fn malformed_messages_are_refused() {
    for (text, needle) in [
        ("not json", "invalid JSON"),
        (r#"[1,2]"#, "not an object"),
        (r#"{"id":1,"method":"x"}"#, "\"jsonrpc\""),
        (r#"{"jsonrpc":"1.0","id":1,"method":"x"}"#, "\"jsonrpc\""),
        (r#"{"jsonrpc":"2.0","id":1}"#, "no method, result or error"),
        (r#"{"jsonrpc":"2.0","result":{}}"#, "without an id"),
        (
            r#"{"jsonrpc":"2.0","id":1.5,"method":"x"}"#,
            "string or integer",
        ),
        (
            r#"{"jsonrpc":"2.0","id":1,"error":{"message":"m"}}"#,
            "code",
        ),
    ] {
        let err = Message::from_json(text).expect_err(text);
        assert!(err.to_string().contains(needle), "{text}: {err}");
    }
}

#[test]
fn bad_typed_params_name_the_field() {
    let err =
        CallToolParams::from_value(&parse(r#"{"name":"t","arguments":[1]}"#)).expect_err("array");
    assert!(matches!(
        err,
        Error::Decode {
            what: "CallToolParams",
            ..
        }
    ));
    let err = Tool::from_value(&parse(r#"{"description":"no name"}"#)).expect_err("no name");
    assert!(err.to_string().contains("\"name\""), "{err}");
    let caps = ServerCapabilities::from_value(&parse(r#"{"tools":5}"#)).expect_err("tools");
    assert!(
        caps.to_string().contains("ServerCapabilities.tools"),
        "{caps}"
    );
}

#[test]
fn unknown_members_are_ignored() {
    let tool = Tool::from_value(&parse(
        r#"{"name":"t","future":{"x":1},"inputSchema":{"type":"object"}}"#,
    ))
    .expect("decodes");
    assert_eq!(tool.name, "t");
}

// ---------------------------------------------------------------- P1 methods

mod p1 {
    use super::parse;
    use rusty_mcp_proto::{
        CompleteParams, CompleteResult, GetPromptParams, GetPromptResult, ListPromptsResult,
        ListResourceTemplatesResult, ListResourcesResult, ReadResourceResult, Reference,
        ResourceContents, ServerCapabilities,
    };

    #[test]
    fn resources_list_round_trips() {
        let text = r#"{"resources":[{"uri":"file:///project/src/main.rs","name":"main.rs","title":"Rust Software Application Main File","description":"Primary application entry point","mimeType":"text/x-rust","size":1024}],"nextCursor":"next-page-cursor"}"#;
        let result = ListResourcesResult::from_value(&parse(text)).expect("decodes");
        assert_eq!(result.resources[0].size, Some(1024));
        assert_eq!(result.to_value(), parse(text));
    }

    #[test]
    fn resource_templates_round_trip() {
        let text = r#"{"resourceTemplates":[{"uriTemplate":"file:///{path}","name":"Project Files","title":"Project Files","description":"Access files in the project directory","mimeType":"application/octet-stream"}]}"#;
        let result = ListResourceTemplatesResult::from_value(&parse(text)).expect("decodes");
        assert_eq!(result.resource_templates[0].uri_template, "file:///{path}");
        assert_eq!(result.to_value(), parse(text));
    }

    #[test]
    fn read_result_distinguishes_text_from_blob() {
        let text = r#"{"contents":[{"uri":"file:///a.rs","mimeType":"text/x-rust","text":"fn main() {}"},{"uri":"file:///b.bin","blob":"AAEC"}]}"#;
        let result = ReadResourceResult::from_value(&parse(text)).expect("decodes");
        assert!(matches!(result.contents[0], ResourceContents::Text { .. }));
        assert!(matches!(result.contents[1], ResourceContents::Blob { .. }));
        assert_eq!(result.to_value(), parse(text));
        let err = ReadResourceResult::from_value(&parse(r#"{"contents":[{"uri":"x"}]}"#))
            .expect_err("neither text nor blob");
        assert!(err.to_string().contains("neither"), "{err}");
    }

    #[test]
    fn prompts_round_trip() {
        let list = r#"{"prompts":[{"name":"code_review","title":"Request Code Review","description":"Asks the LLM to analyze code quality","arguments":[{"name":"code","description":"The code to review","required":true}]}]}"#;
        let result = ListPromptsResult::from_value(&parse(list)).expect("decodes");
        assert_eq!(result.prompts[0].arguments[0].required, Some(true));
        assert_eq!(result.to_value(), parse(list));

        let params = GetPromptParams::from_value(&parse(
            r#"{"name":"code_review","arguments":{"code":"def hello():\n    print('world')"}}"#,
        ))
        .expect("decodes");
        assert_eq!(params.arguments["code"], "def hello():\n    print('world')");
        assert_eq!(
            params.to_value(),
            parse(
                r#"{"name":"code_review","arguments":{"code":"def hello():\n    print('world')"}}"#
            )
        );

        let got = r#"{"description":"Code review prompt","messages":[{"role":"user","content":{"type":"text","text":"Please review this code"}}]}"#;
        let result = GetPromptResult::from_value(&parse(got)).expect("decodes");
        assert_eq!(result.to_value(), parse(got));
        let err = GetPromptResult::from_value(&parse(
            r#"{"messages":[{"role":"system","content":{"type":"text","text":"x"}}]}"#,
        ))
        .expect_err("bad role");
        assert!(err.to_string().contains("unknown role"), "{err}");
    }

    #[test]
    fn completion_round_trips() {
        let req = r#"{"ref":{"type":"ref/prompt","name":"code_review"},"argument":{"name":"language","value":"py"},"context":{"arguments":{"framework":"flask"}}}"#;
        let params = CompleteParams::from_value(&parse(req)).expect("decodes");
        assert_eq!(
            params.reference,
            Reference::Prompt {
                name: "code_review".into()
            }
        );
        assert_eq!(params.context["framework"], "flask");
        assert_eq!(params.to_value(), parse(req));

        let res =
            r#"{"completion":{"values":["python","pytorch","pyside"],"total":10,"hasMore":true}}"#;
        let result = CompleteResult::from_value(&parse(res)).expect("decodes");
        assert_eq!(result.completion.total, Some(10));
        assert_eq!(result.to_value(), parse(res));

        let err = CompleteParams::from_value(&parse(
            r#"{"ref":{"type":"ref/other"},"argument":{"name":"a","value":"b"}}"#,
        ))
        .expect_err("unknown ref");
        assert!(err.to_string().contains("unknown type"), "{err}");
    }

    #[test]
    fn resources_capability_carries_subscribe() {
        let text = r#"{"resources":{"subscribe":true,"listChanged":true}}"#;
        let caps = ServerCapabilities::from_value(&parse(text)).expect("decodes");
        let resources = caps.resources.expect("resources");
        assert!(resources.subscribe && resources.list_changed);
        assert_eq!(caps.to_value(), parse(text));
    }
}
