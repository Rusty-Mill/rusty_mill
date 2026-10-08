#![allow(clippy::unwrap_used)]
//! The client against `rusty_mcp_server`, in one process: the two ends talk
//! over pipes, so every call crosses the real newline-delimited JSON framing.

use rusty_mcp_client_native::json::Value;
use rusty_mcp_client_native::proto::{
    CallToolResult, ContentBlock, ElicitAction, ElicitParams, ElicitResult, ErrorData, Prompt,
    PromptMessage, ProtocolVersion, ReadResourceResult, Resource, ResourceContents,
    ResourceTemplate, Role, Tool,
};
use rusty_mcp_client_native::{
    Client, ClientConfig, ClientError, Handler, NoHandler, StdioTransport,
};
use rusty_mcp_server::{
    answer, serve_lines, Ask, Server, ServerBuilder, StdioConfig, ToolOutcome, Turn,
};
use std::io::BufReader;
use std::sync::Arc;
use std::time::Duration;

const SECS: Duration = Duration::from_secs(5);

fn schema() -> Value {
    let mut s = Value::object();
    s.insert("type", "object");
    s
}

fn text(s: &str) -> CallToolResult {
    CallToolResult {
        content: vec![ContentBlock::text(s)],
        ..CallToolResult::default()
    }
}

fn form(message: &str) -> ElicitParams {
    ElicitParams::Form {
        message: message.to_owned(),
        requested_schema: schema(),
        meta: None,
    }
}

fn first_text(r: &CallToolResult) -> String {
    match &r.content[0] {
        ContentBlock::Text { text, .. } => text.clone(),
        other => panic!("not text: {other:?}"),
    }
}

fn builder() -> ServerBuilder {
    Server::builder("fixture", "1.0")
        .page_size(2)
        .tool(Tool::new("add", schema()), |_c, call| {
            let n = |k: &str| {
                call.arguments
                    .as_ref()
                    .and_then(|a| a.get(k))
                    .and_then(Value::as_i64)
            };
            match (n("a"), n("b")) {
                (Some(a), Some(b)) => Ok(text(&(a + b).to_string())),
                _ => Err(ErrorData::new(
                    rusty_mcp_client_native::proto::ErrorCode::INVALID_PARAMS,
                    "a and b are required",
                )),
            }
        })
        .tool(Tool::new("slow", schema()), |_c, _p| {
            std::thread::sleep(Duration::from_millis(400));
            Ok(text("late"))
        })
        .tool(Tool::new("progress", schema()), |ctx, _p| {
            for i in 1..=3 {
                ctx.progress(f64::from(i), Some(3.0), None);
            }
            Ok(text("done"))
        })
        .interactive_tool(Tool::new("confirm", schema()), |ctx, call| {
            match ctx.turn(&call)? {
                Turn::Fresh => Ok(ToolOutcome::Ask(
                    Ask::new()
                        .elicit("ok", &form("Proceed?"))
                        .with_state("pending"),
                )),
                Turn::Resumed { state } => {
                    let yes =
                        answer(&call, "ok")?.is_some_and(|r| r.action == ElicitAction::Accept);
                    Ok(ToolOutcome::Done(text(&format!(
                        "{} {}",
                        String::from_utf8_lossy(state),
                        if yes { "accepted" } else { "declined" }
                    ))))
                }
            }
        })
        .interactive_tool(Tool::new("nag", schema()), |_ctx, _call| {
            Ok(ToolOutcome::Ask(
                Ask::new().elicit("ok", &form("Again?")).with_state("x"),
            ))
        })
        .task_tool(Tool::new("job", schema()), |_ctx, _p| Ok(text("job done")))
        .task_tool(Tool::new("ask_job", schema()), |ctx, _p| {
            let reply = ctx.elicit("name", &form("Name?")).map_err(|_| {
                ErrorData::new(
                    rusty_mcp_client_native::proto::ErrorCode::INTERNAL_ERROR,
                    "no",
                )
            })?;
            let who = reply
                .content
                .as_ref()
                .and_then(|c| c.get("n"))
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_owned();
            Ok(text(&format!("hello {who}")))
        })
        .prompt(Prompt::new("hi"), |_c, _g| {
            Ok(rusty_mcp_client_native::proto::GetPromptResult {
                messages: vec![PromptMessage {
                    role: Role::User,
                    content: ContentBlock::text("hi there"),
                }],
                ..Default::default()
            })
        })
        .resource(Resource::new("mem://a", "a"), |_c, read| {
            Ok(ReadResourceResult {
                contents: vec![ResourceContents::Text {
                    uri: read.uri,
                    mime_type: None,
                    text: "contents of a".to_owned(),
                    meta: None,
                }],
                ..ReadResourceResult::default()
            })
        })
        .resource_template(
            ResourceTemplate::new("mem://t/{x}", "t"),
            |_c, vars, read| {
                Ok(ReadResourceResult {
                    contents: vec![ResourceContents::Text {
                        uri: read.uri,
                        mime_type: None,
                        text: format!("x={}", vars.get("x").unwrap_or_default()),
                        meta: None,
                    }],
                    ..ReadResourceResult::default()
                })
            },
        )
}

