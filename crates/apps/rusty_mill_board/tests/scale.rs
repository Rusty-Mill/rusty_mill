//! Storage traffic and payload size per operation on a board shaped like production.
//!
//! The fixture is the live board's size profile on 2026-11-10 (1,425 cards: per card its
//! project, column and the lengths of title, body, prompt and each comment; no content, project
//! names anonymised). It is served by the fake tick over loopback HTTP with the default
//! production client, so the board reads it through the same paged collections it uses on ms-01.
//!
//! [`traffic_stays_within_ceilings`] runs in CI and checks tick requests, tick response bytes
//! and the MCP payload of each operation against [`CEILINGS`]: a slice that bounds a read
//! lowers its row. Wall time is never asserted; [`bench`] reports it (see
//! `docs/operations/scale-baseline.md`).
mod fake_tick;

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use fake_tick::FakeTick;
use fake_tick::http::{Running, Traffic, call};
use rusty_mill_board::api::{Shared, ops};
use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::projects::Projects;
use rusty_mill_board::store::Store;
use rusty_mill_board::tick::{TickHttp, TickStore};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

const TOKEN: &str = "scale-test-token";
const PROFILE: &str = include_str!("fixtures/board-profile-2026-11-10.json");
/// The profile's largest project (145 cards).
const BIG_PROJECT: &str = "p01";
/// A project of 112 cards, the size of the one in the audit (`pythrust`).
const MID_PROJECT: &str = "p03";

/// One card of the size profile.
#[derive(Deserialize)]
struct Shape {
    /// Project (anonymised).
    p: String,
    /// Column id.
    c: String,
    /// Title length.
    t: usize,
    /// Body length.
    b: usize,
    /// Prompt length.
    pr: usize,
    /// Comment lengths.
    cm: Vec<usize>,
}

/// What one operation cost: storage traffic and the MCP result's JSON size.
#[derive(Clone, Copy, Debug)]
struct Cost {
    tick: Traffic,
    payload: usize,
}

/// Upper bounds per operation at 1x scale: (tick requests, tick bytes, payload bytes).
/// Measured on main f06584c0: requests exact, bytes plus 5%. Lower a row when a slice bounds that operation;
/// never raise one without saying why in the PR.
const CEILINGS: &[(&str, usize, usize, usize)] = &[
    ("board_list", 7, 3_878_000, 2_059_000),
    ("board_list project", 7, 3_878_000, 169_000),
    ("card_get id", 5, 4_900, 2_130),
    ("card_get number", 11, 10_270, 2_130),
    ("card_take", 33, 3_912_000, 3_200),
    ("card_add", 11, 3_880_000, 500),
    ("board_events", 2, 1_301_000, 100),
];

fn shapes() -> Vec<Shape> {
    serde_json::from_str(PROFILE).expect("size profile")
}

/// A fake tick holding the profile `times` over: each card a task in its project's list with
/// a board document and its comments, all of the profile's sizes.
fn seed(times: usize) -> FakeTick {
    let shapes = shapes();
    let mut tick = FakeTick::default();
    let mut lists: BTreeMap<String, Value> = BTreeMap::new();
    let mut n: u128 = 0;
    let mut comment: u128 = 1 << 64;
    for _ in 0..times {
        for s in &shapes {
            n += 1;
            let list = lists
                .entry(s.p.clone())
                .or_insert_with(|| {
                    call(
                        &mut tick,
                        rusty_http::Method::Post,
                        "/api/v1/lists",
                        json!({ "name": s.p }),
                        false,
                    )["id"]
                        .clone()
                })
                .clone();
            let id = Uuid::from_u128(n);
            call(
                &mut tick,
                rusty_http::Method::Post,
                "/api/v1/tasks",
                json!({
                    "id": id, "listId": list, "title": "t".repeat(s.t), "notes": "b".repeat(s.b),
                    "tags": ["agent", format!("board:{}", s.c), format!("number:{n}")]
                }),
                false,
            );
            call(
                &mut tick,
                rusty_http::Method::Put,
                &format!("/api/v1/docs/board/{id}"),
                json!({
                    "v": 1, "prompt": "p".repeat(s.pr),
                    "deps": if n > 1 { vec![Uuid::from_u128(n - 1)] } else { vec![] }
                }),
                true,
            );
            for len in &s.cm {
                comment += 1;
                call(
                    &mut tick,
                    rusty_http::Method::Put,
                    &format!("/api/v1/docs/comment/{}", Uuid::from_u128(comment)),
                    json!({ "taskId": id, "text": "c".repeat(*len), "createdMs": n as i64 }),
                    false,
                );
            }
        }
    }
    tick
}

/// The board over `server`, knowing the profile's projects.
fn board(server: &Running) -> Shared {
    let mut names: Vec<String> = shapes().into_iter().map(|s| s.p).collect();
    names.sort();
    names.dedup();
    let projects: BTreeMap<&str, Vec<&str>> = names
        .iter()
        .map(|p| (p.as_str(), vec![p.as_str()]))
        .collect();
    let projects = Projects::parse(&json!(projects).to_string()).expect("projects");
    let store: Box<dyn Store> = Box::new(TickStore::new(
        TickHttp::new(&server.url, TOKEN).expect("client"),
    ));
    Board::new(store, Roles::default()).with_projects(projects)
}

