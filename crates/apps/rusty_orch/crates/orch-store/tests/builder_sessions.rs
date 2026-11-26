//! The `builder_session` table (#600, card #1404): round-trips, resume
//! after a crash mid-session, and refused rows.

use std::path::PathBuf;

use orch_lens::builder::{self, End, Phase, ProfileResult, RunResult, Session};
use orch_store::{Store, StoreError};
use rusty_bbp::{ArtId, MsgId};
use rusty_multimodal_db_engine::test_support::fresh_temp_dir;

fn dir(label: &str) -> PathBuf {
    fresh_temp_dir(&format!("orch_store_builders_{label}")).expect("temp dir")
}

fn session(claim: u64, n: u32, phase: Phase) -> Session {
    Session {
        claim: MsgId(claim),
        n,
        started: 1_000 + u64::from(n),
        phase,
        tokens_in: 120,
        tokens_out: 45,
        run: None,
    }
}

fn every_phase() -> Vec<Session> {
    let stored = Session {
        run: Some(RunResult {
            profiles: vec![
                ProfileResult {
                    profile: "suite:unit".to_owned(),
                    passed: true,
                },
                ProfileResult {
                    profile: "erosion".to_owned(),
                    passed: false,
                },
            ],
            erosion_delta: Some(3),
        }),
        ..session(
            7,
            1,
            Phase::Ended {
                end: End::Stored(ArtId(11)),
                at: 2_000,
            },
        )
    };
    vec![
        stored,
        session(
            7,
            2,
            Phase::Ended {
                end: End::Failed,
                at: 2_100,
            },
        ),
        session(
            7,
            3,
            Phase::Ended {
                end: End::Aborted,
                at: 2_200,
            },
        ),
        session(8, 1, Phase::Running),
        session(9, 1, Phase::Admitted),
    ]
}

#[test]
fn sessions_round_trip_through_a_reopen() {
    let dir = dir("round_trip");
    let sessions = every_phase();
    {
        let mut store = Store::open(&dir).expect("open");
        assert!(store.load_builder_sessions("t1").expect("empty").is_empty());
        store.save_builder_sessions("t1", &sessions).expect("save");
    }
    let store = Store::open(&dir).expect("reopen");
    assert_eq!(store.load_builder_sessions("t1").expect("load"), sessions);
    assert!(
        store.load_builder_sessions("t2").expect("other").is_empty(),
        "boards are kept apart"
    );
    drop(store);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn saving_again_replaces_the_board() {
    let dir = dir("replace");
    let mut store = Store::open(&dir).expect("open");
    store
        .save_builder_sessions("t1", &every_phase())
        .expect("first");
    let one = vec![session(8, 1, Phase::Running)];
    store.save_builder_sessions("t1", &one).expect("second");
    assert_eq!(store.load_builder_sessions("t1").expect("load"), one);
    drop(store);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_crash_mid_session_resumes_as_aborted_and_frees_the_claim() {
    let dir = dir("resume");
    {
        let mut store = Store::open(&dir).expect("open");
        store
            .save_builder_sessions("t1", &every_phase())
            .expect("save");
        // The process dies here with 8#1 running and 9#1 admitted.
    }
    let mut store = Store::open(&dir).expect("reopen");
    let loaded = store.load_builder_sessions("t1").expect("load");
    let recovered = builder::recover(loaded, 5_000);
    assert!(
        recovered.iter().all(|s| !s.live()),
        "nothing is still running"
    );
    assert_eq!(
        recovered[3].phase,
        Phase::Ended {
            end: End::Aborted,
            at: 5_000
        }
    );
    assert_eq!(recovered[0], every_phase()[0], "ended rows are untouched");
    store
        .save_builder_sessions("t1", &recovered)
        .expect("save recovered");
    drop(store);
    let store = Store::open(&dir).expect("reopen again");
    assert_eq!(store.load_builder_sessions("t1").expect("load"), recovered);
    drop(store);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn rows_that_do_not_check_out_are_refused_before_writing() {
    let dir = dir("refused");
    let mut store = Store::open(&dir).expect("open");
    let twice = vec![
        session(7, 1, Phase::Running),
        session(7, 1, Phase::Admitted),
    ];
    assert!(matches!(
        store.save_builder_sessions("t1", &twice),
        Err(StoreError::Corrupt(_))
    ));
    assert!(store.load_builder_sessions("t1").expect("load").is_empty());
    drop(store);
    let _ = std::fs::remove_dir_all(dir);
}