/// Connect a client to `server` over pipes.
fn transport(server: Server) -> StdioTransport {
    let (server_in, client_out) = std::io::pipe().unwrap();
    let (client_in, server_out) = std::io::pipe().unwrap();
    std::thread::spawn(move || {
        let _ = serve_lines(
            Arc::new(server),
            BufReader::new(server_in),
            server_out,
            StdioConfig::default(),
        );
    });
    StdioTransport::new(client_in, client_out)
}

fn config() -> ClientConfig {
    ClientConfig::new("test-client", "0.1")
}

fn with_tasks(mut c: ClientConfig) -> ClientConfig {
    let mut caps = c.capabilities.to_value_for_test();
    let mut ext = Value::object();
    ext.insert("io.modelcontextprotocol/tasks", Value::object());
    caps.insert("extensions", ext);
    c.capabilities = rusty_mcp_client_native::proto::ClientCapabilities::from_value_for_test(&caps);
    c
}

trait CapsJson {
    fn to_value_for_test(&self) -> Value;
    fn from_value_for_test(v: &Value) -> Self;
}
impl CapsJson for rusty_mcp_client_native::proto::ClientCapabilities {
    fn to_value_for_test(&self) -> Value {
        use rusty_mcp_client_native::proto::Wire;
        self.to_value()
    }
    fn from_value_for_test(v: &Value) -> Self {
        use rusty_mcp_client_native::proto::Wire;
        Self::from_value(v).unwrap()
    }
}

fn connect<H: Handler>(
    server: Server,
    config: ClientConfig,
    handler: H,
) -> Client<StdioTransport, H> {
    Client::connect(transport(server), config, handler, SECS).unwrap()
}

fn args(a: i64, b: i64) -> Option<Value> {
    let mut v = Value::object();
    v.insert("a", a);
    v.insert("b", b);
    Some(v)
}

#[test]
fn the_stateless_handshake_then_calls() {
    let mut c = connect(builder().build().unwrap(), config(), NoHandler);
    assert_eq!(c.session().negotiated().unwrap().as_str(), "2026-07-28");
    assert_eq!(c.session().server_info().unwrap().name, "fixture");
    c.ping().unwrap();
    let tools = c.list_tools().unwrap();
    assert_eq!(tools.len(), 7, "followed the pages");
    let r = c.call_tool("add", args(20, 22)).unwrap();
    assert_eq!(first_text(&r), "42");
}

#[test]
fn the_classic_handshake_when_only_classic_is_configured() {
    let mut cfg = config();
    cfg.versions.retain(|v| !v.is_stateless());
    let mut c = connect(builder().build().unwrap(), cfg, NoHandler);
    assert_eq!(c.session().negotiated().unwrap().as_str(), "2025-11-25");
    assert_eq!(first_text(&c.call_tool("add", args(1, 2)).unwrap()), "3");
}

