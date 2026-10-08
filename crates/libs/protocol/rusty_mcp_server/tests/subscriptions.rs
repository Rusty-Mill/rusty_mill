#![allow(clippy::unwrap_used)]
//! `subscriptions/listen` through the stdio loop.

mod support;

use rusty_json::Value;
use rusty_mcp_proto::{
    CallToolResult, ErrorCode, Prompt, ReadResourceResult, Resource, ResourceTemplate, Tool,
};
use rusty_mcp_server::{
    serve_lines, BuildError, ChangeBroadcaster, ChangeKinds, Server, StdioConfig,
};
use std::io::BufReader;
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};
use support::{code, reply, result, run_with, Feed, Out};

const V: &str = r#""io.modelcontextprotocol/protocolVersion":"2026-07-28""#;

fn schema() -> Value {
    let mut s = Value::object();
    s.insert("type", "object");
    s
}

fn server_with(changes: &ChangeBroadcaster, kinds: ChangeKinds) -> Arc<Server> {
    Arc::new(
        Server::builder("subs", "1")
            .tool(Tool::new("t", schema()), |_c, _p| {
                Ok(CallToolResult::default())
            })
            .prompt(Prompt::new("p"), |_c, _g| Ok(Default::default()))
            .resource(Resource::new("mem://a", "a"), |_c, _r| {
                Ok(ReadResourceResult::default())
            })
            .resource_template(ResourceTemplate::new("file:///{x}", "f"), |_c, _v, _r| {
                Ok(ReadResourceResult::default())
            })
            .notify_changes(changes, kinds)
            .build()
            .unwrap(),
    )
}

fn listen(id: i64, filter: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":{id},"method":"subscriptions/listen","params":{{"notifications":{filter},"_meta":{{{V}}}}}}}"#
    )
}

