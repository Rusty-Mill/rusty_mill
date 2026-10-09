#![allow(clippy::unwrap_used)]
//! `ToolSource`: tools known only at request time, listed after the
//! registered ones and called when no registered tool has the name.

use rusty_json::Value;
use rusty_mcp_proto::{
    CallToolParams, CallToolResult, ContentBlock, ErrorData, Message, Tool, Wire,
};
use rusty_mcp_server::{
    BuildError, CallContext, ChangeKinds, Connection, Notifier, Server, ToolSource,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const V: &str = r#""io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}"#;

struct Mute;
impl Notifier for Mute {
    fn notify(&self, _: Message) {}
}

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

/// Offers whatever names it holds; answers with its own name.
#[derive(Clone, Default)]
struct Names(Arc<Mutex<Vec<String>>>, Arc<AtomicUsize>);

impl ToolSource for Names {
    fn tools(&self) -> Vec<Tool> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|n| Tool::new(n.as_str(), schema()))
            .collect()
    }

    fn call(
        &self,
        _ctx: &CallContext,
        call: &CallToolParams,
    ) -> Option<Result<CallToolResult, ErrorData>> {
        self.1.fetch_add(1, Ordering::SeqCst);
        let held = self.0.lock().unwrap().contains(&call.name);
        held.then(|| Ok(text(&format!("source:{}", call.name))))
    }
}

fn conn(server: Server) -> Arc<Connection> {
    Arc::new(Connection::new(Arc::new(server), Arc::new(Mute)))
}

fn send(conn: &Arc<Connection>, method: &str, params: &str) -> Value {
    let body = format!(r#"{{"jsonrpc":"2.0","id":1,"method":"{method}","params":{params}}}"#);
    let reply = conn.handle(Message::from_json(&body).unwrap()).unwrap();
    Value::from_json_str(&reply.to_json()).unwrap()
}

fn list(conn: &Arc<Connection>) -> Vec<String> {
    let r = send(conn, "tools/list", &format!(r#"{{"_meta":{{{V}}}}}"#));
    r["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("no tools: {}", r.to_json_string()))
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .collect()
}

fn call(conn: &Arc<Connection>, tool: &str) -> Value {
    send(
        conn,
        "tools/call",
        &format!(r#"{{"name":"{tool}","_meta":{{{V}}}}}"#),
    )
}

fn names(list: &[&str]) -> Names {
    let n = Names::default();
    *n.0.lock().unwrap() = list.iter().map(|s| (*s).to_owned()).collect();
    n
}

#[test]
fn a_server_with_only_a_source_offers_tools() {
    let c = conn(
        Server::builder("s", "1")
            .tool_source(names(&["a", "b"]))
            .build()
            .unwrap(),
    );
    assert_eq!(list(&c), ["a", "b"]);
    let r = call(&c, "b");
    assert_eq!(
        r["result"]["content"][0]["text"].as_str(),
        Some("source:b"),
        "{}",
        r.to_json_string()
    );
}

#[test]
fn the_list_follows_the_source_between_requests() {
    let n = names(&["a"]);
    let c = conn(
        Server::builder("s", "1")
            .tool_source(n.clone())
            .build()
            .unwrap(),
    );
    assert_eq!(list(&c), ["a"]);
    n.0.lock().unwrap().push("later".to_owned());
    assert_eq!(list(&c), ["a", "later"]);
    n.0.lock().unwrap().clear();
    assert!(call(&c, "a")["error"]["code"].as_i64().is_some());
}

#[test]
fn a_registered_tool_wins_a_clash_and_the_source_is_not_asked() {
    let n = names(&["dup", "other"]);
    let c = conn(
        Server::builder("s", "1")
            .tool(Tool::new("dup", schema()), |_c, _p| Ok(text("registered")))
            .tool_source(n.clone())
            .build()
            .unwrap(),
    );
    assert_eq!(list(&c), ["dup", "other"]);
    let r = call(&c, "dup");
    assert_eq!(
        r["result"]["content"][0]["text"].as_str(),
        Some("registered")
    );
    assert_eq!(n.1.load(Ordering::SeqCst), 0);
}

#[test]
fn a_tool_nobody_has_is_invalid_params() {
    let c = conn(
        Server::builder("s", "1")
            .tool_source(names(&["a"]))
            .build()
            .unwrap(),
    );
    let r = call(&c, "nope");
    assert_eq!(
        r["error"]["code"].as_i64(),
        Some(-32602),
        "{}",
        r.to_json_string()
    );
}

#[test]
fn a_source_error_is_the_calls_error() {
    struct Broken;
    impl ToolSource for Broken {
        fn tools(&self) -> Vec<Tool> {
            vec![Tool::new("x", schema())]
        }
        fn call(
            &self,
            _: &CallContext,
            _: &CallToolParams,
        ) -> Option<Result<CallToolResult, ErrorData>> {
            Some(Err(ErrorData::new(
                rusty_mcp_proto::ErrorCode::INTERNAL_ERROR,
                "upstream down",
            )))
        }
    }
    let c = conn(
        Server::builder("s", "1")
            .tool_source(Broken)
            .build()
            .unwrap(),
    );
    let r = call(&c, "x");
    assert_eq!(r["error"]["message"].as_str(), Some("upstream down"));
}

#[test]
fn announcing_tool_changes_needs_only_a_source() {
    let b = rusty_mcp_server::ChangeBroadcaster::new();
    let ok = Server::builder("s", "1")
        .notify_changes(
            &b,
            ChangeKinds {
                tools_list: true,
                ..ChangeKinds::default()
            },
        )
        .tool_source(names(&[]))
        .build();
    assert!(ok.is_ok());
    let b = rusty_mcp_server::ChangeBroadcaster::new();
    let err = Server::builder("s", "1")
        .notify_changes(
            &b,
            ChangeKinds {
                tools_list: true,
                ..ChangeKinds::default()
            },
        )
        .build();
    assert!(matches!(
        err,
        Err(BuildError::ChangesWithoutFeature("tools"))
    ));
}
