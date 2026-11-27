#![allow(clippy::unwrap_used)]
//! `PromptSource` and `ResourceSource`: prompts and resources known only at
//! request time, listed after the registered ones and served when nothing
//! registered matches.

use rusty_json::Value;
use rusty_mcp_proto::{
    ErrorData, GetPromptParams, GetPromptResult, Message, Prompt, ReadResourceParams,
    ReadResourceResult, Resource, ResourceContents, ResourceTemplate, Wire,
};
use rusty_mcp_server::{
    BuildError, CallContext, ChangeKinds, Connection, Notifier, PromptSource, ResourceSource,
    Server,
};
use std::sync::{Arc, Mutex};

const V: &str = r#""io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}"#;

struct Mute;
impl Notifier for Mute {
    fn notify(&self, _: Message) {}
}

#[derive(Clone, Default)]
struct Names(Arc<Mutex<Vec<String>>>);

impl PromptSource for Names {
    fn prompts(&self, _ctx: &CallContext) -> Result<Vec<Prompt>, ErrorData> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .iter()
            .map(|n| Prompt::new(n.as_str()))
            .collect())
    }

    fn get(
        &self,
        _ctx: &CallContext,
        params: &GetPromptParams,
    ) -> Option<Result<GetPromptResult, ErrorData>> {
        let held = self.0.lock().unwrap().contains(&params.name);
        held.then(|| {
            Ok(GetPromptResult {
                description: Some(format!("source:{}", params.name)),
                ..GetPromptResult::default()
            })
        })
    }
}

impl ResourceSource for Names {
    fn resources(&self, _ctx: &CallContext) -> Result<Vec<Resource>, ErrorData> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .iter()
            .map(|n| Resource::new(format!("mem://{n}"), n.as_str()))
            .collect())
    }

    fn templates(&self, _ctx: &CallContext) -> Result<Vec<ResourceTemplate>, ErrorData> {
        Ok(vec![ResourceTemplate::new("mem://t/{id}", "t")])
    }

    fn read(
        &self,
        _ctx: &CallContext,
        params: &ReadResourceParams,
    ) -> Option<Result<ReadResourceResult, ErrorData>> {
        let held = self
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|n| params.uri == format!("mem://{n}"));
        held.then(|| {
            Ok(ReadResourceResult {
                contents: vec![ResourceContents::Text {
                    uri: params.uri.clone(),
                    mime_type: None,
                    text: "from source".to_owned(),
                    meta: None,
                }],
                ..ReadResourceResult::default()
            })
        })
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