/// Run the MCP operation `name` and report what it cost.
fn run(server: &Running, board: &Shared, name: &str, args: Value) -> Cost {
    let op = ops().into_iter().find(|o| o.name == name).expect("op");
    server.take_counts();
    let out = op.call(board, None, args).expect(name);
    Cost {
        tick: server.take_counts(),
        payload: serde_json::to_vec(&out).expect("json").len(),
    }
}

/// Every measured operation, in an order that leaves the reads unaffected by the writes.
fn operations(server: &Running, board: &Shared) -> Vec<(&'static str, Cost)> {
    let builder = json!({ "actor": "scale", "role": "builder" });
    // The first observation only primes the event log; measure a steady-state poll.
    run(server, board, "board_events", json!({}));
    vec![
        ("board_list", run(server, board, "board_list", json!({}))),
        (
            "board_list project",
            run(
                server,
                board,
                "board_list",
                json!({ "project": MID_PROJECT }),
            ),
        ),
        (
            "card_get id",
            run(
                server,
                board,
                "card_get",
                json!({ "id": Uuid::from_u128(700) }),
            ),
        ),
        (
            "card_get number",
            run(server, board, "card_get", json!({ "id": "#700" })),
        ),
        (
            "board_events",
            run(server, board, "board_events", json!({})),
        ),
        (
            "card_take",
            run(server, board, "card_take", builder.clone()),
        ),
        (
            "card_add",
            run(
                server,
                board,
                "card_add",
                json!({ "actor": "scale", "role": "builder", "title": "scale card", "project": BIG_PROJECT }),
            ),
        ),
    ]
}

#[test]
fn traffic_stays_within_ceilings() {
    let server = Running::start(seed(1), TOKEN);
    let board = board(&server);
    let mut over = Vec::new();
    for (name, cost) in operations(&server, &board) {
        let &(_, requests, bytes, payload) = CEILINGS
            .iter()
            .find(|c| c.0 == name)
            .expect("a ceiling for every operation");
        eprintln!(
            "{name}: tick requests={} tick bytes={} payload={}",
            cost.tick.requests, cost.tick.bytes, cost.payload
        );
        if cost.tick.requests > requests || cost.tick.bytes > bytes || cost.payload > payload {
            over.push(format!(
                "{name}: {cost:?} exceeds requests={requests} bytes={bytes} payload={payload}"
            ));
        }
    }
    assert!(over.is_empty(), "over ceiling:\n{}", over.join("\n"));
}

/// The value at quantile `q` of `samples` (sorted here).
fn quantile(samples: &mut [Duration], q: f64) -> Duration {
    samples.sort();
    let at = ((samples.len() - 1) as f64 * q).round() as usize;
    samples[at]
}

/// Wall time of the read operations, p50/p95 over 20 warm runs, at 1x and 4x the profile;
/// four concurrent full lists; and peak RSS. Run with
/// `cargo test --release -p rusty_mill_board --test scale -- --ignored --nocapture`.
#[test]
#[ignore = "benchmark: run by hand, release build"]
fn bench() {
    const RUNS: usize = 20;
    for times in [1, 4] {
        let server = Running::start(seed(times), TOKEN);
        let board = board(&server);
        println!("## {times}x ({} cards)", times * shapes().len());
        println!("| operation | p50 | p95 | tick requests | tick bytes | payload |");
        println!("|---|---:|---:|---:|---:|---:|");
        let reads: [(&str, Value, &str); 5] = [
            ("board_list", json!({}), "board_list"),
            (
                "board_list project",
                json!({ "project": MID_PROJECT }),
                "board_list",
            ),
            (
                "card_get id",
                json!({ "id": Uuid::from_u128(700) }),
                "card_get",
            ),
            ("card_get number", json!({ "id": "#700" }), "card_get"),
            ("board_events", json!({}), "board_events"),
        ];
        run(&server, &board, "board_events", json!({}));
        for (label, args, name) in reads {
            let mut walls = Vec::with_capacity(RUNS);
            let mut cost = run(&server, &board, name, args.clone());
            for _ in 0..RUNS {
                let start = Instant::now();
                cost = run(&server, &board, name, args.clone());
                walls.push(start.elapsed());
            }
            println!(
                "| {label} | {:.1?} | {:.1?} | {} | {} | {} |",
                quantile(&mut walls, 0.5),
                quantile(&mut walls, 0.95),
                cost.tick.requests,
                cost.tick.bytes,
                cost.payload
            );
        }
        let start = Instant::now();
        std::thread::scope(|s| {
            let readers: Vec<_> = (0..4)
                .map(|_| s.spawn(|| run(&server, &board, "board_list", json!({}))))
                .collect();
            for r in readers {
                r.join().expect("reader");
            }
        });
        println!("\n4 concurrent board_list: {:.1?} wall\n", start.elapsed());
    }
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        for line in status.lines().filter(|l| l.starts_with("VmHWM:")) {
            println!("peak RSS (fixture, server and board in one process): {line}");
        }
    }
}
