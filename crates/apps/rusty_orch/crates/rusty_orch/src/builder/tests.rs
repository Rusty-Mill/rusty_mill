//! #1318 S5: `builder run`'s command line, prompt, row and store helpers;
//! the Linux adapter's pure parts are in `live`'s tests.

use orch_lens::builder::{End, Phase, Session};
use orch_lens::card::Card;
use orch_lens::DataClass;
use rusty_bbp::{ArtId, ClaimStatus, MsgId, Stakes};
use rusty_json::Value;

use super::*;

fn argv(extra: &[&str]) -> Vec<String> {
    let base = [
        "run",
        "cfg.json",
        "--bbp",
        "bbp",
        "--dir",
        "/d",
        "--task",
        "t",
        "--claim",
        "7",
        "--state",
        "/s",
        "--repo",
        "/r",
        "--base",
        "main",
        "--cgroup-parent",
        "/sys/fs/cgroup/x",
        "--slots",
        "/slots",
        "--mux-ctl",
        "rusty-mux-ctl",
    ];
    base.iter().chain(extra).map(|s| (*s).to_owned()).collect()
}

#[test]
fn the_command_line_parses_with_defaults() {
    let o = options(argv(&["--read", "/opt/node", "--read", "/home/n/.cargo"])).expect("parse");
    assert_eq!(o.claim, MsgId(7));
    assert_eq!(o.jail, PathBuf::from("lel-jail"));
    assert_eq!(o.mux_socket, None);
    assert_eq!(
        o.read,
        [PathBuf::from("/opt/node"), PathBuf::from("/home/n/.cargo")]
    );
    assert_eq!(o.cgroup_parent, PathBuf::from("/sys/fs/cgroup/x"));
}

#[test]
fn a_bad_command_line_is_a_usage_error() {
    let usage = |a: Vec<String>| matches!(options(a), Err(ArgsError::Usage(_)));
    assert!(usage(argv(&["--claim", "8"])), "duplicate flag");
    let mut a = argv(&[]);
    a[9] = "seven".into();
    assert!(usage(a), "claim not a number");
    let a: Vec<String> = argv(&[]).into_iter().take(20).collect();
    assert!(usage(a), "--mux-ctl missing");
    assert!(usage(argv(&["--wat"])));
    assert!(matches!(
        options(["--help".to_owned()]),
        Err(ArgsError::Help)
    ));
}

#[test]
fn the_prompt_names_the_claim_and_the_submit_marker() {
    let card = Card {
        id: MsgId(7),
        text: "Hash by size first".to_owned(),
        domains: vec!["dedup".to_owned()],
        status: ClaimStatus::Open,
        score: 0,
        accept_at: 8,
        epoch: 0,
        stakes: Stakes::Medium,
        reversible: true,
        answers: None,
        deps: vec![],
        spent: 0,
        budget: 16,
        evidence: vec![],
        candidates: vec![],
        current: None,
        class: DataClass::Internal,
    };
    let p = prompt(&card, Path::new("/wt/7/1"));
    assert!(p.contains(&card.text));
    assert!(p.contains("/wt/7/1"));
    assert!(p.contains(SUBMIT));
}

fn session(n: u32, phase: Phase) -> Session {
    Session {
        claim: MsgId(7),
        n,
        started: 10,
        phase,
        tokens_in: 5,
        tokens_out: 2,
        run: None,
    }
}

#[test]
fn the_row_is_one_json_object() {
    let s = session(
        2,
        Phase::Ended {
            end: End::Stored(ArtId(40)),
            at: 99,
        },
    );
    let v = row(&s, &["kill: busy".into()]);
    assert_eq!(v["state"], Value::from("stored"));
    assert_eq!(v["candidate"], Value::from(40u64));
    assert_eq!(v["ended"], Value::from(99u64));
    assert_eq!(v["notes"][0], Value::from("kill: busy"));
    let v = row(&session(1, Phase::Running), &[]);
    assert_eq!(v["state"], Value::from("running"));
    assert_eq!(v["candidate"], Value::Null);
}

#[test]
fn a_row_is_replaced_in_place_or_added() {
    let rows = vec![session(1, Phase::Admitted), session(2, Phase::Admitted)];
    let rows = replace(rows, &session(2, Phase::Running));
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].phase, Phase::Running);
    let rows = replace(rows, &session(3, Phase::Admitted));
    assert_eq!(rows.len(), 3);
}

