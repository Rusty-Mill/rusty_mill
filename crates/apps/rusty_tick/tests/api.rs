//! The HTTP API through the sans-IO router: no sockets, a fixed clock.

use rusty_http::Method;
use rusty_json::Value;
use rusty_tick::api::Request;
use rusty_tick::backend::Backend;
use std::path::Path;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

const TOKEN: &str = "test-token-0123456789";
/// 2026-09-29T12:00:00Z
const NOW: i64 = 1_790_596_800_000;
const HOUR: i64 = 3_600_000;
const DAY: i64 = 24 * HOUR;

struct Harness {
    backend: Backend,
    clock: Arc<AtomicI64>,
}

impl Harness {
    fn open(dir: &Path) -> Self {
        let clock = Arc::new(AtomicI64::new(NOW));
        let c = Arc::clone(&clock);
        let clock_fn = Box::new(move || c.load(Ordering::SeqCst));
        Self {
            backend: Backend::single(dir, TOKEN.into(), clock_fn).unwrap(),
            clock,
        }
    }

    fn send(
        &mut self,
        method: Method,
        target: &str,
        body: &str,
        auth: Option<&str>,
    ) -> (u16, Value) {
        let header = auth.map(|t| format!("Bearer {t}"));
        let request = Request {
            method: &method,
            target,
            authorization: header.as_deref(),
            body: body.as_bytes(),
        };
        let response = self.backend.handle(&request);
        let value = if response.body.is_empty() {
            Value::Null
        } else {
            rusty_json::from_slice(&response.body).expect("responses are JSON")
        };
        (response.status.as_u16(), value)
    }

    fn call(&mut self, method: Method, target: &str, body: &str) -> (u16, Value) {
        self.send(method, target, body, Some(TOKEN))
    }