/// A running stdio loop the test can talk to while it runs.
struct Session {
    input: Option<Sender<Vec<u8>>>,
    out: Out,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Session {
    fn start(server: Arc<Server>) -> Self {
        let (tx, rx) = channel();
        let out = Out::default();
        let sink = out.clone();
        let thread = std::thread::spawn(move || {
            serve_lines(
                server,
                BufReader::new(Feed(rx, Vec::new())),
                sink,
                StdioConfig::default(),
            )
            .unwrap();
        });
        Self {
            input: Some(tx),
            out,
            thread: Some(thread),
        }
    }

    fn send(&self, line: &str) {
        self.input
            .as_ref()
            .unwrap()
            .send(format!("{line}\n").into_bytes())
            .unwrap();
    }

    /// Wait until `count` messages satisfy `pred`; return them.
    fn wait(&self, count: usize, pred: impl Fn(&Value) -> bool) -> Vec<Value> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let hits: Vec<Value> = self
                .out
                .messages()
                .into_iter()
                .filter(|m| pred(m))
                .collect();
            if hits.len() >= count || Instant::now() > deadline {
                return hits;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// End the input and return how long the loop took to finish.
    fn finish(mut self) -> Duration {
        drop(self.input.take());
        let started = Instant::now();
        self.thread.take().unwrap().join().unwrap();
        started.elapsed()
    }
}

fn method(m: &Value) -> Option<&str> {
    m.get("method").and_then(Value::as_str)
}

fn sub_id(m: &Value) -> Option<i64> {
    m.get("params")?
        .get("_meta")?
        .get("io.modelcontextprotocol/subscriptionId")?
        .as_i64()
}

#[test]
fn a_listener_is_acknowledged_with_what_it_was_granted_and_hears_only_that() {
    let changes = ChangeBroadcaster::new();
    let s = Session::start(server_with(&changes, ChangeKinds::all()));
    // Unknown and duplicate URIs are dropped; prompts were not requested.
    s.send(&listen(
        5,
        r#"{"toolsListChanged":true,"resourcesListChanged":true,"resourceSubscriptions":["mem://a","file:///x","unknown://nope","mem://a"]}"#,
    ));
    let acks = s.wait(1, |m| {
        method(m) == Some("notifications/subscriptions/acknowledged")
    });
    let ack = &acks[0]["params"];
    assert_eq!(sub_id(&acks[0]), Some(5));
    let granted = &ack["notifications"];
    assert_eq!(granted["toolsListChanged"].as_bool(), Some(true));
    assert_eq!(granted["resourcesListChanged"].as_bool(), Some(true));
    let uris: Vec<&str> = granted["resourceSubscriptions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(uris, ["mem://a", "file:///x"]);
    assert!(granted.get("promptsListChanged").is_none());

    changes.tools_changed();
    changes.prompts_changed(); // not followed
    changes.resource_updated("mem://a");
    changes.resource_updated("mem://zzz"); // not followed
    changes.resource_updated("file:///x");
    changes.resources_changed();
    s.wait(4, |m| {
        m.get("method").is_some() && method(m) != Some("notifications/subscriptions/acknowledged")
    });
    std::thread::sleep(Duration::from_millis(200)); // room for anything wrongly sent
    let heard: Vec<Value> = s
        .out
        .messages()
        .into_iter()
        .filter(|m| {
            m.get("method").is_some()
                && method(m) != Some("notifications/subscriptions/acknowledged")
        })
        .collect();
    let seen: Vec<(&str, Option<&str>)> = heard
        .iter()
        .map(|m| {
            (
                method(m).unwrap(),
                m["params"].get("uri").and_then(Value::as_str),
            )
        })
        .collect();
    assert_eq!(
        seen,
        [
            ("notifications/tools/list_changed", None),
            ("notifications/resources/updated", Some("mem://a")),
            ("notifications/resources/updated", Some("file:///x")),
            ("notifications/resources/list_changed", None),
        ]
    );
    assert!(
        heard.iter().all(|m| sub_id(m) == Some(5)),
        "every notification names its subscription"
    );

    // Cancelling the listen request ends it without an answer.
    s.send(r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":5}}"#);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(changes.listeners(), 0, "the listener unsubscribed");
    changes.tools_changed();
    s.finish();
}

#[test]
fn a_cancelled_listen_gets_no_result_and_stops_forwarding() {
    let changes = ChangeBroadcaster::new();
    let s = Session::start(server_with(&changes, ChangeKinds::all()));
    s.send(&listen(1, r#"{"toolsListChanged":true}"#));
    s.wait(1, |m| {
        method(m) == Some("notifications/subscriptions/acknowledged")
    });
    s.send(r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}"#);
    let deadline = Instant::now() + Duration::from_secs(5);
    while changes.listeners() > 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(changes.listeners(), 0);
    changes.tools_changed();
    std::thread::sleep(Duration::from_millis(150));
    let out = s.out.messages();
    assert_eq!(out.len(), 1, "only the acknowledgement: {out:?}");
    s.finish();
}

#[test]
fn end_of_input_closes_open_listeners_at_once_with_their_final_result() {
    let changes = ChangeBroadcaster::new();
    let s = Session::start(server_with(&changes, ChangeKinds::all()));
    s.send(&listen(7, r#"{"toolsListChanged":true}"#));
    s.wait(1, |m| {
        method(m) == Some("notifications/subscriptions/acknowledged")
    });
    let out = s.out.clone();
    let took = s.finish();
    assert!(
        took < Duration::from_secs(2),
        "did not wait out the 10 s drain: {took:?}"
    );
    let msgs = out.messages();
    let done = reply(&msgs, 7);
    assert_eq!(result(done)["resultType"].as_str(), Some("complete"));
    assert_eq!(
        result(done)["_meta"]["io.modelcontextprotocol/subscriptionId"].as_i64(),
        Some(7)
    );
    assert_eq!(
        result(done)["_meta"]["io.modelcontextprotocol/serverInfo"]["name"].as_str(),
        Some("subs")
    );
}

#[test]
fn closing_the_broadcaster_ends_every_listener_gracefully() {
    let changes = ChangeBroadcaster::new();
    let s = Session::start(server_with(&changes, ChangeKinds::all()));
    s.send(&listen(1, r#"{"toolsListChanged":true}"#));
    s.send(&listen(2, r#"{"resourcesListChanged":true}"#));
    s.wait(2, |m| {
        method(m) == Some("notifications/subscriptions/acknowledged")
    });
    assert_eq!(changes.listeners(), 2);
    changes.close();
    let done = s.wait(2, |m| m.get("result").is_some());
    assert_eq!(done.len(), 2);
    // A listener that arrives after the close ends at once.
    s.send(&listen(3, r#"{"toolsListChanged":true}"#));
    let late = s.wait(1, |m| {
        m.get("id").and_then(Value::as_i64) == Some(3) && m.get("result").is_some()
    });
    assert_eq!(late.len(), 1);
    s.finish();
}

#[test]
fn unannounced_categories_are_never_granted() {
    let changes = ChangeBroadcaster::new();
    let kinds = ChangeKinds {
        resource_updates: true,
        ..ChangeKinds::default()
    };
    let s = Session::start(server_with(&changes, kinds));
    s.send(&listen(1, r#"{"toolsListChanged":true,"resourcesListChanged":true,"resourceSubscriptions":["mem://a"]}"#));
    let ack = &s.wait(1, |m| {
        method(m) == Some("notifications/subscriptions/acknowledged")
    })[0];
    let granted = &ack["params"]["notifications"];
    assert!(
        granted.get("toolsListChanged").is_none() && granted.get("resourcesListChanged").is_none()
    );
    assert_eq!(
        granted["resourceSubscriptions"].as_array().unwrap().len(),
        1
    );
    changes.tools_changed();
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(
        s.out.messages().len(),
        1,
        "the tools change was not announced, so not sent"
    );
    s.finish();
}

#[test]
fn listen_is_refused_where_it_cannot_work() {
    let changes = ChangeBroadcaster::new();
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"c","version":"1"}}}"#;
    // A classic connection cannot listen; the same server serves a modern one.
    let classic_listen = r#"{"jsonrpc":"2.0","id":2,"method":"subscriptions/listen","params":{"notifications":{"toolsListChanged":true}}}"#;
    let out = run_with(
        server_with(&changes, ChangeKinds::all()),
        &format!("{init}\n{classic_listen}\n"),
    );
    assert_eq!(code(reply(&out, 2)), ErrorCode::METHOD_NOT_FOUND.0);

    // A server that announces nothing has no such method.
    let plain = Arc::new(Server::builder("p", "1").build().unwrap());
    let out = run_with(
        plain,
        &format!("{}\n", listen(1, r#"{"toolsListChanged":true}"#)),
    );
    assert_eq!(code(reply(&out, 1)), ErrorCode::METHOD_NOT_FOUND.0);

    // Malformed parameters.
    let no_filter = format!(
        r#"{{"jsonrpc":"2.0","id":3,"method":"subscriptions/listen","params":{{"_meta":{{{V}}}}}}}"#
    );
    let bad_filter = listen(4, r#"{"resourceSubscriptions":"mem://a"}"#);
    let out = run_with(
        server_with(&changes, ChangeKinds::all()),
        &format!("{no_filter}\n{bad_filter}\n"),
    );
    assert_eq!(code(reply(&out, 3)), ErrorCode::INVALID_PARAMS.0);
    assert_eq!(code(reply(&out, 4)), ErrorCode::INVALID_PARAMS.0);
}

#[test]
fn capabilities_advertise_exactly_what_is_announced() {
    let changes = ChangeBroadcaster::new();
    let discover = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"server/discover","params":{{"_meta":{{{V}}}}}}}"#
    );
    let caps = |kinds| {
        let out = run_with(server_with(&changes, kinds), &format!("{discover}\n"));
        result(reply(&out, 1))["capabilities"].clone()
    };
    let all = caps(ChangeKinds::all());
    assert_eq!(all["tools"]["listChanged"].as_bool(), Some(true));
    assert_eq!(all["resources"]["listChanged"].as_bool(), Some(true));
    assert_eq!(all["resources"]["subscribe"].as_bool(), Some(true));
    let some = caps(ChangeKinds {
        resource_updates: true,
        ..ChangeKinds::default()
    });
    assert!(some["tools"].get("listChanged").is_none());
    assert_eq!(some["resources"]["subscribe"].as_bool(), Some(true));
    assert!(some["resources"].get("listChanged").is_none());
    assert!(caps(ChangeKinds::default())["resources"]
        .get("subscribe")
        .is_none());
}

#[test]
fn announcing_changes_for_a_feature_the_server_lacks_is_a_build_error() {
    let changes = ChangeBroadcaster::new();
    let build = |kinds: ChangeKinds| {
        Server::builder("x", "1")
            .tool(Tool::new("t", schema()), |_c, _p| {
                Ok(CallToolResult::default())
            })
            .notify_changes(&changes, kinds)
            .build()
            .err()
    };
    assert_eq!(
        build(ChangeKinds {
            prompts_list: true,
            ..ChangeKinds::default()
        }),
        Some(BuildError::ChangesWithoutFeature("prompts"))
    );
    assert_eq!(
        build(ChangeKinds {
            resource_updates: true,
            ..ChangeKinds::default()
        }),
        Some(BuildError::ChangesWithoutFeature("resources"))
    );
    assert_eq!(
        build(ChangeKinds {
            resources_list: true,
            ..ChangeKinds::default()
        }),
        Some(BuildError::ChangesWithoutFeature("resources"))
    );
    assert_eq!(
        build(ChangeKinds {
            tools_list: true,
            ..ChangeKinds::default()
        }),
        None
    );
    let with_prompt = Server::builder("x", "1")
        .prompt(Prompt::new("p"), |_c, _g| Ok(Default::default()))
        .notify_changes(
            &changes,
            ChangeKinds {
                prompts_list: true,
                ..ChangeKinds::default()
            },
        )
        .build();
    assert!(with_prompt.is_ok());
}
