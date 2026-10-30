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

/// A request's precondition headers.
#[derive(Default)]
struct Conditions<'a> {
    if_match: Option<&'a str>,
    if_none_match: Option<&'a str>,
    if_task_match: Option<&'a str>,
    actor: Option<&'a str>,
}

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
        self.send_conditional(
            method,
            target,
            body,
            auth,
            Conditions {
                if_match,
                ..Conditions::default()
            },
        )
    }

    fn send_conditional(
        &mut self,
        method: Method,
        target: &str,
        body: &str,
        auth: Option<&str>,
        when: Conditions<'_>,
    ) -> (u16, Value) {
        let header = auth.map(|t| format!("Bearer {t}"));
        let request = Request {
            method: &method,
            target,
            authorization: header.as_deref(),
            if_match: when.if_match,
            if_none_match: when.if_none_match,
            if_task_match: when.if_task_match,
            actor: when.actor,
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

#[test]
fn a_board_number_is_reserved_once() {
    let (_d, mut h) = harness();
    let path = format!("/api/v1/docs/board_number/{}", uuid::Uuid::now_v7());
    let create = Conditions {
        if_none_match: Some("*"),
        ..Conditions::default()
    };
    let body = r#"{"v":1,"number":1}"#;
    let (s, doc) = h.send_conditional(Method::Put, &path, body, Some(TOKEN), create);
    assert_eq!(s, 200, "{doc:?}");
    let again = Conditions {
        if_none_match: Some("*"),
        ..Conditions::default()
    };
    let (s, _) = h.send_conditional(Method::Put, &path, body, Some(TOKEN), again);
    assert_eq!(s, 412, "a reserved number is not reserved twice");
    let (s, v) = h.call(Method::Get, "/api/v1/docs/board_number", "");
    assert_eq!((s, v["docs"].as_array().map(Vec::len)), (200, Some(1)));
}

#[test]
fn a_board_document_is_fenced_by_its_task_and_moves_it() {
    let (_d, mut h) = harness();
    let list = h.list("L");
    let t = h.task(&list, r#""title":"card""#);
    let id = t["id"].as_str().unwrap().to_string();
    let path = format!("/api/v1/docs/board/{id}");
    let version = |h: &mut Harness| {
        let (_, t) = h.call(Method::Get, &format!("/api/v1/tasks/{id}"), "");
        t["etag"].as_str().unwrap().to_string()
    };
    let put = |h: &mut Harness, when: Conditions<'_>, gen: u32| {
        let body = format!(r#"{{"v":1,"generation":{gen}}}"#);
        h.send_conditional(Method::Put, &path, &body, Some(TOKEN), when)
    };

    let (s, _) = put(&mut h, Conditions::default(), 1);
    assert_eq!(s, 400, "a board document needs If-Task-Match");

    let v1 = version(&mut h);
    let create = Conditions {
        if_none_match: Some("*"),
        if_task_match: Some(&v1),
        ..Conditions::default()
    };
    let (s, doc) = put(&mut h, create, 1);
    assert_eq!(s, 200, "{doc:?}");
    let e1 = doc["etag"].as_str().unwrap().to_string();
    let v2 = version(&mut h);
    assert_ne!(v1, v2, "the write moves the task's version");

    let replayed = Conditions {
        if_none_match: Some("*"),
        if_task_match: Some(&v1),
        ..Conditions::default()
    };
    let (s, v) = put(&mut h, replayed, 9);
    assert_eq!(s, 412, "the version before the write no longer matches");
    assert_eq!(v["current"]["etag"], v2.as_str());
    let create = Conditions {
        if_none_match: Some("*"),
        if_task_match: Some(&v2),
        ..Conditions::default()
    };
    let (s, v) = put(&mut h, create, 9);
    assert_eq!(s, 412, "If-None-Match: * refuses an existing document");
    assert_eq!(v["current"]["body"]["generation"], 1);

    let next = Conditions {
        if_match: Some(&e1),
        if_task_match: Some(&v2),
        ..Conditions::default()
    };
    let (s, doc) = put(&mut h, next, 2);
    assert_eq!(s, 200);
    let e2 = doc["etag"].as_str().unwrap().to_string();
    assert_ne!(e1, e2, "the etag moves even within one millisecond");
    let v3 = version(&mut h);
    let stale = Conditions {
        if_match: Some(&e1),
        if_task_match: Some(&v3),
        ..Conditions::default()
    };
    assert_eq!(
        put(&mut h, stale, 3).0,
        412,
        "a stale document etag is refused"
    );

    h.call(
        Method::Patch,
        &format!("/api/v1/tasks/{id}"),
        r#"{"title":"moved"}"#,
    );
    let fenced = Conditions {
        if_match: Some(&e2),
        if_task_match: Some(&v3),
        ..Conditions::default()
    };
    let (s, v) = put(&mut h, fenced, 3);
    assert_eq!(
        s, 412,
        "a write authorized against an older task is refused"
    );
    assert_eq!(
        v["current"]["title"], "moved",
        "and the response carries the task"
    );

    let (s, v) = h.call(Method::Get, &path, "");
    assert_eq!((s, v["body"]["generation"].as_i64()), (200, Some(2)));
    assert_eq!(v["etag"], e2.as_str());
}

#[test]
fn a_comment_written_with_if_task_match_is_fenced_by_its_task_and_moves_it() {
    let (_d, mut h) = harness();
    let list = h.list("L");
    let t = h.task(&list, r#""title":"card""#);
    let id = t["id"].as_str().unwrap().to_string();
    let v1 = t["etag"].as_str().unwrap().to_string();
    let comment = |h: &mut Harness, task: &str, version: &str| {
        let path = format!("/api/v1/docs/comment/{}", uuid::Uuid::now_v7());
        let body = format!(r#"{{"v":1,"taskId":"{task}","text":"x","createdMs":1}}"#);
        let when = Conditions {
            if_none_match: Some("*"),
            if_task_match: Some(version),
            ..Conditions::default()
        };
        h.send_conditional(Method::Put, &path, &body, Some(TOKEN), when)
            .0
    };

    assert_eq!(comment(&mut h, &id, &v1), 200);
    let (_, now) = h.call(Method::Get, &format!("/api/v1/tasks/{id}"), "");
    assert_ne!(
        now["etag"].as_str(),
        Some(v1.as_str()),
        "it moves the version"
    );
    assert_eq!(
        comment(&mut h, &id, &v1),
        412,
        "a second append from the same read is refused"
    );
    assert_eq!(comment(&mut h, "not-a-task", &v1), 400);
    let (_, docs) = h.call(Method::Get, "/api/v1/docs/comment", "");
    assert_eq!(docs["docs"].as_array().unwrap().len(), 1);

    // Without the header a comment is written as before and the task is untouched.
    let path = format!("/api/v1/docs/comment/{}", uuid::Uuid::now_v7());
    let body = format!(r#"{{"v":1,"taskId":"{id}","text":"owner","createdMs":2}}"#);
    let (s, _) = h.call(Method::Put, &path, &body);
    assert_eq!(s, 200);
    let (_, after) = h.call(Method::Get, &format!("/api/v1/tasks/{id}"), "");
    assert_eq!(after["etag"], now["etag"]);
}

#[test]
fn a_board_document_needs_a_task_that_is_not_in_the_trash() {
    let (_d, mut h) = harness();
    let list = h.list("L");
    let when = Conditions {
        if_task_match: Some("1"),
        ..Conditions::default()
    };
    let nowhere = format!("/api/v1/docs/board/{}", uuid::Uuid::now_v7());
    assert_eq!(
        h.send_conditional(Method::Put, &nowhere, "{}", Some(TOKEN), when)
            .0,
        404
    );

    let t = h.task(&list, r#""title":"binned""#);
    let id = t["id"].as_str().unwrap();
    h.call(Method::Delete, &format!("/api/v1/tasks/{id}"), "");
    let version = h.call(Method::Get, &format!("/api/v1/tasks/{id}"), "").1["etag"]
        .as_str()
        .unwrap()
        .to_string();
    let when = Conditions {
        if_task_match: Some(&version),
        ..Conditions::default()
    };
    let path = format!("/api/v1/docs/board/{id}");
    assert_eq!(
        h.send_conditional(Method::Put, &path, "{}", Some(TOKEN), when)
            .0,
        422
    );
}

#[test]
fn a_purge_takes_the_task_board_document_with_it() {
    let (_d, mut h) = harness();
    let list = h.list("L");
    let board_doc = |h: &mut Harness| {
        let t = h.task(&list, r#""title":"card""#);
        let id = t["id"].as_str().unwrap().to_string();
        let when = Conditions {
            if_task_match: t["etag"].as_str(),
            ..Conditions::default()
        };
        let path = format!("/api/v1/docs/board/{id}");
        let (s, _) = h.send_conditional(Method::Put, &path, "{}", Some(TOKEN), when);
        assert_eq!(s, 200);
        (id, path)
    };
    let (purged, purged_doc) = board_doc(&mut h);
    let (binned, binned_doc) = board_doc(&mut h);

    h.call(Method::Delete, &format!("/api/v1/tasks/{purged}"), "");
    assert_eq!(
        h.call(Method::Get, &purged_doc, "").0,
        200,
        "the trash keeps it"
    );
    h.call(
        Method::Delete,
        &format!("/api/v1/tasks/{purged}?permanent=true"),
        "",
    );
    assert_eq!(h.call(Method::Get, &purged_doc, "").0, 404);

    h.call(Method::Delete, &format!("/api/v1/tasks/{binned}"), "");
    h.call(Method::Delete, "/api/v1/trash", "");
    assert_eq!(
        h.call(Method::Get, &binned_doc, "").0,
        404,
        "emptying the trash does too"
    );
}

#[test]
fn document_preconditions_and_single_reads() {
    let (_d, mut h) = harness();
    let id = uuid::Uuid::now_v7();
    let path = format!("/api/v1/docs/habit/{id}");
    assert_eq!(h.call(Method::Get, &path, "").0, 404);

    let when = Conditions {
        if_match: Some("123"),
        ..Conditions::default()
    };
    let (s, v) = h.send_conditional(Method::Put, &path, "{}", Some(TOKEN), when);
    assert_eq!(s, 412, "If-Match on a missing document");
    assert!(v["current"].is_null());

    let when = Conditions {
        if_none_match: Some("\"5\""),
        ..Conditions::default()
    };
    let (s, _) = h.send_conditional(Method::Put, &path, "{}", Some(TOKEN), when);
    assert_eq!(s, 400, "only If-None-Match: * is understood");

    assert_eq!(h.call(Method::Put, &path, r#"{"name":"Read"}"#).0, 200);
    let (s, v) = h.call(Method::Get, &path, "");
    assert_eq!((s, v["body"]["name"].as_str()), (200, Some("Read")));
    assert_eq!(
        h.call(Method::Get, &format!("/api/v1/docs/focus/{id}"), "")
            .0,
        404,
        "a document is read under its own kind"
    );
}

#[test]
fn a_leaf_delete_refuses_a_task_with_subtasks() {
    let (_dir, mut h) = harness();
    let list = h.list("L");
    let parent = h.task(&list, r#""title":"parent""#)["id"]
        .as_str()
        .unwrap()
        .to_string();
    let child = h.task(&list, &format!(r#""title":"child","parentId":"{parent}""#))["id"]
        .as_str()
        .unwrap()
        .to_string();

    let (status, _) = h.call(
        Method::Delete,
        &format!("/api/v1/tasks/{parent}?leaf=true"),
        "",
    );
    assert_eq!(status, 409);
    let (_, v) = h.call(Method::Get, &format!("/api/v1/tasks/{child}"), "");
    assert!(v["deletedMs"].is_null(), "the subtask is untouched");
    let (status, _) = h.call(
        Method::Delete,
        &format!("/api/v1/tasks/{parent}?leaf=true&permanent=true"),
        "",
    );
    assert_eq!(status, 400, "a purge cannot be a leaf delete");

    let (status, _) = h.call(
        Method::Delete,
        &format!("/api/v1/tasks/{child}?leaf=true"),
        "",
    );
    assert_eq!(status, 204);
    let (status, _) = h.call(
        Method::Delete,
        &format!("/api/v1/tasks/{parent}?leaf=true"),
        "",
    );
    assert_eq!(status, 409, "a trashed subtask is still its subtask");

    let (status, _) = h.call(
        Method::Delete,
        "/api/v1/tasks/00000000-0000-0000-0000-000000000000?leaf=probe",
        "",
    );
    assert_eq!(
        status, 400,
        "an unknown leaf value is refused before the task is looked up"
    );
}

/// `(from, to, actor)` of each move in a `moves` response.
fn moves(v: &Value) -> Vec<(Value, Value, String)> {
    v["moves"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            (
                m["from"].clone(),
                m["to"].clone(),
                m["actor"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[test]
fn every_column_change_is_recorded_with_its_actor_and_time() {
    let (_d, mut h) = harness();
    let list = h.list("L");
    let actor = |name| Conditions {
        actor: Some(name),
        ..Conditions::default()
    };
    let (s, card) = h.send_conditional(
        Method::Post,
        "/api/v1/tasks",
        &format!(r#"{{"listId":"{list}","title":"card","tags":["agent","board:ready"]}}"#),
        Some(TOKEN),
        actor("planner"),
    );
    assert_eq!(s, 201);
    let id = card["id"].as_str().unwrap().to_string();
    let path = format!("/api/v1/tasks/{id}");
    h.task(&list, r#""title":"not on the board""#);

    h.clock.store(NOW + HOUR, Ordering::SeqCst);
    let (s, _) = h.send_conditional(
        Method::Patch,
        &path,
        r#"{"tags":["agent","board:building","claim:builder"]}"#,
        Some(TOKEN),
        actor("builder"),
    );
    assert_eq!(s, 200);
    // Not a column change: nothing recorded.
    h.call(Method::Patch, &path, r#"{"title":"renamed"}"#);
    h.clock.store(NOW + 2 * HOUR, Ordering::SeqCst);
    // Completing in tick's own UI moves the card to done.
    h.call(Method::Post, &format!("{path}/complete"), "");

    let (s, all) = h.call(Method::Get, &format!("{path}/moves"), "");
    assert_eq!(s, 200);
    let ready = Value::from("ready");
    let building = Value::from("building");
    assert_eq!(
        moves(&all),
        [
            (Value::Null, ready.clone(), "planner".to_string()),
            (ready, building.clone(), "builder".to_string()),
            (building, Value::from("done"), "tick".to_string()),
        ]
    );
    let at: Vec<i64> = all["moves"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["atMs"].as_i64().unwrap())
        .collect();
    assert_eq!(at, [NOW, NOW + HOUR, NOW + 2 * HOUR]);
    assert_eq!(all["moves"][0]["taskId"].as_str(), Some(id.as_str()));
}

#[test]
fn moves_page_by_time_and_seq_and_survive_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = Harness::open(dir.path());
    let list = h.list("L");
    for (hour, column) in [(0, "ready"), (1, "qa"), (2, "review")] {
        h.clock.store(NOW + hour * HOUR, Ordering::SeqCst);
        h.task(&list, &format!(r#""title":"c","tags":["board:{column}"]"#));
    }
    drop(h);
    let mut h = Harness::open(dir.path());

    let since = NOW + HOUR;
    let (s, page) = h.call(
        Method::Get,
        &format!("/api/v1/moves?since={since}&limit=1"),
        "",
    );
    assert_eq!(s, 200);
    assert_eq!(page["moves"][0]["to"].as_str(), Some("qa"));
    let next = page["next"].as_u64().unwrap();
    let (_, rest) = h.call(
        Method::Get,
        &format!("/api/v1/moves?since={since}&after={next}&limit=1"),
        "",
    );
    assert_eq!(rest["moves"][0]["to"].as_str(), Some("review"));
    assert_eq!(rest["next"], Value::Null, "the last page");
    let (_, everything) = h.call(Method::Get, "/api/v1/moves", "");
    assert_eq!(everything["moves"].as_array().unwrap().len(), 3);
}

#[test]
fn bad_actors_and_paging_arguments_are_refused() {
    let (_d, mut h) = harness();
    let list = h.list("L");
    let (s, _) = h.send_conditional(
        Method::Post,
        "/api/v1/tasks",
        &format!(r#"{{"listId":"{list}","title":"c"}}"#),
        Some(TOKEN),
        Conditions {
            actor: Some("two words"),
            ..Conditions::default()
        },
    );
    assert_eq!(s, 422);
    assert_eq!(h.call(Method::Get, "/api/v1/moves?limit=0", "").0, 400);
    assert_eq!(h.call(Method::Get, "/api/v1/moves?limit=1001", "").0, 400);
    assert_eq!(h.call(Method::Get, "/api/v1/moves?since=soon", "").0, 400);
    assert_eq!(
        h.call(
            Method::Get,
            "/api/v1/tasks/00000000-0000-0000-0000-000000000000/moves",
            ""
        )
        .0,
        404
    );
}

#[test]
fn a_purge_takes_the_task_history_with_it() {
    let (_d, mut h) = harness();
    let list = h.list("L");
    let t = h.task(&list, r#""title":"c","tags":["board:ready"]"#);
    let id = t["id"].as_str().unwrap();
    h.call(Method::Delete, &format!("/api/v1/tasks/{id}"), "");
    let (_, kept) = h.call(Method::Get, "/api/v1/moves", "");
    assert_eq!(
        kept["moves"].as_array().unwrap().len(),
        1,
        "the trash keeps it"
    );
    h.call(Method::Delete, "/api/v1/trash", "");
    let (_, gone) = h.call(Method::Get, "/api/v1/moves", "");
    assert!(gone["moves"].as_array().unwrap().is_empty());
}