#[test]
fn rows_save_through_the_store_lock() {
    let dir = std::env::temp_dir().join(format!("orch-builder-store-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let saved = with_store(&dir, |store| {
        store
            .save_builder_sessions("b", &[session(1, Phase::Admitted)])
            .map_err(|e| e.to_string())
    });
    assert_eq!(saved, Ok(()));
    // Held by another opener: the wait gives up with an error, not a hang.
    let held = Store::open(&dir).expect("hold");
    let start = std::time::Instant::now();
    let wait = Duration::from_millis(300);
    assert!(with_store_waiting(&dir, wait, |_| Ok(())).is_err());
    assert!(start.elapsed() >= wait);
    drop(held);
    let rows = with_store(&dir, |s| {
        s.load_builder_sessions("b").map_err(|e| e.to_string())
    });
    assert_eq!(rows.map(|r| r.len()), Ok(1));
    let _ = std::fs::remove_dir_all(&dir);
}

fn sessions_argv(extra: &[&str]) -> Vec<String> {
    let base = [
        "sessions", "cfg.json", "--dir", "/d", "--task", "t", "--state", "/s",
    ];
    base.iter().chain(extra).map(|s| (*s).to_owned()).collect()
}

#[test]
fn the_sessions_command_line_parses() {
    let o = sessions_options(sessions_argv(&[])).expect("parse");
    assert_eq!(o.claim, None);
    assert_eq!(o.state, PathBuf::from("/s"));
    let o = sessions_options(sessions_argv(&["--claim", "7"])).expect("parse");
    assert_eq!(o.claim, Some(MsgId(7)));
}

#[test]
fn a_bad_sessions_command_line_is_a_usage_error() {
    let usage = |a: Vec<String>| matches!(sessions_options(a), Err(ArgsError::Usage(_)));
    assert!(
        usage(sessions_argv(&["--claim", "x"])),
        "claim not a number"
    );
    assert!(usage(sessions_argv(&["--task", "u"])), "duplicate flag");
    assert!(usage(sessions_argv(&["--bbp", "bbp"])), "a run-only flag");
    let a: Vec<String> = sessions_argv(&[]).into_iter().take(6).collect();
    assert!(usage(a), "--state missing");
    assert!(usage(vec!["run".into()]), "not sessions");
}

#[test]
fn session_lines_drop_notes_keep_the_claim_and_sort() {
    let mut other = session(1, Phase::Running);
    other.claim = MsgId(3);
    let failed = Phase::Ended {
        end: End::Failed,
        at: 50,
    };
    let rows = vec![session(2, failed), other, session(1, Phase::Running)];
    let all = session_lines(rows.clone(), None);
    let ns: Vec<(u64, u64)> = all
        .lines()
        .map(|l| {
            let v = Value::parse(l).expect("json");
            assert!(v.get("notes").is_none(), "no notes: {l}");
            (
                v["claim"].as_u64().expect("claim"),
                v["n"].as_u64().expect("n"),
            )
        })
        .collect();
    assert_eq!(ns, [(3, 1), (7, 1), (7, 2)]);
    let seven = session_lines(rows, Some(MsgId(7)));
    assert_eq!(seven.lines().count(), 2);
    assert!(seven.contains(r#""state":"failed""#), "{seven}");
    assert_eq!(session_lines(vec![], None), "");
}

#[test]
fn sessions_reads_the_store_and_refuses_a_missing_state_dir() {
    let root = std::env::temp_dir().join(format!("orch-builder-sessions-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (bbp, state, cfg) = (root.join("bbp"), root.join("state"), root.join("lens.json"));
    std::fs::create_dir_all(&bbp).expect("bbp dir");
    std::fs::write(
        &cfg,
        include_str!("../../../orch-lens/tests/fixtures/lens/lens.json"),
    )
    .expect("config");
    let o = SessionsOptions {
        config: cfg,
        dir: bbp.clone(),
        task: "t".into(),
        state: state.clone(),
        claim: Some(MsgId(7)),
    };
    let missing = sessions(&o);
    assert!(missing.is_err(), "no state dir: {missing:?}");
    assert!(!state.exists(), "a missing state dir is not created");
    std::fs::create_dir_all(&state).expect("state dir");
    assert_eq!(sessions(&o), Ok(String::new()));
    let board = crate::lens::run::board(&bbp, "t").expect("board");
    let saved = with_store(&state, |s| {
        s.save_builder_sessions(&board, &[session(1, Phase::Running)])
            .map_err(|e| e.to_string())
    });
    assert_eq!(saved, Ok(()));
    let out = sessions(&o).expect("read");
    assert_eq!(out.lines().count(), 1);
    assert!(out.contains(r#""state":"running""#), "{out}");
    let _ = std::fs::remove_dir_all(&root);
}
