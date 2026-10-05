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
        self.send_with(method, target, body, auth, None)
    }

    fn send_with(
        &mut self,
        method: Method,
        target: &str,
        body: &str,
        auth: Option<&str>,
        if_match: Option<&str>,
    ) -> (u16, Value) {
        let header = auth.map(|t| format!("Bearer {t}"));
        let request = Request {
            method: &method,
            target,
            authorization: header.as_deref(),
            if_match,
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

fn names(v: &Value, key: &str) -> Vec<String> {
    v[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["name"].as_str().unwrap().to_string())
        .collect()
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
    let list = h.list("Errands");
    let (_, v) = h.call(Method::Get, "/api/v1/lists", "");
    assert_eq!(
        names(&v, "lists"),
        ["Inbox", "Errands"],
        "the built-in Inbox, then ours"
    );

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
    let (status, trashed) = h.call(Method::Get, &format!("/api/v1/tasks/{task_id}"), "");
    assert_eq!(
        status, 200,
        "a deleted list's tasks go to the trash, not away"
    );
    assert!(trashed["deletedMs"].is_i64());
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
fn wont_do_is_a_status_of_its_own() {
    let (_d, mut h) = harness();
    let list = h.list("L");
    let a = h.task(&list, r#""title":"a""#);
    h.task(&list, r#""title":"b""#);
    let path = format!("/api/v1/tasks/{}", a["id"].as_str().unwrap());

    let (_, v) = h.call(Method::Patch, &path, r#"{"status":"wontdo"}"#);
    assert_eq!(v["status"], "wontdo");
    assert!(v["completedMs"].is_i64(), "closing stamps completedMs");
    let (_, closed) = h.call(
        Method::Get,
        &format!("/api/v1/lists/{list}/tasks?status=wontdo"),
        "",
    );
    assert_eq!(titles(&closed), ["a"]);
    let (_, open) = h.call(
        Method::Get,
        &format!("/api/v1/lists/{list}/tasks?status=open"),
        "",
    );
    assert_eq!(titles(&open), ["b"]);
    let (_, v) = h.call(Method::Patch, &path, r#"{"status":"open"}"#);
    assert!(v["completedMs"].is_null(), "reopening clears it");
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
    let (_, v) = h.call(Method::Get, "/api/v1/search?q=grocer", "");
    assert_eq!(
        v["tasks"].as_array().unwrap().len(),
        2,
        "a term matches as a prefix: search as you type"
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
    let (_, child) = h.call(Method::Get, &format!("/api/v1/tasks/{cid}"), "");
    assert!(
        child["deletedMs"].is_i64(),
        "subtask trashed with its parent"
    );
}

#[test]
fn trash_family_preserves_pretrashed_children_and_restore_ownership() {
    let (_d, mut h) = harness();
    let list = h.list("L");
    let parent = h.task(&list, r#""title":"parent""#);
    let pid = parent["id"].as_str().unwrap();
    let first = h.task(&list, &format!(r#""title":"first","parentId":"{pid}""#));
    let first_id = first["id"].as_str().unwrap();
    let old = h.task(&list, &format!(r#""title":"old","parentId":"{pid}""#));
    let old_id = old["id"].as_str().unwrap();

    h.clock.store(NOW - HOUR, Ordering::SeqCst);
    assert_eq!(
        h.call(Method::Delete, &format!("/api/v1/tasks/{old_id}"), "")
            .0,
        204
    );
    let (_, old_before) = h.call(Method::Get, &format!("/api/v1/tasks/{old_id}"), "");
    h.clock.store(NOW, Ordering::SeqCst);
    assert_eq!(
        h.call(Method::Delete, &format!("/api/v1/tasks/{pid}"), "")
            .0,
        204
    );

    let (_, first_trashed) = h.call(Method::Get, &format!("/api/v1/tasks/{first_id}"), "");
    let (_, old_trashed) = h.call(Method::Get, &format!("/api/v1/tasks/{old_id}"), "");
    assert_eq!(first_trashed["deletedMs"].as_i64(), Some(NOW));
    assert_eq!(old_trashed["deletedMs"], old_before["deletedMs"]);
    assert_eq!(first_trashed["etag"].as_str(), Some("2"));
    assert_eq!(old_trashed["etag"].as_str(), Some("3"));

    assert_eq!(
        h.call(Method::Post, &format!("/api/v1/tasks/{pid}/restore"), "")
            .0,
        200
    );
    let (_, first_back) = h.call(Method::Get, &format!("/api/v1/tasks/{first_id}"), "");
    let (_, old_still_trashed) = h.call(Method::Get, &format!("/api/v1/tasks/{old_id}"), "");
    assert!(first_back["deletedMs"].is_null());
    assert_eq!(old_still_trashed["deletedMs"], old_before["deletedMs"]);
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

const INBOX: &str = "00000000-0000-7000-8000-000000000001";

#[test]
fn the_inbox_is_permanent() {
    let (_d, mut h) = harness();
    let (s, v) = h.call(Method::Get, &format!("/api/v1/lists/{INBOX}"), "");
    assert_eq!((s, v["name"].as_str()), (200, Some("Inbox")));
    assert_eq!(
        h.call(Method::Delete, &format!("/api/v1/lists/{INBOX}"), "")
            .0,
        422
    );
    assert_eq!(
        h.call(
            Method::Patch,
            &format!("/api/v1/lists/{INBOX}"),
            r#"{"archived":true}"#
        )
        .0,
        422
    );
    assert_eq!(
        h.call(Method::Get, "/api/v1/snapshot", "").1["inboxId"],
        INBOX
    );
}

#[test]
fn clients_may_choose_ids_so_creates_are_idempotent() {
    let (_d, mut h) = harness();
    let id = uuid::Uuid::now_v7();
    let body = format!(r#"{{"id":"{id}","name":"Mine"}}"#);
    let (s, v) = h.call(Method::Post, "/api/v1/lists", &body);
    assert_eq!((s, v["id"].as_str()), (201, Some(id.to_string().as_str())));
    assert_eq!(
        h.call(Method::Post, "/api/v1/lists", &body).0,
        409,
        "replaying a create is refused, not duplicated"
    );

    let tid = uuid::Uuid::now_v7();
    let task = format!(r#"{{"id":"{tid}","listId":"{id}","title":"t"}}"#);
    assert_eq!(h.call(Method::Post, "/api/v1/tasks", &task).0, 201);
    assert_eq!(h.call(Method::Post, "/api/v1/tasks", &task).0, 409);
    assert_eq!(
        h.call(
            Method::Post,
            "/api/v1/tasks",
            r#"{"id":"nope","listId":"x","title":"t"}"#
        )
        .0,
        400
    );
}

#[test]
fn rich_fields_round_trip_and_completion_is_stamped() {
    let (_d, mut h) = harness();
    let list = h.list("L");
    let item = uuid::Uuid::now_v7();
    let t = h.task(
        &list,
        &format!(
            r#""title":"rich","kind":"checklist","startMs":1000,"dueMs":2000,"isAllDay":true,"timeZone":"America/Chicago","reminders":["TRIGGER:PT0S"],"repeatFlag":"RRULE:FREQ=DAILY","items":[{{"id":"{item}","title":"step","done":false,"sortOrder":1}}]"#
        ),
    );
    assert_eq!(t["kind"], "checklist");
    assert_eq!(t["reminders"], rusty_json::json!(["TRIGGER:PT0S"]));
    assert_eq!(t["items"][0]["title"], "step");
    assert_eq!(
        (t["startMs"].as_i64(), t["isAllDay"].as_bool()),
        (Some(1000), Some(true))
    );
    let path = format!("/api/v1/tasks/{}", t["id"].as_str().unwrap());

    let (_, done) = h.call(Method::Patch, &path, r#"{"status":"done"}"#);
    assert_eq!(
        (done["status"].as_str(), done["completedMs"].as_i64()),
        (Some("done"), Some(NOW))
    );
    let (_, open) = h.call(Method::Patch, &path, r#"{"status":"open"}"#);
    assert!(
        open["completedMs"].is_null(),
        "reopening clears the completion time"
    );
    let (_, v) = h.call(
        Method::Patch,
        &path,
        &format!(r#"{{"items":[{{"id":"{item}","title":"step","done":true}}]}}"#),
    );
    assert_eq!(v["items"][0]["done"], true);
    assert_eq!(
        h.call(
            Method::Patch,
            &path,
            r#"{"items":[{"id":"x","title":"t"}]}"#
        )
        .0,
        400,
        "a malformed item id"
    );
}

#[test]
fn trash_hides_restores_and_purges() {
    let (_d, mut h) = harness();
    let list = h.list("L");
    let t = h.task(&list, r#""title":"findable","tags":["keep"],"dueMs":1000"#);
    let id = t["id"].as_str().unwrap().to_string();
    let path = format!("/api/v1/tasks/{id}");

    assert_eq!(h.call(Method::Delete, &path, "").0, 204);
    let (_, v) = h.call(Method::Get, &path, "");
    assert!(v["deletedMs"].is_i64(), "still readable, marked deleted");
    assert!(titles(
        &h.call(Method::Get, &format!("/api/v1/lists/{list}/tasks"), "")
            .1
    )
    .is_empty());
    assert!(
        titles(&h.call(Method::Get, "/api/v1/search?q=findable", "").1).is_empty(),
        "trash is not searchable"
    );
    assert!(titles(&h.call(Method::Get, "/api/v1/tags/keep/tasks", "").1).is_empty());
    assert!(titles(&h.call(Method::Get, "/api/v1/smart/overdue", "").1).is_empty());
    let (_, snap) = h.call(Method::Get, "/api/v1/snapshot", "");
    assert!(
        snap["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == id.as_str()),
        "the snapshot carries the trash"
    );

    let (s, back) = h.call(Method::Post, &format!("{path}/restore"), "");
    assert_eq!((s, back["deletedMs"].is_null()), (200, true));
    assert_eq!(
        titles(&h.call(Method::Get, "/api/v1/search?q=findable", "").1),
        ["findable"],
        "restored tasks are searchable again"
    );

    assert_eq!(
        h.call(Method::Delete, &format!("{path}?permanent=true"), "")
            .0,
        204
    );
    assert_eq!(h.call(Method::Get, &path, "").0, 404);

    let a = h.task(&list, r#""title":"a""#);
    h.call(
        Method::Delete,
        &format!("/api/v1/tasks/{}", a["id"].as_str().unwrap()),
        "",
    );
    let (_, purged) = h.call(Method::Delete, "/api/v1/trash", "");
    assert_eq!(purged["purged"], 1);
}

#[test]
fn comments_follow_their_task_to_the_grave_but_not_to_the_trash() {
    let (_d, mut h) = harness();
    let list = h.list("L");
    let comment = |h: &mut Harness, task: &str, text: &str| {
        let id = uuid::Uuid::now_v7();
        let body = format!(r#"{{"v":1,"taskId":"{task}","text":"{text}","createdMs":1}}"#);
        assert_eq!(
            h.call(Method::Put, &format!("/api/v1/docs/comment/{id}"), &body)
                .0,
            200
        );
    };
    let count = |h: &mut Harness| {
        h.call(Method::Get, "/api/v1/docs/comment", "").1["docs"]
            .as_array()
            .unwrap()
            .len()
    };
    let kept = h.task(&list, r#""title":"kept""#);
    let purged = h.task(&list, r#""title":"purged""#);
    let binned = h.task(&list, r#""title":"binned""#);
    let (kept, purged, binned) = (
        kept["id"].as_str().unwrap().to_string(),
        purged["id"].as_str().unwrap().to_string(),
        binned["id"].as_str().unwrap().to_string(),
    );
    for t in [&kept, &purged, &binned] {
        comment(&mut h, t, "hello");
    }

    h.call(Method::Delete, &format!("/api/v1/tasks/{purged}"), "");
    assert_eq!(
        count(&mut h),
        3,
        "the trash keeps comments, so a restore loses nothing"
    );
    h.call(
        Method::Delete,
        &format!("/api/v1/tasks/{purged}?permanent=true"),
        "",
    );
    assert_eq!(
        count(&mut h),
        2,
        "a purge takes the task's comments with it"
    );

    h.call(Method::Delete, &format!("/api/v1/tasks/{binned}"), "");
    h.call(Method::Delete, "/api/v1/trash", "");
    let (_, docs) = h.call(Method::Get, "/api/v1/docs/comment", "");
    let docs = docs["docs"].as_array().unwrap();
    assert_eq!(docs.len(), 1, "emptying the trash does too");
    assert_eq!(docs[0]["body"]["taskId"], kept.as_str());
}

#[test]
fn a_task_whose_list_was_deleted_restores_into_the_inbox() {
    let (_d, mut h) = harness();
    let list = h.list("Doomed");
    let parent = h.task(&list, r#""title":"orphan""#);
    let id = parent["id"].as_str().unwrap();
    let child = h.task(&list, &format!(r#""title":"child","parentId":"{id}""#));
    let child_id = child["id"].as_str().unwrap();
    let old = h.task(&list, &format!(r#""title":"old","parentId":"{id}""#));
    let old_id = old["id"].as_str().unwrap();
    h.clock.store(NOW - HOUR, Ordering::SeqCst);
    h.call(Method::Delete, &format!("/api/v1/tasks/{old_id}"), "");
    h.clock.store(NOW, Ordering::SeqCst);
    h.call(Method::Delete, &format!("/api/v1/lists/{list}"), "");
    let (_, back) = h.call(Method::Post, &format!("/api/v1/tasks/{id}/restore"), "");
    assert_eq!(back["listId"], INBOX);
    let (_, child_back) = h.call(Method::Get, &format!("/api/v1/tasks/{child_id}"), "");
    let (_, old_back) = h.call(Method::Get, &format!("/api/v1/tasks/{old_id}"), "");
    assert_eq!(child_back["listId"], INBOX);
    assert!(child_back["deletedMs"].is_null());
    assert_eq!(old_back["listId"], INBOX);
    assert_eq!(old_back["deletedMs"].as_i64(), Some(NOW - HOUR));
}

#[test]
fn if_match_refuses_stale_writes_and_returns_the_current_record() {
    let (_d, mut h) = harness();
    let list = h.list("L");
    let t = h.task(&list, r#""title":"v1""#);
    let path = format!("/api/v1/tasks/{}", t["id"].as_str().unwrap());
    let etag = t["etag"].as_str().unwrap().to_string();

    let (s, v) = h.send_with(
        Method::Patch,
        &path,
        r#"{"title":"v2"}"#,
        Some(TOKEN),
        Some(&format!("\"{etag}\"")),
    );
    assert_eq!((s, v["title"].as_str()), (200, Some("v2")));
    assert_ne!(
        v["etag"].as_str().unwrap(),
        etag,
        "every write changes the etag"
    );

    let (s, v) = h.send_with(
        Method::Patch,
        &path,
        r#"{"title":"lost"}"#,
        Some(TOKEN),
        Some(&etag),
    );
    assert_eq!(s, 412, "the old etag is stale");
    assert_eq!(v["current"]["title"], "v2", "the response carries what won");
    assert_eq!(v["error"]["code"], "precondition_failed");
    assert_eq!(
        h.call(Method::Get, &path, "").1["title"],
        "v2",
        "nothing was written"
    );
    assert_eq!(
        h.call(Method::Patch, &path, r#"{"title":"no header"}"#).0,
        200,
        "If-Match is optional"
    );
}

#[test]
fn tags_are_entities_that_survive_their_tasks() {
    let (_d, mut h) = harness();
    let list = h.list("L");
    let t = h.task(&list, r#""title":"t","tags":["Home Office"]"#);
    assert_eq!(
        t["tags"],
        rusty_json::json!(["home office"]),
        "tasks store the lowercase name"
    );
    let (_, tags) = h.call(Method::Get, "/api/v1/tags", "");
    assert_eq!(
        tags["tags"][0]["label"], "Home Office",
        "the entity keeps the label as typed"
    );

    let (s, v) = h.call(
        Method::Patch,
        "/api/v1/tags/home%20office",
        r##"{"color":"#ED70A5"}"##,
    );
    assert_eq!((s, v["color"].as_str()), (200, Some("#ed70a5")));
    assert_eq!(
        h.call(
            Method::Patch,
            "/api/v1/tags/home%20office",
            r#"{"color":"pink"}"#
        )
        .0,
        422
    );
    assert_eq!(
        h.call(Method::Post, "/api/v1/tags", r#"{"label":"HOME OFFICE"}"#)
            .0,
        409,
        "names are case-insensitive"
    );

    let (s, v) = h.call(
        Method::Post,
        "/api/v1/tags/home%20office/rename",
        r#"{"label":"Work"}"#,
    );
    assert_eq!((s, v["name"].as_str()), (200, Some("work")));
    assert_eq!(
        h.call(
            Method::Get,
            &format!("/api/v1/tasks/{}", t["id"].as_str().unwrap()),
            ""
        )
        .1["tags"],
        rusty_json::json!(["work"])
    );
    assert_eq!(
        h.call(Method::Get, "/api/v1/tags/home%20office/tasks", "")
            .1["tasks"]
            .as_array()
            .unwrap()
            .len(),
        0
    );

    assert_eq!(h.call(Method::Delete, "/api/v1/tags/work", "").0, 204);
    let (_, v) = h.call(
        Method::Get,
        &format!("/api/v1/tasks/{}", t["id"].as_str().unwrap()),
        "",
    );
    assert_eq!(
        v["tags"],
        rusty_json::json!([]),
        "deleting a tag takes it off its tasks"
    );
    assert_eq!(h.call(Method::Delete, "/api/v1/tags/work", "").0, 404);
}

#[test]
fn tag_parents_must_exist_and_be_another_tag() {
    let (_d, mut h) = harness();
    h.call(Method::Post, "/api/v1/tags", r#"{"label":"parent"}"#);
    h.call(Method::Post, "/api/v1/tags", r#"{"label":"child"}"#);
    assert_eq!(
        h.call(
            Method::Patch,
            "/api/v1/tags/child",
            r#"{"parent":"parent"}"#
        )
        .0,
        200
    );
    assert_eq!(
        h.call(Method::Patch, "/api/v1/tags/child", r#"{"parent":"child"}"#)
            .0,
        422
    );
    assert_eq!(
        h.call(Method::Patch, "/api/v1/tags/child", r#"{"parent":"ghost"}"#)
            .0,
        422
    );
    let (_, v) = h.call(Method::Patch, "/api/v1/tags/child", r#"{"parent":null}"#);
    assert!(v["parent"].is_null(), "null clears the parent");
}

#[test]
fn moving_a_task_moves_its_subtasks() {
    let (d, mut h) = harness();
    let (a, b) = (h.list("A"), h.list("B"));
    let parent = h.task(&a, r#""title":"p""#);
    let pid = parent["id"].as_str().unwrap();
    let child = h.task(&a, &format!(r#""title":"c","parentId":"{pid}""#));
    let cid = child["id"].as_str().unwrap();

    let (s, v) = h.call(
        Method::Patch,
        &format!("/api/v1/tasks/{pid}"),
        &format!(r#"{{"listId":"{b}","sortOrder":17}}"#),
    );
    assert_eq!((s, v["listId"].as_str()), (200, Some(b.as_str())));
    assert_eq!(v["sortOrder"].as_i64(), Some(17));
    assert_eq!(
        h.call(Method::Get, &format!("/api/v1/tasks/{cid}"), "").1["listId"],
        b.as_str()
    );
    assert_eq!(
        h.call(
            Method::Patch,
            &format!("/api/v1/tasks/{cid}"),
            &format!(r#"{{"listId":"{a}"}}"#)
        )
        .0,
        422,
        "a subtask cannot move alone"
    );
    assert_eq!(
        titles(
            &h.call(Method::Get, &format!("/api/v1/lists/{b}/tasks"), "")
                .1
        ),
        ["p", "c"]
    );
    drop(h);
    let mut reopened = Harness::open(d.path());
    let (_, parent) = reopened.call(Method::Get, &format!("/api/v1/tasks/{pid}"), "");
    let (_, child) = reopened.call(Method::Get, &format!("/api/v1/tasks/{cid}"), "");
    assert_eq!(parent["listId"], b.as_str());
    assert_eq!(parent["sortOrder"].as_i64(), Some(17));
    assert_eq!(child["listId"], b.as_str());
}

#[test]
fn list_colors_view_modes_and_reordering() {
    let (_d, mut h) = harness();
    let list = h.list("L");
    let path = format!("/api/v1/lists/{list}");
    let (_, v) = h.call(
        Method::Patch,
        &path,
        r##"{"color":"#4772FA","viewMode":"kanban","sortType":"priority"}"##,
    );
    assert_eq!(
        (
            v["color"].as_str(),
            v["viewMode"].as_str(),
            v["sortType"].as_str()
        ),
        (Some("#4772fa"), Some("kanban"), Some("priority"))
    );
    assert_eq!(
        h.call(Method::Patch, &path, r#"{"viewMode":"gantt"}"#).0,
        400
    );
    let (_, v) = h.call(Method::Patch, &path, r#"{"color":null}"#);
    assert!(v["color"].is_null());
    let (_, v) = h.call(Method::Patch, &path, r#"{"sortOrder":-5000000}"#);
    assert_eq!(v["sortOrder"], -5_000_000);
    assert_eq!(
        names(&h.call(Method::Get, "/api/v1/lists", "").1, "lists"),
        ["Inbox", "L"].map(String::from),
        "Inbox stays first"
    );
}

#[test]
fn docs_store_client_json_durably_and_validate_it() {
    let dir = tempfile::tempdir().unwrap();
    let id = uuid::Uuid::now_v7();
    {
        let mut h = Harness::open(dir.path());
        let (s, v) = h.call(
            Method::Put,
            &format!("/api/v1/docs/habit/{id}"),
            r#"{"name":"Read","goal":30}"#,
        );
        assert_eq!((s, v["body"]["name"].as_str()), (200, Some("Read")));
        let (s, _) = h.call(
            Method::Put,
            &format!("/api/v1/docs/habit/{id}"),
            r#"{"name":"Read more"}"#,
        );
        assert_eq!(s, 200, "PUT replaces");
        assert_eq!(
            h.call(Method::Put, &format!("/api/v1/docs/nonsense/{id}"), "{}")
                .0,
            422,
            "kinds are a closed set"
        );
        assert_eq!(
            h.call(
                Method::Put,
                &format!("/api/v1/docs/habit/{}", uuid::Uuid::now_v7()),
                "not json"
            )
            .0,
            422
        );
        assert_eq!(
            h.call(Method::Put, &format!("/api/v1/docs/focus/{id}"), "{}")
                .0,
            409,
            "an id belongs to one kind"
        );
        let big = format!(r#"{{"x":"{}"}}"#, "a".repeat(70_000));
        assert_eq!(
            h.call(
                Method::Put,
                &format!("/api/v1/docs/habit/{}", uuid::Uuid::now_v7()),
                &big
            )
            .0,
            422
        );
    }
    let mut h = Harness::open(dir.path());
    let (_, v) = h.call(Method::Get, "/api/v1/docs/habit", "");
    assert_eq!(v["docs"].as_array().unwrap().len(), 1);
    assert_eq!(
        v["docs"][0]["body"]["name"], "Read more",
        "documents survive a restart"
    );
    assert_eq!(
        h.call(Method::Delete, &format!("/api/v1/docs/habit/{id}"), "")
            .0,
        204
    );
    assert_eq!(
        h.call(Method::Delete, &format!("/api/v1/docs/habit/{id}"), "")
            .0,
        404
    );
    assert_eq!(
        h.call(Method::Get, "/api/v1/docs/prefs", "").1["docs"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn the_snapshot_carries_everything_a_client_boots_from() {
    let (_d, mut h) = harness();
    let list = h.list("L");
    let a = h.task(&list, r#""title":"open one","tags":["t"]"#);
    let b = h.task(&list, r#""title":"done one""#);
    h.call(
        Method::Post,
        &format!("/api/v1/tasks/{}/complete", b["id"].as_str().unwrap()),
        "",
    );
    h.call(
        Method::Delete,
        &format!("/api/v1/tasks/{}", a["id"].as_str().unwrap()),
        "",
    );
    let (s, v) = h.call(Method::Get, "/api/v1/snapshot", "");
    assert_eq!(s, 200);
    assert_eq!(
        v["tasks"].as_array().unwrap().len(),
        2,
        "completed and trashed tasks both come along"
    );
    assert_eq!(v["lists"].as_array().unwrap().len(), 2);
    assert_eq!(v["tags"][0]["name"], "t");
    assert_eq!(v["serverTimeMs"], NOW);
}

#[test]
fn a_task_can_be_created_at_an_explicit_position() {
    let (_d, mut h) = harness();
    let list = h.list("L");
    h.task(&list, r#""title":"appended""#);
    h.task(&list, r#""title":"on top","sortOrder":-5000"#);
    let (_, v) = h.call(Method::Get, &format!("/api/v1/lists/{list}/tasks"), "");
    assert_eq!(titles(&v), ["on top", "appended"]);
}
