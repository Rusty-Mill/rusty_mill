//! The runner over real HTTP against the real board server, with a fake workspace.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusty_mcp_server::{HttpConfig, HttpHandler};
use rusty_mill_board::api::Shared;
use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::domain::{Card, Column};
use rusty_mill_board::mcp;
use rusty_mill_board::store::MemStore;
use rusty_mill_board::web::{Auth, Web};
use rusty_mill_runner::config::{Stage, parse};
use rusty_mill_runner::http_board::HttpBoard;
use rusty_mill_runner::ports::{BoardApi, Outcome, Work};
use rusty_mill_runner::runner::Runner;

fn start(token: Option<&str>) -> (SocketAddr, Arc<Shared>) {
    let board: Arc<Shared> = Arc::new(Board::new(Box::new(MemStore::default()), Roles::default()));
    let config = HttpConfig {
        allowed_hosts: vec!["127.0.0.1".into()],
        ..HttpConfig::default()
    };
    let path = config.path.clone();
    let mcp = HttpHandler::new(mcp::server(board.clone()).expect("server"), config);
    let web = Web::new(
        board.clone(),
        mcp,
        path,
        vec!["127.0.0.1".into()],
        Auth::from(token),
    );
    let server =
        rusty_serve::Server::bind_shared("127.0.0.1:0".parse().expect("addr"), web).expect("bind");
    let addr = server.local_addr().expect("addr");
    std::thread::spawn(move || {
        let _ = server.run();
    });
    (addr, board)
}

/// Every command succeeds, except `make test` the first time.
struct Scripted {
    test_runs: std::sync::atomic::AtomicUsize,
}

impl Work for Scripted {
    fn prepare(&self, _: &Card) -> Result<PathBuf, String> {
        Ok(PathBuf::from("/work/x"))
    }
    fn run(
        &self,
        stage: &Stage,
        _: &[String],
        _: &Path,
        _: &[(String, String)],
    ) -> Result<Outcome, String> {
        let first_test = stage.name == "test"
            && self
                .test_runs
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                == 0;
        Ok(Outcome {
            ok: !first_test,
            summary: "exit".into(),
            tail: if first_test {
                "1 test failed".into()
            } else {
                String::new()
            },
        })
    }
    fn changed(&self, _: &Path) -> Result<Vec<String>, String> {
        Ok(Vec::new())
    }
    fn discard(&self, _: &Path) -> Result<(), String> {
        Ok(())
    }
    fn commit(&self, _: &Path, _: &str) -> Result<bool, String> {
        Ok(false)
    }
}

const STAGES: &str = r#"[
  {"name":"build","from":["ready","building"],"enter":"building","role":"builder","argv":["agent","{prompt}"],"ok":"qa","fail":"blocked"},
  {"name":"test","from":["qa"],"role":"qa","argv":["make","test"],"ok":"review","fail":"building"}
]"#;

#[test]
fn runner_drives_a_card_to_review_over_http_and_stops_there() {
    let (addr, board) = start(Some("tok"));
    let human = board.roles().actor("nano", "human").expect("role");
    let card = board
        .add(&human, "Add retry", "make it retry", vec![])
        .expect("add");
    board
        .move_card(card.id, Column::Ready, &human)
        .expect("triage");

    let http = HttpBoard::new(&format!("http://{addr}"), Some("tok".into()), "mill-runner")
        .expect("client");
    let runner = Runner::new(
        http,
        Scripted {
            test_runs: 0.into(),
        },
        parse(STAGES).expect("stages"),
        "mill-runner",
        1,
        vec![],
    );
    runner.validate().expect("roles allow the stages");

    // build -> qa, test fails -> building, build again -> qa, test passes -> review.
    for _ in 0..4 {
        assert_eq!(runner.tick().expect("tick").len(), 1);
    }
    assert!(
        runner.tick().expect("idle").is_empty(),
        "nothing left for the runner in Review"
    );

    let done = board.get(card.id).expect("card");
    assert_eq!(
        (done.column, done.claim.clone()),
        (Column::Review, None),
        "a person merges; the runner never does"
    );
    let notes: Vec<_> = done.comments.iter().map(|c| c.text.as_str()).collect();
    assert!(notes[0].starts_with("[build] attempt 1"), "{notes:?}");
    assert!(
        notes[1].starts_with("[test] attempt 1") && notes[1].contains("1 test failed"),
        "{notes:?}"
    );
    assert!(notes[2].starts_with("[build] attempt 2"), "{notes:?}");
    assert!(notes[3].starts_with("[test] attempt 2"), "{notes:?}");
    assert!(done.comments.iter().all(|c| c.by == "mill-runner"));
}

#[test]
fn a_wrong_token_is_a_clear_error_not_a_hang() {
    let (addr, _) = start(Some("right"));
    let http =
        HttpBoard::new(&format!("http://{addr}"), Some("wrong".into()), "r").expect("client");
    let e = http.list(Column::Ready).expect_err("unauthorized");
    assert_eq!(e.status, 401);
}

#[test]
fn unreachable_board_is_a_transport_error() {
    let http = HttpBoard::new("http://127.0.0.1:1", None, "r").expect("client");
    assert_eq!(http.list(Column::Ready).expect_err("refused").status, 0);
}

#[test]
fn validate_fails_fast_when_the_role_cannot_make_the_move() {
    let (addr, _) = start(None);
    let http = HttpBoard::new(&format!("http://{addr}"), None, "r").expect("client");
    // `qa` cannot start Ready -> Building (needs Build).
    let stages =
        parse(r#"[{"name":"b","from":["ready"],"enter":"building","role":"qa","argv":["x"]}]"#)
            .expect("stages");
    let e = Runner::new(
        http,
        Scripted {
            test_runs: 0.into(),
        },
        stages,
        "r",
        1,
        vec![],
    )
    .validate()
    .expect_err("lacks Build");
    assert!(e.contains("lacks Build"), "{e}");
}