fn meta() -> String {
    format!(r#"{{"_meta":{{{V}}}}}"#)
}

fn names(reply: &Value, key: &str, field: &str) -> Vec<String> {
    reply["result"][key]
        .as_array()
        .unwrap_or_else(|| panic!("no {key}: {}", reply.to_json_string()))
        .iter()
        .map(|e| e[field].as_str().unwrap().to_owned())
        .collect()
}

fn with_sources(held: &[&str]) -> (Arc<Connection>, Names) {
    let names = Names::default();
    *names.0.lock().unwrap() = held.iter().map(|s| (*s).to_owned()).collect();
    let server = Server::builder("s", "1")
        .prompt(Prompt::new("fixed"), |_c, _p| {
            Ok(GetPromptResult {
                description: Some("registered".to_owned()),
                ..GetPromptResult::default()
            })
        })
        .prompt_source(names.clone())
        .resource_source(names.clone())
        .build()
        .unwrap();
    (conn(server), names)
}

#[test]
fn prompts_are_listed_after_the_registered_ones_and_registered_win_a_clash() {
    let (c, _) = with_sources(&["up", "fixed"]);
    let r = send(&c, "prompts/list", &meta());
    assert_eq!(names(&r, "prompts", "name"), ["fixed", "up"]);
}

#[test]
fn a_prompt_nobody_registered_is_asked_of_the_source() {
    let (c, names) = with_sources(&["up"]);
    let p = |n: &str| format!(r#"{{"name":"{n}","_meta":{{{V}}}}}"#);
    let r = send(&c, "prompts/get", &p("up"));
    assert_eq!(r["result"]["description"].as_str(), Some("source:up"));
    let r = send(&c, "prompts/get", &p("fixed"));
    assert_eq!(r["result"]["description"].as_str(), Some("registered"));
    let r = send(&c, "prompts/get", &p("nope"));
    assert_eq!(r["error"]["code"].as_i64(), Some(-32602));
    // The source is asked afresh each time.
    names.0.lock().unwrap().push("later".to_owned());
    let r = send(&c, "prompts/get", &p("later"));
    assert_eq!(r["result"]["description"].as_str(), Some("source:later"));
}

#[test]
fn resources_and_templates_come_from_the_source_and_reads_fall_through() {
    let (c, _) = with_sources(&["a"]);
    let r = send(&c, "resources/list", &meta());
    assert_eq!(names(&r, "resources", "uri"), ["mem://a"]);
    let r = send(&c, "resources/templates/list", &meta());
    assert_eq!(
        names(&r, "resourceTemplates", "uriTemplate"),
        ["mem://t/{id}"]
    );
    let read = |uri: &str| {
        send(
            &c,
            "resources/read",
            &format!(r#"{{"uri":"{uri}","_meta":{{{V}}}}}"#),
        )
    };
    let r = read("mem://a");
    assert_eq!(
        r["result"]["contents"][0]["text"].as_str(),
        Some("from source")
    );
    let r = read("mem://missing");
    assert_eq!(r["error"]["code"].as_i64(), Some(-32002));
}

#[test]
fn a_server_with_only_a_source_advertises_the_capability() {
    let (c, _) = with_sources(&[]);
    let r = send(&c, "server/discover", &format!(r#"{{"_meta":{{{V}}}}}"#));
    let caps = &r["result"]["capabilities"];
    assert!(caps.get("prompts").is_some(), "{}", r.to_json_string());
    assert!(caps.get("resources").is_some(), "{}", r.to_json_string());
}

#[test]
fn a_server_without_either_still_refuses_the_methods() {
    let c = conn(Server::builder("s", "1").build().unwrap());
    for m in ["prompts/list", "resources/list", "resources/templates/list"] {
        assert!(send(&c, m, &meta())["error"].get("code").is_some(), "{m}");
    }
}

#[test]
fn announcing_changes_needs_a_source_or_a_registration() {
    let changes = rusty_mcp_server::ChangeBroadcaster::new();
    let kinds = ChangeKinds {
        prompts_list: true,
        ..ChangeKinds::default()
    };
    let without = Server::builder("s", "1")
        .notify_changes(&changes, kinds)
        .build();
    assert!(matches!(
        without,
        Err(BuildError::ChangesWithoutFeature("prompts"))
    ));
    let with = Server::builder("s", "1")
        .prompt_source(Names::default())
        .notify_changes(&changes, kinds)
        .build();
    assert!(with.is_ok());
}

/// Refuses every listing unless the caller sent `x-pass`.
struct Gate;
impl PromptSource for Gate {
    fn prompts(&self, ctx: &CallContext) -> Result<Vec<Prompt>, ErrorData> {
        if ctx.caller().header("x-pass").is_some() {
            Ok(vec![Prompt::new("ok")])
        } else {
            Err(ErrorData::new(
                rusty_mcp_proto::ErrorCode::INVALID_REQUEST,
                "no pass",
            ))
        }
    }

    fn get(
        &self,
        _ctx: &CallContext,
        _params: &GetPromptParams,
    ) -> Option<Result<GetPromptResult, ErrorData>> {
        None
    }
}

#[test]
fn a_source_can_refuse_a_listing_and_sees_who_asks() {
    let server = || {
        Server::builder("s", "1")
            .prompt_source(Gate)
            .build()
            .unwrap()
    };
    let denied = send(&conn(server()), "prompts/list", &meta());
    assert_eq!(denied["error"]["message"].as_str(), Some("no pass"));
    let caller = rusty_mcp_server::Caller {
        headers: vec![("x-pass".to_owned(), "1".to_owned())],
        principal: None,
    };
    let c = Arc::new(Connection::new(Arc::new(server()), Arc::new(Mute)).with_caller(caller));
    let allowed = send(&c, "prompts/list", &meta());
    assert_eq!(names(&allowed, "prompts", "name"), ["ok"]);
}