#[test]
fn a_classic_only_server_makes_a_stateless_first_client_fall_back() {
    let server = builder()
        .versions(vec![
            ProtocolVersion::new("2025-06-18"),
            ProtocolVersion::new("2025-03-26"),
        ])
        .build()
        .unwrap();
    let mut c = connect(server, config(), NoHandler);
    assert_eq!(c.session().negotiated().unwrap().as_str(), "2025-06-18");
    assert_eq!(first_text(&c.call_tool("add", args(2, 2)).unwrap()), "4");
}

#[test]
fn no_shared_revision_is_an_error() {
    let server = builder()
        .versions(vec![ProtocolVersion::new("2025-06-18")])
        .build()
        .unwrap();
    let mut cfg = config();
    cfg.versions
        .retain(rusty_mcp_client_native::proto::ProtocolVersion::is_stateless);
    let err = Client::connect(transport(server), cfg, NoHandler, SECS)
        .err()
        .unwrap();
    assert!(
        matches!(err, ClientError::Rpc(_) | ClientError::NoCommonVersion),
        "{err}"
    );
}

#[test]
fn a_server_error_is_an_rpc_error_and_the_client_carries_on() {
    let mut c = connect(builder().build().unwrap(), config(), NoHandler);
    let err = c.call_tool("add", None).unwrap_err();
    match err {
        ClientError::Rpc(e) => assert_eq!(e.code.0, -32602),
        other => panic!("{other}"),
    }
    assert_eq!(first_text(&c.call_tool("add", args(1, 1)).unwrap()), "2");
}

#[test]
fn prompts_resources_and_templates() {
    let mut c = connect(builder().build().unwrap(), config(), NoHandler);
    assert_eq!(c.list_prompts().unwrap()[0].name, "hi");
    let hi = c.get_prompt("hi", None).unwrap();
    assert_eq!(hi.messages.len(), 1);
    assert_eq!(c.list_resources().unwrap()[0].uri, "mem://a");
    assert_eq!(
        c.list_resource_templates().unwrap()[0].uri_template,
        "mem://t/{x}"
    );
    let read = c.read_resource("mem://t/42").unwrap();
    match &read.contents[0] {
        ResourceContents::Text { text, .. } => assert_eq!(text, "x=42"),
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        c.read_resource("mem://nope").unwrap_err(),
        ClientError::Rpc(e) if e.code.0 == -32002
    ));
}

#[derive(Default)]
struct Collect {
    progress: Vec<f64>,
}

impl Handler for Collect {
    fn notification(&mut self, method: &str, params: Option<&Value>) {
        if method == "notifications/progress" {
            if let Some(p) = params
                .and_then(|p| p.get("progress"))
                .and_then(Value::as_f64)
            {
                self.progress.push(p);
            }
        }
    }
}

#[test]
fn notifications_during_a_call_reach_the_handler() {
    let mut c = connect(builder().build().unwrap(), config(), Collect::default());
    let mut params = Value::object();
    params.insert("name", "progress");
    let mut meta = Value::object();
    meta.insert("progressToken", "tok");
    params.insert("_meta", meta);
    let done = c.call("tools/call", Some(params)).unwrap();
    assert_eq!(done["content"][0]["text"].as_str(), Some("done"));
    assert_eq!(c.handler().progress, [1.0, 2.0, 3.0]);
}

