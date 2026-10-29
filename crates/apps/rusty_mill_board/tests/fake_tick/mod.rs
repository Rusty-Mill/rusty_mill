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
    /// Answer the next request whose method and path start like this with a 500, unserved.
    fail_next: Option<(String, String)>,
    /// After each of the next `n` requests whose method and path start like this is
    /// served, another writer puts this board document on the task, moving its version.
    race: Option<Race>,
}

struct Race {
    method: String,
    path: String,
    left: usize,
    task: Uuid,
    doc: Value,
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
    stamp_completed(task, now);
}

/// As rusty_tick does: a task is stamped once when it closes (done or won't-do), cleared on reopen.
fn stamp_completed(task: &mut Value, now: i64) {
    if task["status"] == "open" {
        task["completedMs"] = Value::Null;
    } else if task["completedMs"].is_null() {
        task["completedMs"] = json!(now);
    }
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

    /// The owner adds a comment in rusty_tick, which does not move the task's version.
    pub fn owner_comment(&self, task: Uuid, text: &str) {
        let mut state = self.0.lock().expect("lock");
        let body = json!({ "v": 1, "taskId": task, "text": text, "createdMs": 0 });
        state
            .docs
            .insert(("comment".into(), Uuid::now_v7()), (body, 1));
    }

    /// Fail the next `method` request whose path starts with `path`, before it is served.
    pub fn fail_next(&self, method: &str, path: &str) {
        self.0.lock().expect("lock").fail_next = Some((method.into(), path.into()));
    }

    /// After each of the next `times` `method` requests whose path starts with `path` is
    /// served, another writer puts `doc` as `task`'s board document, moving its version as
    /// a fenced write does: a write landing between two reads of one card.
    pub fn race_after(&self, method: &str, path: &str, times: usize, task: Uuid, doc: Value) {
        self.0.lock().expect("lock").race = Some(Race {
            method: method.into(),
            path: path.into(),
            left: times,
            task,
            doc,
        });
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
                let mut task = json!({
                    "id": id,
                    "listId": body.get("listId").cloned().unwrap_or(json!(INBOX)),
                    "title": body["title"],
                    "notes": body.get("notes").cloned().unwrap_or(json!("")),
                    "status": body.get("status").cloned().unwrap_or(json!("open")),
                    "priority": body.get("priority").cloned().unwrap_or(json!(0)),
                    "tags": lower_tags(&body["tags"]),
                    "sortOrder": body.get("sortOrder").cloned().unwrap_or(json!(0)),
                    "createdMs": now,
                    "updatedMs": now,
                    "deletedMs": null,
                    "etag": "1",
                });
                stamp_completed(&mut task, now);
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
                // The task the write is fenced to; rusty_tick moves its version with the write.
                let fenced = match (*kind, r.if_task_match.as_deref()) {
                    ("board", None) => return (400, Value::Null),
                    ("board", Some(v)) => Some((id, v)),
                    ("comment", Some(v)) => Some((parse(body["taskId"].as_str().unwrap_or("")), v)),
                    (_, Some(_)) => return (400, Value::Null),
                    (_, None) => None,
                };
                if let Some((task, presented)) = fenced {
                    let Some(task) = state.tasks.get_mut(&task) else {
                        return (404, Value::Null);
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
                if let Some((task, _)) = fenced {
                    bump(state.tasks.get_mut(&task).expect("task"), now);
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

impl FakeTick {
    fn run_race(state: &mut State, request: &TickRequest<'_>) {
        let Some(race) = state.race.as_mut() else {
            return;
        };
        if race.method != request.method || !request.path.starts_with(race.path.as_str()) {
            return;
        }
        race.left -= 1;
        let (task, doc, done) = (race.task, race.doc.clone(), race.left == 0);
        if done {
            state.race = None;
        }
        state.clock += 1;
        let now = state.clock;
        state.docs.insert(("board".into(), task), (doc, now));
        bump(state.tasks.get_mut(&task).expect("task"), now);
    }
}

impl TickApi for FakeTick {
    fn send(&self, request: &TickRequest<'_>) -> Result<(u16, Value), BoardError> {
        let mut state = self.0.lock().expect("lock");
        let failing = state
            .fail_next
            .as_ref()
            .is_some_and(|(m, p)| m == request.method && request.path.starts_with(p.as_str()));
        if failing {
            state.fail_next = None;
            return Ok((500, json!({ "error": { "message": "injected" } })));
        }
        let served = Self::route(&mut state, request);
        Self::run_race(&mut state, request);
        Ok(served)
    }
}
