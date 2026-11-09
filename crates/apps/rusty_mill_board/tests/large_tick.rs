//! Regression at the Tick HTTP boundary, using the default production client.
mod fake_tick;
use fake_tick::FakeTick;
use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::store::Store;
use rusty_mill_board::tick::{TickApi, TickHttp, TickRequest, TickStore};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use std::time::Instant;
use uuid::Uuid;

const TOKEN: &str = "large-board-test-token";
const COUNT: u128 = 240;
const INBOX: &str = "00000000-0000-7000-8000-000000000001";

struct Running {
    url: String,
    addr: std::net::SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Running {
    fn start(tick: FakeTick) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture");
        let addr = listener.local_addr().expect("fixture address");
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = Some(std::thread::spawn(move || {
            for stream in listener.incoming() {
                if stopping.load(Ordering::SeqCst) {
                    break;
                }
                serve(stream.expect("accept"), &tick);
            }
        }));
        Self {
            url: format!("http://{addr}"),
            addr,
            stop,
            thread,
        }
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
        self.thread
            .take()
            .expect("server thread")
            .join()
            .expect("join server");
    }
}
fn serve(mut stream: TcpStream, tick: &FakeTick) {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("read timeout");
    let mut bytes = Vec::new();
    let mut buffer = [0; 8192];
    let head_end = loop {
        let n = stream.read(&mut buffer).expect("request head");
        assert!(n > 0);
        bytes.extend_from_slice(&buffer[..n]);
        if let Some(index) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let head = String::from_utf8(bytes[..head_end].to_vec()).expect("HTTP head");
    let mut lines = head.lines();
    let mut first = lines.next().expect("request line").split_whitespace();
    let method = first.next().expect("method");
    let path = first
        .next()
        .expect("path")
        .strip_prefix("/api/v1")
        .expect("API prefix");
    let headers: Vec<_> = lines.filter_map(|l| l.split_once(':')).collect();
    let header = |name: &str| {
        headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.trim().to_owned())
    };
    assert_eq!(
        header("Authorization").as_deref(),
        Some("Bearer large-board-test-token")
    );
    let len = header("Content-Length")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    while bytes.len() - head_end < len {
        let n = stream.read(&mut buffer).expect("request body");
        assert!(n > 0);
        bytes.extend_from_slice(&buffer[..n]);
    }
    let (status, body) = tick
        .send(&TickRequest {
            method,
            path: path.into(),
            body: if len == 0 {
                None
            } else {
                Some(
                    serde_json::from_slice(&bytes[head_end..head_end + len]).expect("JSON request"),
                )
            },
            if_match: header("If-Match"),
            if_absent: header("If-None-Match").as_deref() == Some("*"),
            if_task_match: header("If-Task-Match"),
        })
        .expect("fixture route");
    let body = body.to_string();
    let response = format!(
        "HTTP/1.1 {status} Result\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    // The client deliberately closes oversized responses before reading the body.
    let _ = stream.write_all(response.as_bytes());
}
fn call(
    tick: &mut FakeTick,
    method: rusty_http::Method,
    path: &str,
    body: Value,
    fenced: bool,
) -> Value {
    let (status, body) = tick
        .send(&TickRequest {
            method: &method.to_string(),
            path: path.strip_prefix("/api/v1").expect("API path").into(),
            body: Some(body),
            if_task_match: fenced.then(|| "1".into()),
            ..Default::default()
        })
        .expect("fixture request");
    assert!(status < 300, "fixture status {status}");
    body
}

#[test]
fn large_board_reads_preserve_prompts_and_relationships() {
    let mut backend = FakeTick::default();
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
    let server = Running::start(backend.clone());
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
    let server = Running::start(backend);
    let reopened = TickStore::new(TickHttp::new(&server.url, TOKEN).expect("test fixture"));
    assert_eq!(reopened.list().expect("test fixture"), before);
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        for line in status.lines().filter(|l| l.starts_with("VmHWM:")) {
            eprintln!(
                "process peak RSS {line} (includes fixture, HTTP server, and several full board copies)"
            );
        }
    }
}