#[test]
fn a_timeout_is_reported_and_a_late_answer_does_not_confuse_the_next_call() {
    let (server_in, client_out) = std::io::pipe().unwrap();
    let (client_in, server_out) = std::io::pipe().unwrap();
    let server = builder().build().unwrap();
    std::thread::spawn(move || {
        let _ = serve_lines(
            Arc::new(server),
            BufReader::new(server_in),
            server_out,
            StdioConfig::default(),
        );
    });
    let t = StdioTransport::new(client_in, client_out);
    let mut c = Client::connect(t, config(), NoHandler, Duration::from_millis(150)).unwrap();
    assert!(matches!(
        c.call_tool("slow", None).unwrap_err(),
        ClientError::Timeout
    ));
    // The slow tool is cancelled and its answer, if any, is dropped; the next
    // call gets its own answer.
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(first_text(&c.call_tool("add", args(3, 4)).unwrap()), "7");
}

struct Asker {
    accept: bool,
    asked: Vec<String>,
}

impl Handler for Asker {
    fn elicit(&mut self, params: &ElicitParams) -> ElicitResult {
        if let ElicitParams::Form { message, .. } = params {
            self.asked.push(message.clone());
        }
        let mut content = Value::object();
        content.insert("n", "Ann");
        ElicitResult {
            action: if self.accept {
                ElicitAction::Accept
            } else {
                ElicitAction::Decline
            },
            content: self.accept.then_some(content),
            meta: None,
        }
    }
}

fn eliciting() -> ClientConfig {
    let mut c = config();
    let mut caps = Value::object();
    let mut e = Value::object();
    e.insert("form", Value::object());
    caps.insert("elicitation", e);
    c.capabilities = rusty_mcp_client_native::proto::ClientCapabilities::from_value_for_test(&caps);
    c
}

#[test]
fn input_required_is_driven_to_an_answer() {
    let mut c = connect(
        builder().build().unwrap(),
        eliciting(),
        Asker {
            accept: true,
            asked: vec![],
        },
    );
    let r = c.call_tool("confirm", None).unwrap();
    assert_eq!(first_text(&r), "pending accepted");
    assert_eq!(c.handler().asked, ["Proceed?"]);
    let mut d = connect(
        builder().build().unwrap(),
        eliciting(),
        Asker {
            accept: false,
            asked: vec![],
        },
    );
    assert_eq!(
        first_text(&d.call_tool("confirm", None).unwrap()),
        "pending declined"
    );
}

#[test]
fn a_server_that_never_stops_asking_is_cut_off() {
    let mut c = connect(
        builder().build().unwrap(),
        eliciting(),
        Asker {
            accept: true,
            asked: vec![],
        },
    );
    let err = c.call_tool("nag", None).unwrap_err();
    assert!(
        matches!(err, ClientError::TooManyRounds(_) | ClientError::Rpc(_)),
        "{err}"
    );
    assert!(c.handler().asked.len() <= 8);
}

#[test]
fn a_task_is_polled_to_its_result_and_can_ask_questions() {
    let cfg = with_tasks(eliciting());
    let mut c = connect(
        builder().build().unwrap(),
        cfg,
        Asker {
            accept: true,
            asked: vec![],
        },
    );
    assert_eq!(first_text(&c.call_tool("job", None).unwrap()), "job done");
    assert_eq!(
        first_text(&c.call_tool("ask_job", None).unwrap()),
        "hello Ann"
    );
    assert_eq!(c.handler().asked, ["Name?"]);
}

#[test]
fn a_client_without_the_extension_gets_a_task_tool_inline() {
    let mut c = connect(builder().build().unwrap(), config(), NoHandler);
    assert_eq!(first_text(&c.call_tool("job", None).unwrap()), "job done");
}

#[test]
fn a_server_that_goes_away_is_closed_not_hung() {
    let (server_in, client_out) = std::io::pipe().unwrap();
    let (client_in, server_out) = std::io::pipe().unwrap();
    drop(server_out); // the server side of the read pipe is gone at once
    drop(server_in);
    let t = StdioTransport::new(client_in, client_out);
    let err = Client::connect(t, config(), NoHandler, SECS).err().unwrap();
    assert!(
        matches!(err, ClientError::Closed | ClientError::Io(_)),
        "{err}"
    );
}
