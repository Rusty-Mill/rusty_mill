#![allow(clippy::unwrap_used)]
//! Prompts, resources and completion through the stdio loop.

mod support;

use rusty_json::Value;
use rusty_mcp_proto::{
    CompletionInfo, ContentBlock, ErrorCode, ErrorData, GetPromptResult, Prompt, PromptArgument,
    PromptMessage, ReadResourceResult, Resource, ResourceContents, ResourceTemplate, Role,
};
use rusty_mcp_server::{BuildError, Server};
use std::sync::Arc;
use support::{code, reply, result, run_with};

const V: &str = r#""io.modelcontextprotocol/protocolVersion":"2026-07-28""#;

fn req(id: i64, method: &str, params: &str) -> String {
    let params = if params.is_empty() {
        format!(r#"{{"_meta":{{{V}}}}}"#)
    } else {
        format!(r#"{{{params},"_meta":{{{V}}}}}"#)
    };
    format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"{method}","params":{params}}}"#)
}

fn text_contents(uri: &str, text: &str) -> ReadResourceResult {
    ReadResourceResult {
        contents: vec![ResourceContents::Text {
            uri: uri.to_owned(),
            mime_type: Some("text/plain".to_owned()),
            text: text.to_owned(),
            meta: None,
        }],
        ..ReadResourceResult::default()
    }
}

fn full_server() -> Arc<Server> {
    let mut greet = Prompt::new("greet");
    greet.arguments = Some(vec![
        PromptArgument {
            name: "name".into(),
            title: None,
            description: None,
            required: Some(true),
        },
        PromptArgument {
            name: "tone".into(),
            title: None,
            description: None,
            required: None,
        },
    ]);
    Arc::new(
        Server::builder("features", "1")
            .page_size(2)
            .prompt(greet, |_c, get| {
                let arg = |k: &str| {
                    get.arguments
                        .as_ref()
                        .and_then(|a| a.get(k))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                };
                Ok(GetPromptResult {
                    description: Some("a greeting".into()),
                    messages: vec![PromptMessage {
                        role: Role::User,
                        content: ContentBlock::text(format!(
                            "Hello, {}! ({})",
                            arg("name"),
                            arg("tone")
                        )),
                    }],
                    ..GetPromptResult::default()
                })
            })
            .prompt(
                Prompt::new("plain"),
                |_c, _g| Ok(GetPromptResult::default()),
            )
            .prompt(
                Prompt::new("third"),
                |_c, _g| Ok(GetPromptResult::default()),
            )
            .resource(Resource::new("mem://readme", "readme"), |_c, r| {
                Ok(text_contents(&r.uri, "read me"))
            })
            .resource(Resource::new("mem://license", "license"), |_c, r| {
                Ok(text_contents(&r.uri, "MIT"))
            })
            .resource(Resource::new("mem://third", "third"), |_c, r| {
                Ok(text_contents(&r.uri, "3"))
            })
            .resource_template(
                ResourceTemplate::new("file:///{dir}/{name}", "file"),
                |_c, vars, r| {
                    let body =
                        format!("{}:{}", vars.get("dir").unwrap(), vars.get("name").unwrap());
                    Ok(text_contents(&r.uri, &body))
                },
            )
            .resource_template(
                ResourceTemplate::new("log://{+path}", "log"),
                |_c, vars, r| Ok(text_contents(&r.uri, vars.get("path").unwrap())),
            )
            .completer(|_c, p| {
                Ok(CompletionInfo {
                    values: vec![
                        format!("{}-1", p.argument.value),
                        format!("{}-2", p.argument.value),
                    ],
                    total: None,
                    has_more: None,
                })
            })
            .build()
            .unwrap(),
    )
}

fn text_of(m: &Value) -> &str {
    result(m)["contents"]
        .get_index(0)
        .unwrap()
        .get("text")
        .unwrap()
        .as_str()
        .unwrap()
}

