//! The GitHub adapter against a recording mock server: the exact requests
//! sent, pagination, PR filtering and error mapping.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

use rusty_mill_board::api::Shared;
use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::github::{GithubConfig, GithubForge};
use rusty_mill_board::store::MemStore;
use rusty_mill_board::sync::{Forge, SyncError, reconcile};

/// What the mock saw: request line, lowercase headers, body.
#[derive(Debug, Clone)]
struct Seen {
    line: String,
    headers: String,
    body: String,
}

/// Serve `(status, body)` pairs in order, one connection each; returns the
/// base URL and the shared request log.
fn mock(replies: Vec<(u16, String)>) -> (String, Arc<Mutex<Vec<Seen>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let log = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&log);
    thread::spawn(move || {
        for (status, body) in replies {
            let (mut sock, _) = listener.accept().expect("accept");
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            let (head_end, want) = loop {
                let n = sock.read(&mut chunk).expect("read");
                buf.extend_from_slice(&chunk[..n]);
                if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..i]).to_lowercase();
                    let len = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    break (i + 4, len);
                }
                assert!(n > 0, "closed before headers");
            };
            while buf.len() < head_end + want {
                let n = sock.read(&mut chunk).expect("read body");
                assert!(n > 0, "closed before body");
                buf.extend_from_slice(&chunk[..n]);
            }
            let head = String::from_utf8_lossy(&buf[..head_end - 4]).into_owned();
            let (line, headers) = head.split_once("\r\n").unwrap_or((&head, ""));
            seen.lock().expect("log").push(Seen {
                line: line.to_owned(),
                headers: headers.to_lowercase(),
                body: String::from_utf8_lossy(&buf[head_end..]).into_owned(),
            });
            let reason = if status == 200 { "OK" } else { "Error" };
            let reply = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            sock.write_all(reply.as_bytes()).expect("reply");
        }
    });
    (base, log)
}

fn forge(base: &str) -> GithubForge {
    GithubForge::new(GithubConfig::new(base, "o/r", "s3cret").expect("config")).expect("forge")
}

fn issue_json(n: u64, title: &str, labels: &[&str], state: &str) -> String {
    let labels: Vec<_> = labels
        .iter()
        .map(|l| format!(r#"{{"name":"{l}"}}"#))
        .collect();
    format!(
        r#"{{"number":{n},"title":"{title}","body":null,"labels":[{}],"state":"{state}","updated_at":"2026-10-13T14:48:34Z"}}"#,
        labels.join(",")
    )
}

#[test]
fn unlinked_card_creates_an_issue_with_the_right_request() {
    let (base, log) = mock(vec![
        (200, "[]".into()),
        (
            201,
            issue_json(1, "Ship", &["board:backlog", "bbp"], "open"),
        ),
    ]);
    let board: Shared = Board::new(Box::new(MemStore::default()), Roles::default());
    let human = board.roles().actor("nano", "human").expect("role");
    let card = board
        .add(&human, "Ship", "details", vec!["bbp".into()])
        .expect("add");

    let report = reconcile(&board, &forge(&base)).expect("sync");
    assert_eq!(report.issues_created, 1);
    assert_eq!(board.get(card.id).expect("card").issue, Some(1));

    let log = log.lock().expect("log");
    assert_eq!(
        log[0].line,
        "GET /repos/o/r/issues?state=all&per_page=100&page=1 HTTP/1.1"
    );
    assert!(log[0].headers.contains("authorization: bearer s3cret"));
    assert!(
        log[0]
            .headers
            .contains("accept: application/vnd.github+json")
    );
    assert!(log[0].headers.contains("user-agent: rusty_mill_board"));
    assert_eq!(log[1].line, "POST /repos/o/r/issues HTTP/1.1");
    let body: serde_json::Value = serde_json::from_str(&log[1].body).expect("json body");
    assert_eq!(body["title"], "Ship");
    assert_eq!(body["body"], "details");
    assert_eq!(body["labels"], serde_json::json!(["board:backlog", "bbp"]));
}

#[test]
fn pull_requests_are_skipped_and_pages_are_followed() {
    let item = |n: u64| issue_json(n, "i", &[], "open");
    let page1: Vec<String> = (1..=100).map(item).collect();
    // The last entry of page 2 is a pull request: the issues API lists those too.
    let pr = r#"{"number":102,"title":"pr","body":"","labels":[],"state":"open","updated_at":"2026-10-13T14:48:34Z","pull_request":{}}"#;
    let page2 = vec![item(101), pr.to_owned()];
    let (base, log) = mock(vec![
        (200, format!("[{}]", page1.join(","))),
        (200, format!("[{}]", page2.join(","))),
    ]);

    let issues = forge(&base).list_issues().expect("list");
    assert_eq!(issues.len(), 101);
    assert!(issues.iter().all(|i| i.number != 102));
    assert!(log.lock().expect("log")[1].line.contains("page=2"));
}

#[test]
fn done_card_without_issue_is_created_then_closed() {
    use rusty_mill_board::sync::Draft;
    let (base, log) = mock(vec![
        (201, issue_json(7, "t", &["board:done"], "open")),
        (200, issue_json(7, "t", &["board:done"], "closed")),
    ]);
    let draft = Draft {
        title: "t".into(),
        body: String::new(),
        labels: vec!["board:done".into()],
        closed: true,
    };
    let issue = forge(&base).create_issue(&draft).expect("create");
    assert!(issue.closed);
    let log = log.lock().expect("log");
    assert!(log[0].line.starts_with("POST /repos/o/r/issues "));
    assert!(log[1].line.starts_with("PATCH /repos/o/r/issues/7 "));
    assert!(log[1].body.contains(r#""state":"closed""#));
}

#[test]
fn api_errors_name_the_status_and_never_the_token() {
    let (base, _) = mock(vec![(
        403,
        r#"{"message":"Resource not accessible"}"#.into(),
    )]);
    let err = forge(&base).list_issues().expect_err("403");
    let SyncError::Forge(msg) = &err else {
        panic!("{err}")
    };
    assert!(
        msg.contains("403") && msg.contains("Resource not accessible"),
        "{msg}"
    );
    assert!(!msg.contains("s3cret"));
}
