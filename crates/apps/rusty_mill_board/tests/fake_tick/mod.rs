//! An in-memory rusty_tick for `TickStore` tests: the routes, preconditions and
//! version rules the adapter relies on, and nothing else.

// Each test crate that includes this module uses part of it.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use rusty_mill_board::domain::BoardError;
use rusty_mill_board::tick::{TickApi, TickRequest};
use serde_json::{Value, json};
use uuid::Uuid;

const INBOX: Uuid = Uuid::from_u128(0x0000_0000_0000_7000_8000_0000_0000_0001);

#[derive(Default)]
struct State {
    lists: Vec<(Uuid, String)>,
    tasks: BTreeMap<Uuid, Value>,
    docs: BTreeMap<(String, Uuid), (Value, i64)>,
    clock: i64,
}

/// A shared handle: the store owns one clone, the test keeps another to act as the owner.
#[derive(Clone)]
pub struct FakeTick(Arc<Mutex<State>>);

impl Default for FakeTick {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(State {
            lists: vec![(INBOX, "Inbox".into())],
            ..State::default()
        })))
    }
}

fn version(task: &Value) -> u64 {
    task["etag"]
        .as_str()
        .and_then(|e| e.parse().ok())
        .unwrap_or(0)
}

fn bump(task: &mut Value, now: i64) {
    task["etag"] = json!((version(task) + 1).to_string());
    task["updatedMs"] = json!(now);
}

fn lower_tags(tags: &Value) -> Value {
    let mut out: Vec<String> = Vec::new();
    for tag in tags
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        let tag = tag.trim().to_lowercase();
        if !tag.is_empty() && !out.contains(&tag) {
            out.push(tag);
        }
    }
    json!(out)
}

impl FakeTick {
    /// The owner edits a task in rusty_tick: `edit` changes it, and its version moves.
    pub fn owner_edit(&self, id: Uuid, edit: impl FnOnce(&mut Value)) {
        let mut state = self.0.lock().expect("lock");
        state.clock += 1;
        let now = state.clock;
        let task = state.tasks.get_mut(&id).expect("task");
        edit(task);
        bump(task, now);
    }

    /// The task as rusty_tick holds it.
    pub fn task(&self, id: Uuid) -> Value {
        self.0.lock().expect("lock").tasks[&id].clone()
    }

    /// A stored document's body.
    pub fn doc(&self, kind: &str, id: Uuid) -> Option<Value> {
        let state = self.0.lock().expect("lock");
        state
            .docs
            .get(&(kind.to_owned(), id))
            .map(|(b, _)| b.clone())
    }

    /// Every stored document of `kind`.
    pub fn docs(&self, kind: &str) -> Vec<Value> {
        let state = self.0.lock().expect("lock");
        state
            .docs
            .iter()
            .filter(|((k, _), _)| k == kind)
            .map(|(_, (b, _))| b.clone())
            .collect()
    }

    fn route(state: &mut State, r: &TickRequest<'_>) -> (u16, Value) {
        state.clock += 1;
        let now = state.clock;
        let segments: Vec<&str> = r.path.trim_start_matches('/').split('/').collect();
        let body = r.body.clone().unwrap_or(Value::Null);
        match (r.method, segments.as_slice()) {
            ("GET", ["health"]) => (200, json!({ "ok": true })),
            ("GET", ["lists"]) => {
                let lists: Vec<Value> = state
                    .lists
                    .iter()
                    .map(|(id, name)| json!({ "id": id, "name": name }))
                    .collect();
                (200, json!({ "lists": lists }))
            }
            ("POST", ["lists"]) => {
                let id = Uuid::now_v7();
                let name = body["name"].as_str().unwrap_or_default().to_owned();
                state.lists.push((id, name.clone()));
                (201, json!({ "id": id, "name": name }))
            }
            ("POST", ["tasks"]) => {
                let id: Uuid =
                    serde_json::from_value(body["id"].clone()).unwrap_or_else(|_| Uuid::now_v7());
                if state.tasks.contains_key(&id) {
                    return (409, json!({ "error": { "message": "task exists" } }));
                }
                let task = json!({
                    "id": id,
                    "listId": body.get("listId").cloned().unwrap_or(json!(INBOX)),
                    "title": body["title"],
                    "notes": body.get("notes").cloned().unwrap_or(json!("")),
                    "status": "open",
                    "priority": body.get("priority").cloned().unwrap_or(json!(0)),
                    "tags": lower_tags(&body["tags"]),
                    "sortOrder": body.get("sortOrder").cloned().unwrap_or(json!(0)),
                    "createdMs": now,
                    "updatedMs": now,
                    "deletedMs": null,
                    "etag": "1",
                });
                state.tasks.insert(id, task.clone());
                (201, task)
            }
            ("GET", ["tasks", id]) => match state.tasks.get(&parse(id)) {
                Some(task) => (200, task.clone()),
                None => (404, json!({ "error": { "message": "task not found" } })),
            },
            ("PATCH", ["tasks", id]) => {
                let Some(task) = state.tasks.get_mut(&parse(id)) else {
                    return (404, Value::Null);
                };
                if r.if_match.as_deref().is_some_and(|m| m != task["etag"]) {
                    return (412, json!({ "current": task }));
                }
                for (key, value) in body.as_object().into_iter().flatten() {
                    let value = if key == "tags" {
                        lower_tags(value)
                    } else {
                        value.clone()
                    };
                    task[key.as_str()] = value;
                }
                bump(task, now);
                (200, task.clone())
            }
            ("GET", ["tags", tag, "tasks"]) => {
                let tasks: Vec<Value> = state
                    .tasks
                    .values()
                    .filter(|t| {
                        t["tags"]
                            .as_array()
                            .is_some_and(|a| a.contains(&json!(tag)))
                    })
                    .cloned()
                    .collect();
                (200, json!({ "tasks": tasks }))
            }
            ("GET", ["docs", kind]) => {
                let docs: Vec<Value> = state
                    .docs
                    .iter()
                    .filter(|((k, _), _)| k == kind)
                    .map(|((_, id), (b, etag))| json!({ "id": id, "body": b, "etag": etag.to_string() }))
                    .collect();
                (200, json!({ "docs": docs }))
            }
            ("PUT", ["docs", kind, id]) => {
                let id = parse(id);
                if *kind == "board" {
                    let Some(task) = state.tasks.get(&id) else {
                        return (404, Value::Null);
                    };
                    let Some(presented) = r.if_task_match.as_deref() else {
                        return (400, Value::Null);
                    };
                    if presented != task["etag"] {
                        return (412, json!({ "current": task }));
                    }
                }
                let key = ((*kind).to_owned(), id);
                let current = state.docs.get(&key).map(|(_, etag)| *etag);
                if r.if_absent && current.is_some() {
                    return (412, Value::Null);
                }
                let etag = current.map(|e| e.to_string());
                if r.if_match.is_some() && r.if_match != etag {
                    return (412, Value::Null);
                }
                let etag = current.map_or(now, |e| now.max(e + 1));
                state.docs.insert(key, (body.clone(), etag));
                (
                    200,
                    json!({ "id": id, "body": body, "etag": etag.to_string() }),
                )
            }
            _ => (404, json!({ "error": { "message": "no such route" } })),
        }
    }
}

fn parse(id: &str) -> Uuid {
    Uuid::parse_str(id).unwrap_or_else(|_| Uuid::nil())
}

impl TickApi for FakeTick {
    fn send(&self, request: &TickRequest<'_>) -> Result<(u16, Value), BoardError> {
        let mut state = self.0.lock().expect("lock");
        Ok(Self::route(&mut state, request))
    }
}
