//! Regression against the real storage server and default HTTP response limit.
use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::store::Store;
use rusty_mill_board::tick::{TickApi, TickHttp, TickRequest, TickStore};
use rusty_tick::api::Request;
use rusty_tick::backend::Backend;
use rusty_tick::server::{ShutdownHandle, bind};
use rusty_tick::service::system_clock;
use serde_json::{Value, json};
use std::time::Instant;
use uuid::Uuid;

const TOKEN: &str = "large-board-test-token";
const COUNT: u128 = 240;
const INBOX: &str = "00000000-0000-7000-8000-000000000001";

struct Running {
    url: String,
    stop: ShutdownHandle,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Running {
    fn start(backend: Backend) -> Self {
        let server = bind("127.0.0.1:0".parse().expect("test fixture"), backend, None)
            .expect("test fixture");
        let url = format!("http://{}", server.local_addr().expect("test fixture"));
        let stop = server.shutdown_handle().expect("test fixture");
        let thread = Some(std::thread::spawn(move || {
            server.run().expect("test fixture")
        }));
        Self { url, stop, thread }
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        self.stop.shutdown();
        self.thread
            .take()
            .expect("test fixture")
            .join()
            .expect("test fixture");
    }
}
fn call(
    backend: &mut Backend,
    method: rusty_http::Method,
    path: &str,
    body: Value,
    fenced: bool,
) -> Value {
    let body = body.to_string();
    let response = backend.handle(&Request {
        method: &method,
        target: path,
        authorization: Some("Bearer large-board-test-token"),
        if_match: None,
        if_none_match: None,
        if_task_match: fenced.then_some("1"),
        actor: None,
        body: body.as_bytes(),
    });
    assert!(
        response.status.as_u16() < 300,
        "{}: {}",
        response.status,
        String::from_utf8_lossy(&response.body)
    );
    serde_json::from_slice(&response.body).expect("test fixture")
}

#[test]
fn large_board_reads_preserve_prompts_and_relationships() {
    let dir = tempfile::tempdir().expect("test fixture");
    let mut backend =
        Backend::single(dir.path(), TOKEN.into(), system_clock()).expect("test fixture");
    let other = call(
        &mut backend,
        rusty_http::Method::Post,
        "/api/v1/lists",
        json!({"name":"other"}),
        false,
    )["id"]
        .clone();
    let prompt = "Codex instructions: preserve every detail.\n".repeat(400);
    let notes = "Roadmap context.\n".repeat(400);
    let milestone = Uuid::from_u128(10_000);
    call(
        &mut backend,
        rusty_http::Method::Post,
        "/api/v1/tasks",
        json!({
            "id":milestone, "listId":INBOX, "title":"Launch", "dueMs":2_000_000_000_000i64, "tags":["milestone"]
        }),
        false,
    );
    for n in 1..=COUNT {
        let id = Uuid::from_u128(n);
        call(
            &mut backend,
            rusty_http::Method::Post,
            "/api/v1/tasks",
            json!({
                "id": id, "listId": if n % 2 == 0 {other.clone()} else {json!(INBOX)},
                "title": format!("Card {n}"), "notes": notes, "tags": ["agent", "board:backlog", format!("number:{n}")]
            }),
            false,
        );
        call(
            &mut backend,
            rusty_http::Method::Put,
            &format!("/api/v1/docs/board/{id}"),
            json!({
                "v":1, "prompt":prompt, "parent": (n > 1).then_some(Uuid::from_u128(1)),
                "deps": if n > 1 {vec![Uuid::from_u128(n-1)]} else {vec![]}, "milestone":milestone
            }),
            true,
        );
        call(
            &mut backend,
            rusty_http::Method::Put,
            &format!("/api/v1/docs/comment/{}", Uuid::from_u128(COUNT + n)),
            json!({
                "taskId":id, "text":"Review context.\n".repeat(400), "createdMs":n as i64
            }),
            false,
        );
    }
    let server = Running::start(backend);
    let http = TickHttp::new(&server.url, TOKEN).expect("test fixture");
    // Prove this fixture crosses the exact original transport boundary.
    let start = Instant::now();
    let old = http
        .send(&TickRequest {
            method: "GET",
            path: "/docs/board".into(),
            ..Default::default()
        })
        .expect_err("unpaged response must exceed the limit");
    assert!(
        old.to_string()
            .contains("http body exceeded the maximum allowed size")
    );
    for path in ["/tags/agent/tasks", "/docs/comment"] {
        assert!(
            http.send(&TickRequest {
                method: "GET",
                path: path.into(),
                ..Default::default()
            })
            .expect_err("unpaged response must exceed the limit")
            .to_string()
            .contains("http body exceeded the maximum allowed size")
        );
    }
    eprintln!(
        "unpaged request rejected in {:?}; prompt bytes={}",
        start.elapsed(),
        prompt.len() * COUNT as usize
    );
    for path in ["/tags/agent/tasks", "/docs/board", "/docs/comment"] {
        let (_, page) = http
            .send(&TickRequest {
                method: "GET",
                path: format!("{path}?limit=1000"),
                ..Default::default()
            })
            .expect("bounded page");
        let bytes = serde_json::to_vec(&page).expect("serialize page").len();
        assert!(bytes < 1 << 20);
        assert!(!page["next"].is_null());
        eprintln!("{path} first page bytes={bytes}");
    }
    let store = TickStore::new(http);
    let start = Instant::now();
    let card = store
        .get(Uuid::from_u128(2))
        .expect("test fixture")
        .expect("test fixture");
    assert_eq!(card.prompt, prompt);
    assert_eq!(card.body, notes);
    assert_eq!(card.parent, Some(Uuid::from_u128(1)));
    assert_eq!(card.deps, vec![Uuid::from_u128(1)]);
    assert_eq!(card.comments.len(), 1);
    eprintln!("single-card read {:?}", start.elapsed());
    let start = Instant::now();
    let cards = store.list().expect("test fixture");
    assert_eq!(cards.len(), COUNT as usize);
    assert!(
        cards
            .iter()
            .all(|c| c.prompt == prompt && c.comments.len() == 1)
    );
    assert!(
        store
            .events_since(None, 10)
            .expect("test fixture")
            .is_empty()
    );
    eprintln!("complete board read {:?}", start.elapsed());

    let board = Board::new(store, Roles::default()).with_milestones(Box::new(TickStore::new(
        TickHttp::new(&server.url, TOKEN).expect("test fixture"),
    )));
    assert_eq!(
        board.list(None).expect("test fixture").len(),
        COUNT as usize
    );
    assert_eq!(board.get_number(2).expect("test fixture").prompt, prompt);
    let roadmap = board.roadmap("default").expect("roadmap");
    assert_eq!(
        roadmap
            .lanes
            .iter()
            .map(|lane| lane.cards.len())
            .sum::<usize>(),
        COUNT as usize / 2
    );
    board
        .trace(Uuid::from_u128(1), |_| true)
        .expect("test fixture");
    let milestones = board.milestones(|_| true).expect("test fixture");
    assert_eq!(milestones.len(), 1);
    assert_eq!(
        board.milestone_progress(milestones).expect("test fixture")[0]
            .progress
            .total,
        COUNT as usize
    );

    // Separate HTTP connections race real fenced updates with complete collection reads.
    std::thread::scope(|scope| {
        let reader = scope.spawn(|| {
            for _ in 0..3 {
                let cards = board.list(None).expect("test fixture");
                assert_eq!(cards.len(), COUNT as usize);
                assert!(cards.iter().all(|c| c.prompt == prompt));
            }
        });
        let writer = TickHttp::new(&server.url, TOKEN).expect("test fixture");
        for _ in 0..3 {
            let (status, _) = writer
                .send(&TickRequest {
                    method: "PATCH",
                    path: format!("/tasks/{}", Uuid::from_u128(2)),
                    body: Some(json!({"title":"updated"})),
                    if_match: Some("2".into()),
                    ..Default::default()
                })
                .expect("test fixture");
            assert!(status == 200 || status == 412);
        }
        reader.join().expect("test fixture");
    });
    assert_eq!(
        board.get(Uuid::from_u128(2)).expect("test fixture").title,
        "updated"
    );
    let before = board.list(None).expect("test fixture");
    drop(board);
    drop(server);
    let server = Running::start(
        Backend::single(dir.path(), TOKEN.into(), system_clock()).expect("test fixture"),
    );
    let reopened = TickStore::new(TickHttp::new(&server.url, TOKEN).expect("test fixture"));
    assert_eq!(reopened.list().expect("test fixture"), before);
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        for line in status.lines().filter(|l| l.starts_with("VmHWM:")) {
            eprintln!(
                "process peak RSS {line} (includes fixture, server, and several full board copies)"
            );
        }
    }
}
