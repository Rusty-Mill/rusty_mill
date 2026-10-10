#![allow(clippy::unwrap_used)]
//! The client against `rusty_mcp_server`, in one process: the two ends talk
//! over pipes, so every call crosses the real newline-delimited JSON framing.

mod common;

use common::{builder, eliciting, first_text, with_tasks, Asker, SECS};
use rusty_mcp_client_native::json::Value;
use rusty_mcp_client_native::proto::{ProtocolVersion, ResourceContents};
use rusty_mcp_client_native::{
    Client, ClientConfig, ClientError, Handler, NoHandler, StdioTransport,
};
use rusty_mcp_server::{serve_lines, Server, StdioConfig};
use std::io::BufReader;
use std::sync::Arc;
use std::time::Duration;

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
    let mut cfg = config();
    cfg.call_timeout = Duration::from_millis(150);
    let mut c = Client::connect(t, cfg, NoHandler, SECS).unwrap();
    assert!(matches!(
        c.call_tool("slow", None).unwrap_err(),
        ClientError::Timeout
    ));
    // The slow tool is cancelled and its answer, if any, is dropped; the next
    // call gets its own answer.
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(first_text(&c.call_tool("add", args(3, 4)).unwrap()), "7");
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
    let cfg = with_tasks();
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