#[test]
fn capabilities_follow_what_is_registered() {
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"c","version":"1"}}}"#;
    let caps = |s: Arc<Server>| {
        result(reply(&run_with(s, init), 1))
            .get("capabilities")
            .unwrap()
            .clone()
    };
    let full = caps(full_server());
    for k in ["prompts", "resources", "completions"] {
        assert!(full.get(k).is_some(), "{k}");
    }
    assert!(full.get("tools").is_none());
    let bare = caps(Arc::new(Server::builder("b", "1").build().unwrap()));
    assert_eq!(bare, Value::object());
    // Templates alone are enough for the resources capability.
    let only_templates = Server::builder("t", "1")
        .resource_template(ResourceTemplate::new("x://{a}", "x"), |_c, _v, r| {
            Ok(text_contents(&r.uri, ""))
        })
        .build()
        .unwrap();
    assert!(caps(Arc::new(only_templates)).get("resources").is_some());
}

#[test]
fn lists_page_with_opaque_cursors_and_cache_hints() {
    let srv = full_server();
    let walk = |method: &str, key: &str| -> Vec<String> {
        let mut names = Vec::new();
        let mut cursor: Option<String> = None;
        for id in 1..=10 {
            let params = cursor
                .as_ref()
                .map(|c| format!(r#""cursor":"{c}""#))
                .unwrap_or_default();
            let out = run_with(srv.clone(), &req(id, method, &params));
            let page = result(reply(&out, id));
            assert_eq!(page["ttlMs"].as_i64(), Some(0));
            assert_eq!(page["cacheScope"].as_str(), Some("public"));
            for item in page[key].as_array().unwrap() {
                names.push(item.get("name").unwrap().as_str().unwrap().to_owned());
            }
            cursor = page
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(String::from);
            if cursor.is_none() {
                break;
            }
        }
        names
    };
    assert_eq!(walk("prompts/list", "prompts"), ["greet", "plain", "third"]);
    assert_eq!(
        walk("resources/list", "resources"),
        ["readme", "license", "third"]
    );
    assert_eq!(
        walk("resources/templates/list", "resourceTemplates"),
        ["file", "log"]
    );

    // A cursor from one list is refused by another.
    let out = run_with(srv.clone(), &req(1, "prompts/list", ""));
    let prompt_cursor = result(reply(&out, 1))["nextCursor"]
        .as_str()
        .unwrap()
        .to_owned();
    let out = run_with(
        srv,
        &req(
            2,
            "resources/list",
            &format!(r#""cursor":"{prompt_cursor}""#),
        ),
    );
    assert_eq!(code(reply(&out, 2)), ErrorCode::INVALID_PARAMS.0);
}

#[test]
fn prompts_get_checks_required_arguments_and_names() {
    let srv = full_server();
    let get = |id, args: &str| {
        req(
            id,
            "prompts/get",
            &format!(r#""name":"greet","arguments":{args}"#),
        )
    };
    let input = [
        get(1, r#"{"name":"Ada","tone":"warm"}"#),
        get(2, r#"{"tone":"warm"}"#),
        get(3, "{}"),
        req(4, "prompts/get", r#""name":"nope""#),
        req(5, "prompts/get", r#""name":"greet""#),
    ]
    .join("\n");
    let out = run_with(srv, &input);
    let ok = result(reply(&out, 1));
    assert_eq!(
        ok["messages"].get_index(0).unwrap()["content"]["text"].as_str(),
        Some("Hello, Ada! (warm)")
    );
    assert_eq!(ok["resultType"].as_str(), Some("complete"));
    for id in [2, 3, 4, 5] {
        assert_eq!(
            code(reply(&out, id)),
            ErrorCode::INVALID_PARAMS.0,
            "id {id}"
        );
    }
}

#[test]
fn resources_read_exact_uris_templates_and_misses() {
    let srv = full_server();
    let input = [
        req(1, "resources/read", r#""uri":"mem://readme""#),
        req(2, "resources/read", r#""uri":"file:///src/main.rs""#),
        req(3, "resources/read", r#""uri":"log://a/b/c.log""#),
        req(4, "resources/read", r#""uri":"file:///a/b/c""#),
        req(5, "resources/read", r#""uri":"mem://missing""#),
        req(6, "resources/read", "").replace("\"params\":{", "\"params\":{\"nouri\":1,"),
    ]
    .join("\n");
    let out = run_with(srv, &input);
    assert_eq!(text_of(reply(&out, 1)), "read me");
    assert_eq!(
        result(reply(&out, 1))["resultType"].as_str(),
        Some("complete")
    );
    assert_eq!(
        text_of(reply(&out, 2)),
        "src:main.rs",
        "a template's variables reach the handler"
    );
    assert_eq!(
        text_of(reply(&out, 3)),
        "a/b/c.log",
        "{{+path}} keeps its slashes"
    );
    // `{name}` stops at '/', so this matches neither template: not found.
    for id in [4, 5] {
        let e = reply(&out, id).get("error").unwrap();
        assert_eq!(
            e["code"].as_i64(),
            Some(i64::from(ErrorCode::RESOURCE_NOT_FOUND.0)),
            "id {id}"
        );
    }
    assert_eq!(
        reply(&out, 5)["error"]["data"]["uri"].as_str(),
        Some("mem://missing")
    );
    assert_eq!(code(reply(&out, 6)), ErrorCode::INVALID_PARAMS.0);
}

#[test]
fn an_exact_resource_wins_over_a_template_that_also_matches() {
    let srv = Arc::new(
        Server::builder("o", "1")
            .resource_template(ResourceTemplate::new("mem://{x}", "any"), |_c, _v, r| {
                Ok(text_contents(&r.uri, "template"))
            })
            .resource(Resource::new("mem://special", "special"), |_c, r| {
                Ok(text_contents(&r.uri, "exact"))
            })
            .build()
            .unwrap(),
    );
    let input = [
        req(1, "resources/read", r#""uri":"mem://special""#),
        req(2, "resources/read", r#""uri":"mem://other""#),
    ]
    .join("\n");
    let out = run_with(srv, &input);
    assert_eq!(text_of(reply(&out, 1)), "exact");
    assert_eq!(text_of(reply(&out, 2)), "template");
}

#[test]
fn completion_validates_the_reference_and_caps_the_values() {
    let srv = Arc::new(
        Server::builder("c", "1")
            .prompt(
                {
                    let mut p = Prompt::new("p");
                    p.arguments = Some(vec![PromptArgument {
                        name: "a".into(),
                        title: None,
                        description: None,
                        required: None,
                    }]);
                    p
                },
                |_c, _g| Ok(GetPromptResult::default()),
            )
            .resource_template(ResourceTemplate::new("x://{v}", "x"), |_c, _v, r| {
                Ok(text_contents(&r.uri, ""))
            })
            .completer(|_c, p| {
                let n: usize = p.argument.value.parse().unwrap_or(0);
                Ok(CompletionInfo {
                    values: (0..n).map(|i| format!("v{i}")).collect(),
                    total: None,
                    has_more: None,
                })
            })
            .build()
            .unwrap(),
    );
    let complete = |id, reference: &str, arg: &str, value: &str| {
        req(
            id,
            "completion/complete",
            &format!(r#""ref":{reference},"argument":{{"name":"{arg}","value":"{value}"}}"#),
        )
    };
    let prompt = r#"{"type":"ref/prompt","name":"p"}"#;
    let input = [
        complete(1, prompt, "a", "3"),
        complete(2, prompt, "a", "250"),
        complete(3, r#"{"type":"ref/resource","uri":"x://{v}"}"#, "v", "1"),
        complete(4, r#"{"type":"ref/prompt","name":"nope"}"#, "a", "1"),
        complete(5, prompt, "not_an_argument", "1"),
        complete(6, r#"{"type":"ref/resource","uri":"y://{v}"}"#, "v", "1"),
    ]
    .join("\n");
    let out = run_with(srv, &input);
    let c = &result(reply(&out, 1))["completion"];
    assert_eq!(c["values"].as_array().unwrap().len(), 3);
    assert!(c.get("hasMore").is_none());
    // Over the cap: 100 values, the real count as `total`, and `hasMore`.
    let c = &result(reply(&out, 2))["completion"];
    assert_eq!(c["values"].as_array().unwrap().len(), 100);
    assert_eq!(c["total"].as_i64(), Some(250));
    assert_eq!(c["hasMore"].as_bool(), Some(true));
    assert_eq!(
        result(reply(&out, 3))["completion"]["values"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    for id in [4, 5, 6] {
        assert_eq!(
            code(reply(&out, id)),
            ErrorCode::INVALID_PARAMS.0,
            "id {id}"
        );
    }
}

#[test]
fn methods_of_features_a_server_lacks_are_not_found() {
    let srv = Arc::new(Server::builder("bare", "1").build().unwrap());
    let methods = [
        ("prompts/list", ""),
        ("prompts/get", r#""name":"x""#),
        ("resources/list", ""),
        ("resources/templates/list", ""),
        ("resources/read", r#""uri":"x://y""#),
        (
            "completion/complete",
            r#""ref":{"type":"ref/prompt","name":"x"},"argument":{"name":"a","value":""}"#,
        ),
    ];
    let input: String = methods
        .iter()
        .enumerate()
        .map(|(i, (m, p))| req(i as i64 + 1, m, p) + "\n")
        .collect();
    let out = run_with(srv, &input);
    for i in 1..=methods.len() as i64 {
        assert_eq!(
            code(reply(&out, i)),
            ErrorCode::METHOD_NOT_FOUND.0,
            "{}",
            methods[i as usize - 1].0
        );
    }
}

#[test]
fn handler_errors_are_passed_through_unchanged() {
    let srv = Arc::new(
        Server::builder("e", "1")
            .resource(Resource::new("mem://x", "x"), |_c, _r| {
                Err(ErrorData::new(ErrorCode(-32050), "custom failure"))
            })
            .build()
            .unwrap(),
    );
    let out = run_with(srv, &req(1, "resources/read", r#""uri":"mem://x""#));
    assert_eq!(code(reply(&out, 1)), -32050);
}

#[test]
fn the_builder_refuses_duplicates_and_unmatchable_templates() {
    let p = || Prompt::new("p");
    let r = || Resource::new("mem://r", "r");
    let ok = |_c: &_, _g| Ok(GetPromptResult::default());
    assert_eq!(
        Server::builder("d", "1")
            .prompt(p(), ok)
            .prompt(p(), ok)
            .build()
            .err(),
        Some(BuildError::DuplicatePrompt("p".into()))
    );
    let read = |_c: &_, r: rusty_mcp_proto::ReadResourceParams| Ok(text_contents(&r.uri, ""));
    assert_eq!(
        Server::builder("d", "1")
            .resource(r(), read)
            .resource(r(), read)
            .build()
            .err(),
        Some(BuildError::DuplicateResource("mem://r".into()))
    );
    let tpl = |s: &str| ResourceTemplate::new(s, "t");
    let tread =
        |_c: &_, _v: &_, r: rusty_mcp_proto::ReadResourceParams| Ok(text_contents(&r.uri, ""));
    assert_eq!(
        Server::builder("d", "1")
            .resource_template(tpl("a://{x}"), tread)
            .resource_template(tpl("a://{x}"), tread)
            .build()
            .err(),
        Some(BuildError::DuplicateTemplate("a://{x}".into()))
    );
    for bad in ["a://{?q}", "a://{x}{y}", "a://{", ""] {
        let built = Server::builder("d", "1")
            .resource_template(tpl(bad), tread)
            .build();
        assert!(matches!(built, Err(BuildError::BadTemplate(_))), "{bad:?}");
    }
}