    fn list(&mut self, name: &str) -> String {
        let (status, v) = self.call(
            Method::Post,
            "/api/v1/lists",
            &format!(r#"{{"name":"{name}"}}"#),
        );
        assert_eq!(status, 201, "{v:?}");
        v["id"].as_str().unwrap().to_string()
    }

    fn task(&mut self, list: &str, extra: &str) -> Value {
        let body = format!(r#"{{"listId":"{list}",{extra}}}"#);
        let (status, v) = self.call(Method::Post, "/api/v1/tasks", &body);
        assert_eq!(status, 201, "{v:?}");
        v
    }
}

fn titles(v: &Value) -> Vec<String> {
    v["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["title"].as_str().unwrap().to_string())
        .collect()
}

fn harness() -> (tempfile::TempDir, Harness) {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::open(dir.path());
    (dir, h)
}

#[test]
fn health_is_open_but_everything_else_needs_the_token() {
    let (_d, mut h) = harness();
    assert_eq!(h.send(Method::Get, "/health", "", None).0, 200);
    assert_eq!(h.send(Method::Get, "/api/v1/lists", "", None).0, 401);
    assert_eq!(
        h.send(
            Method::Get,
            "/api/v1/lists",
            "",
            Some("wrong-token-0123456789")
        )
        .0,
        401
    );
    assert_eq!(h.send(Method::Get, "/api/v1/lists", "", Some(TOKEN)).0, 200);
    assert!(
        Backend::single(Path::new("unused"), "short".into(), Box::new(|| 0)).is_err(),
        "a guessable token is refused"
    );
}

#[test]
fn list_lifecycle_and_cascade() {
    let (_d, mut h) = harness();
    let list = h.list("Inbox");
    let (_, v) = h.call(Method::Get, "/api/v1/lists", "");
    assert_eq!(v["lists"][0]["name"], "Inbox");

    let (s, v) = h.call(
        Method::Patch,
        &format!("/api/v1/lists/{list}"),
        r#"{"name":"Work","archived":true}"#,
    );
    assert_eq!(
        (s, v["name"].as_str(), v["archived"].as_bool()),
        (200, Some("Work"), Some(true))
    );

    let task = h.task(&list, r#""title":"a""#);
    let task_id = task["id"].as_str().unwrap().to_string();
    assert_eq!(
        h.call(Method::Delete, &format!("/api/v1/lists/{list}"), "")
            .0,
        204
    );
    assert_eq!(
        h.call(Method::Get, &format!("/api/v1/lists/{list}"), "").0,
        404
    );
    assert_eq!(
        h.call(Method::Get, &format!("/api/v1/tasks/{task_id}"), "")
            .0,
        404,
        "tasks go with their list"
    );
}

#[test]
fn task_input_is_validated_at_the_boundary() {
    let (_d, mut h) = harness();
    let list = h.list("L");
    let post = |h: &mut Harness, body: String| h.call(Method::Post, "/api/v1/tasks", &body).0;
    assert_eq!(
        post(&mut h, format!(r#"{{"listId":"{list}","title":"  "}}"#)),
        422,
        "blank title"
    );
    assert_eq!(
        post(
            &mut h,
            format!(r#"{{"listId":"{list}","title":"x","priority":2}}"#)
        ),
        422,
        "priority off the scale"
    );
    assert_eq!(
        post(
            &mut h,
            format!(r#"{{"listId":"{list}","title":"x","bogus":1}}"#)
        ),
        400,
        "unknown field"
    );
    assert_eq!(
        post(&mut h, r#"{"listId":"nope","title":"x"}"#.into()),
        400,
        "malformed id"
    );
    assert_eq!(
        post(
            &mut h,
            format!(r#"{{"listId":"{}","title":"x"}}"#, uuid::Uuid::now_v7())
        ),
        404,
        "no such list"
    );
    assert_eq!(post(&mut h, "not json".into()), 400);
    let long = "x".repeat(501);
    assert_eq!(
        post(&mut h, format!(r#"{{"listId":"{list}","title":"{long}"}}"#)),
        422,
        "title too long"
    );
    assert_eq!(
        post(
            &mut h,
            format!(r#"{{"listId":"{list}","title":"x","dueMs":{}}}"#, i64::MAX)
        ),
        422,
        "sentinel due date"
    );
}

#[test]
fn patch_distinguishes_absent_from_null() {
    let (_d, mut h) = harness();
    let list = h.list("L");
    let t = h.task(&list, r#""title":"a","dueMs":1000,"tags":["x"]"#);
    let path = format!("/api/v1/tasks/{}", t["id"].as_str().unwrap());

    let (_, v) = h.call(Method::Patch, &path, r#"{"title":"b"}"#);
    assert_eq!(
        (v["title"].as_str(), v["dueMs"].as_i64()),
        (Some("b"), Some(1000)),
        "absent leaves dueMs alone"
    );
    let (_, v) = h.call(Method::Patch, &path, r#"{"dueMs":null}"#);
    assert!(v["dueMs"].is_null(), "null clears it");
    let (_, v) = h.call(Method::Patch, &path, r#"{"tags":[" a ","a","b"]}"#);
    assert_eq!(v["tags"], rusty_json::json!(["a", "b"]));
}

#[test]
fn complete_reopen_and_status_filter() {
    let (_d, mut h) = harness();
    let list = h.list("L");
    let a = h.task(&list, r#""title":"a""#);
    h.task(&list, r#""title":"b""#);
    let a_id = a["id"].as_str().unwrap();

    let (_, v) = h.call(Method::Post, &format!("/api/v1/tasks/{a_id}/complete"), "");
    assert_eq!(v["status"], "done");
    let (_, open) = h.call(
        Method::Get,
        &format!("/api/v1/lists/{list}/tasks?status=open"),
        "",
    );
    assert_eq!(titles(&open), ["b"]);
    let (_, done) = h.call(
        Method::Get,
        &format!("/api/v1/lists/{list}/tasks?status=done"),
        "",
    );
    assert_eq!(titles(&done), ["a"]);
    h.call(Method::Post, &format!("/api/v1/tasks/{a_id}/reopen"), "");
    let (_, open) = h.call(
        Method::Get,
        &format!("/api/v1/lists/{list}/tasks?status=open"),
        "",
    );
    assert_eq!(titles(&open), ["a", "b"]);
    assert_eq!(
        h.call(
            Method::Get,
            &format!("/api/v1/lists/{list}/tasks?status=bogus"),
            ""
        )
        .0,
        400
    );
}

#[test]
fn manual_order_and_reorder() {
    let (_d, mut h) = harness();
    let list = h.list("L");
    let [_a, _b, c] = ["a", "b", "c"].map(|t| h.task(&list, &format!(r#""title":"{t}""#)));
    let (_, v) = h.call(Method::Get, &format!("/api/v1/lists/{list}/tasks"), "");
    assert_eq!(titles(&v), ["a", "b", "c"], "appended in creation order");

    let c_id = c["id"].as_str().unwrap();
    let (s, moved) = h.call(
        Method::Put,
        &format!("/api/v1/tasks/{c_id}/order"),
        r#"{"sortOrder":500}"#,
    );
    assert_eq!((s, moved["sortOrder"].as_i64()), (200, Some(500)));
    let (_, v) = h.call(Method::Get, &format!("/api/v1/lists/{list}/tasks"), "");
    assert_eq!(titles(&v), ["a", "c", "b"]);
}

#[test]
fn search_and_tags() {
    let (_d, mut h) = harness();
    let list = h.list("L");
    h.task(&list, r#""title":"Buy groceries","tags":["home"]"#);
    h.task(
        &list,
        r#""title":"File taxes","notes":"groceries receipts","tags":["money","home"]"#,
    );
    h.task(&list, r#""title":"Call mum""#);

    let (_, v) = h.call(Method::Get, "/api/v1/search?q=groceries", "");
    assert_eq!(v["tasks"].as_array().unwrap().len(), 2);
    let (_, v) = h.call(Method::Get, "/api/v1/search?q=call%20taxes", "");
    assert_eq!(
        v["tasks"].as_array().unwrap().len(),
        2,
        "terms are alternatives; the query is percent-decoded"
    );
    assert_eq!(h.call(Method::Get, "/api/v1/search", "").0, 400);
    let (_, v) = h.call(Method::Get, "/api/v1/tags/home/tasks", "");
    assert_eq!(v["tasks"].as_array().unwrap().len(), 2);
    let (_, v) = h.call(Method::Get, "/api/v1/tags/money/tasks", "");
    assert_eq!(titles(&v), ["File taxes"]);
}

#[test]
fn smart_lists_use_the_callers_day_and_skip_done_and_archived() {
    let (_d, mut h) = harness();
    let list = h.list("L");
    let archived = h.list("Old");
    h.call(
        Method::Patch,
        &format!("/api/v1/lists/{archived}"),
        r#"{"archived":true}"#,
    );
    // NOW is 12:00 UTC on the 29th.
    let day_start = NOW - 12 * HOUR;
    for (title, due) in [
        ("overdue", NOW - 2 * DAY),
        ("this morning", day_start + 2 * HOUR),
        ("tonight", day_start + 23 * HOUR),
        ("tomorrow", day_start + DAY + HOUR),
        ("in a week", day_start + 6 * DAY),
        ("next month", day_start + 30 * DAY),
    ] {
        h.task(&list, &format!(r#""title":"{title}","dueMs":{due}"#));
    }
    h.task(&list, r#""title":"undated""#);
    h.task(
        &archived,
        &format!(r#""title":"hidden","dueMs":{}"#, day_start + HOUR),
    );
    let done = h.task(
        &list,
        &format!(r#""title":"finished","dueMs":{}"#, day_start + HOUR),
    );
    h.call(
        Method::Post,
        &format!("/api/v1/tasks/{}/complete", done["id"].as_str().unwrap()),
        "",
    );

    let get = |h: &mut Harness, p: &str| titles(&h.call(Method::Get, p, "").1);
    assert_eq!(
        get(&mut h, "/api/v1/smart/today"),
        ["overdue", "this morning", "tonight"]
    );
    // At 12:00 UTC the 02:00 task has already passed.
    assert_eq!(
        get(&mut h, "/api/v1/smart/overdue"),
        ["overdue", "this morning"]
    );
    assert_eq!(
        get(&mut h, "/api/v1/smart/next7"),
        ["this morning", "tonight", "tomorrow", "in a week"]
    );
    // At UTC+2 it is 14:00, and "tonight" (23:00 UTC = 01:00 the 30th) is tomorrow.
    assert_eq!(
        get(&mut h, "/api/v1/smart/today?utcOffsetMin=120"),
        ["overdue", "this morning"]
    );
    assert_eq!(
        h.call(Method::Get, "/api/v1/smart/today?utcOffsetMin=9999", "")
            .0,
        400
    );

    // Time passes: by midnight the evening task is overdue too.
    h.clock.store(NOW + 12 * HOUR, Ordering::SeqCst);
    assert_eq!(
        get(&mut h, "/api/v1/smart/overdue"),
        ["overdue", "this morning", "tonight"]
    );
}

#[test]
fn subtasks_are_one_level_and_follow_their_parent() {
    let (_d, mut h) = harness();
    let list = h.list("L");
    let other = h.list("M");
    let parent = h.task(&list, r#""title":"parent""#);
    let pid = parent["id"].as_str().unwrap();
    let child = h.task(&list, &format!(r#""title":"child","parentId":"{pid}""#));
    let cid = child["id"].as_str().unwrap();

    let body = |l: &str, p: &str| format!(r#"{{"listId":"{l}","title":"x","parentId":"{p}"}}"#);
    assert_eq!(
        h.call(Method::Post, "/api/v1/tasks", &body(&list, cid)).0,
        422,
        "no grandchildren"
    );
    assert_eq!(
        h.call(Method::Post, "/api/v1/tasks", &body(&other, pid)).0,
        422,
        "same list as the parent"
    );

    assert_eq!(
        h.call(Method::Delete, &format!("/api/v1/tasks/{pid}"), "")
            .0,
        204
    );
    assert_eq!(
        h.call(Method::Get, &format!("/api/v1/tasks/{cid}"), "").0,
        404,
        "subtask deleted with its parent"
    );
}

#[test]
fn unknown_routes_and_bad_ids() {
    let (_d, mut h) = harness();
    assert_eq!(h.call(Method::Get, "/api/v1/nope", "").0, 404);
    assert_eq!(h.call(Method::Get, "/api/v1/tasks/not-a-uuid", "").0, 400);
    assert_eq!(
        h.call(
            Method::Get,
            &format!("/api/v1/tasks/{}", uuid::Uuid::now_v7()),
            ""
        )
        .0,
        404
    );
    assert_eq!(h.call(Method::Get, "/api/v1/smart/whenever", "").0, 404);
}

#[test]
fn data_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let (list, task) = {
        let mut h = Harness::open(dir.path());
        let list = h.list("Persistent");
        let t = h.task(&list, r#""title":"keep me","tags":["t"]"#);
        (list, t["id"].as_str().unwrap().to_string())
    };
    let mut h = Harness::open(dir.path());
    let (s, v) = h.call(Method::Get, &format!("/api/v1/tasks/{task}"), "");
    assert_eq!(
        (s, v["title"].as_str(), v["listId"].as_str()),
        (200, Some("keep me"), Some(list.as_str()))
    );
    assert_eq!(
        titles(&h.call(Method::Get, "/api/v1/search?q=keep", "").1),
        ["keep me"]
    );
    assert_eq!(
        h.call(Method::Get, "/api/v1/tags/t/tasks", "").1["tasks"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
