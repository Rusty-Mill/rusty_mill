//! #1318 S5: the builder sequence against fake ports: the end-path order,
//! a refused fd hand-off, a lost mux, the wall clock, the D188 submit rule
//! and the rows saved at each step.

use std::time::Duration;

use orch_lens::builder::{End, Phase, Session};
use rusty_bbp::{ArtId, MsgId};

use crate::acp::{StopReason, Turn, Usage};
use crate::builder::{self, Outcome, Ports, Ran, Refused, GRACE, HOLDER_MARGIN};

/// Scripted answers, and every call in order.
struct Fake {
    calls: Vec<String>,
    saved: Vec<Phase>,
    prepare: Result<(), String>,
    hold: Result<(), String>,
    open: Result<(), Refused>,
    ran: Ran,
    empty: bool,
    submit: Result<Option<ArtId>, String>,
    holder_deadline: Option<u64>,
    turn_deadline: Option<u64>,
    term: Result<(), Refused>,
    kill: Result<(), String>,
}

impl Default for Fake {
    fn default() -> Self {
        Self {
            calls: Vec::new(),
            saved: Vec::new(),
            prepare: Ok(()),
            hold: Ok(()),
            open: Ok(()),
            ran: Ran::Turn(turn("end_turn")),
            empty: true,
            submit: Ok(Some(ArtId(9))),
            holder_deadline: None,
            turn_deadline: None,
            term: Ok(()),
            kill: Ok(()),
        }
    }
}

fn turn(stop: &str) -> Turn {
    Turn {
        stop: StopReason(stop.into()),
        usage: Some(Usage {
            tokens_in: 120,
            tokens_out: 30,
        }),
    }
}

impl Ports for Fake {
    fn now_ms(&mut self) -> u64 {
        5_000
    }
    fn save(&mut self, s: &Session) -> Result<(), String> {
        self.calls.push("save".into());
        self.saved.push(s.phase);
        Ok(())
    }
    fn prepare(&mut self, _: &Session) -> Result<(), String> {
        self.calls.push("prepare".into());
        self.prepare.clone()
    }
    fn hold(&mut self, deadline_ms: u64) -> Result<(), String> {
        self.calls.push("hold".into());
        self.holder_deadline = Some(deadline_ms);
        self.hold.clone()
    }
    fn open(&mut self) -> Result<(), Refused> {
        self.calls.push("open".into());
        self.open.clone()
    }
    fn turn(&mut self, deadline_ms: u64) -> Ran {
        self.calls.push("turn".into());
        self.turn_deadline = Some(deadline_ms);
        self.ran.clone()
    }
    fn term(&mut self) -> Result<(), Refused> {
        self.calls.push("term".into());
        self.term.clone()
    }
    fn empty_within(&mut self, grace: Duration) -> Result<bool, String> {
        assert_eq!(grace, GRACE);
        self.calls.push("grace".into());
        Ok(self.empty)
    }
    fn kill(&mut self) -> Result<(), String> {
        self.calls.push("kill".into());
        self.kill.clone()
    }
    fn release(&mut self) -> Result<(), String> {
        self.calls.push("release".into());
        Ok(())
    }
    fn submit(&mut self) -> Result<Option<ArtId>, String> {
        self.calls.push("submit".into());
        self.submit.clone()
    }
}

fn admitted() -> Session {
    Session {
        claim: MsgId(7),
        n: 1,
        started: 1_000,
        phase: Phase::Admitted,
        tokens_in: 0,
        tokens_out: 0,
        run: None,
    }
}

const WALL: Duration = Duration::from_secs(2);

fn run(fake: &mut Fake) -> Outcome {
    builder::run(fake, admitted(), WALL).expect("admitted session runs")
}

fn ended(end: End) -> Phase {
    Phase::Ended { end, at: 5_000 }
}

#[test]
fn a_submitted_candidate_ends_stored_after_the_end_path() {
    let mut fake = Fake::default();
    let out = run(&mut fake);
    assert_eq!(out.session.phase, ended(End::Stored(ArtId(9))));
    assert_eq!((out.session.tokens_in, out.session.tokens_out), (120, 30));
    assert!(out.notes.is_empty(), "{:?}", out.notes);
    assert_eq!(
        fake.calls,
        ["prepare", "hold", "open", "save", "turn", "term", "grace", "release", "submit", "save"]
    );
    assert_eq!(fake.saved, [Phase::Running, ended(End::Stored(ArtId(9)))]);
    // orch ends at the wall; the holder's kill waits past orch's grace.
    assert_eq!(fake.turn_deadline, Some(3_000));
    let margin = (GRACE + HOLDER_MARGIN).as_millis() as u64;
    assert_eq!(fake.holder_deadline, Some(3_000 + margin));
}

#[test]
fn a_block_left_after_the_grace_is_killed_by_orch_before_release() {
    let mut fake = Fake {
        empty: false,
        ..Fake::default()
    };
    run(&mut fake);
    let end: Vec<&str> = fake.calls[5..9].iter().map(String::as_str).collect();
    assert_eq!(end, ["term", "grace", "kill", "release"]);
}

#[test]
fn a_refused_fd_hand_off_launches_nothing() {
    let mut fake = Fake {
        hold: Err("every agent slot is held".into()),
        ..Fake::default()
    };
    let out = run(&mut fake);
    assert_eq!(out.session.phase, ended(End::Failed));
    assert_eq!(fake.calls, ["prepare", "hold", "save"]);
    assert!(out.notes[0].contains("every agent slot"));
}

#[test]
fn a_failed_prepare_stops_before_the_holder() {
    let mut fake = Fake {
        prepare: Err("cgroup-open refused".into()),
        ..Fake::default()
    };
    let out = run(&mut fake);
    assert_eq!(out.session.phase, ended(End::Failed));
    assert_eq!(fake.calls, ["prepare", "save"]);
}

#[test]
fn a_lost_mux_ends_aborted_and_posts_nothing() {
    let mut fake = Fake {
        ran: Ran::Lost("own ended".into()),
        ..Fake::default()
    };
    let out = run(&mut fake);
    assert_eq!(out.session.phase, ended(End::Aborted));
    assert!(!fake.calls.contains(&"submit".to_owned()));
    assert!(fake.calls.contains(&"release".to_owned()));

    let mut fake = Fake {
        open: Err(Refused::Lost("reconnect failed".into())),
        ..Fake::default()
    };
    let out = run(&mut fake);
    assert_eq!(out.session.phase, ended(End::Aborted));
    assert_eq!(
        fake.calls,
        ["prepare", "hold", "open", "term", "grace", "release", "save"]
    );
}

#[test]
fn a_refused_launch_ends_failed_and_frees_the_holder() {
    let mut fake = Fake {
        open: Err(Refused::Failed("no window".into())),
        ..Fake::default()
    };
    let out = run(&mut fake);
    assert_eq!(out.session.phase, ended(End::Failed));
    assert!(fake
        .calls
        .ends_with(&["release".to_owned(), "save".to_owned()]));
}

#[test]
fn the_wall_clock_ends_failed_with_the_cgroup_killed() {
    let mut fake = Fake {
        ran: Ran::Wall,
        empty: false,
        ..Fake::default()
    };
    let out = run(&mut fake);
    assert_eq!(out.session.phase, ended(End::Failed));
    assert!(fake.calls.contains(&"kill".to_owned()));
    assert!(!fake.calls.contains(&"submit".to_owned()));
    assert_eq!(out.notes, ["wall clock ran out"]);
}

#[test]
fn only_end_turn_with_a_marker_and_a_diff_posts() {
    // Another stop reason: nothing is asked of BBP.
    let mut fake = Fake {
        ran: Ran::Turn(turn("max_tokens")),
        ..Fake::default()
    };
    let out = run(&mut fake);
    assert_eq!(out.session.phase, ended(End::Failed));
    assert!(!fake.calls.contains(&"submit".to_owned()));
    assert_eq!(out.session.tokens_in, 120, "usage is kept on a failed turn");

    // end_turn without the marker or with an empty diff: failed.
    let mut fake = Fake {
        submit: Ok(None),
        ..Fake::default()
    };
    assert_eq!(run(&mut fake).session.phase, ended(End::Failed));

    // A refused post: failed, with the reason noted.
    let mut fake = Fake {
        submit: Err("put_artifact: ProtectedPath".into()),
        ..Fake::default()
    };
    let out = run(&mut fake);
    assert_eq!(out.session.phase, ended(End::Failed));
    assert!(out.notes[0].contains("ProtectedPath"));
}

#[test]
fn a_broken_turn_ends_failed() {
    let mut fake = Fake {
        ran: Ran::Failed("closed".into()),
        ..Fake::default()
    };
    assert_eq!(run(&mut fake).session.phase, ended(End::Failed));
}

#[test]
fn only_an_admitted_session_runs() {
    let running = Session {
        phase: Phase::Running,
        ..admitted()
    };
    let mut fake = Fake::default();
    assert!(builder::run(&mut fake, running, WALL).is_err());
    assert!(
        fake.calls.is_empty(),
        "nothing is touched: {:?}",
        fake.calls
    );
}

#[test]
fn a_mux_lost_while_own_retried_ends_aborted_even_after_end_turn() {
    let mut fake = Fake {
        term: Err(Refused::Lost("connection lost".into())),
        ..Fake::default()
    };
    let out = run(&mut fake);
    assert_eq!(out.session.phase, ended(End::Aborted));
    assert!(!fake.calls.contains(&"submit".to_owned()), "nothing posted");
    assert!(fake.calls.contains(&"release".to_owned()));
}

#[test]
fn a_refused_term_is_noted_and_the_end_path_goes_on() {
    let mut fake = Fake {
        term: Err(Refused::Failed("no such block".into())),
        ..Fake::default()
    };
    let out = run(&mut fake);
    assert_eq!(out.session.phase, ended(End::Stored(ArtId(9))));
    assert_eq!(out.notes, ["term: no such block"]);
}

#[test]
fn a_failed_kill_after_end_turn_submits_nothing() {
    let mut fake = Fake {
        empty: false,
        kill: Err("cgroup still populated 10s after the kill".into()),
        ..Fake::default()
    };
    let out = run(&mut fake);
    assert_eq!(out.session.phase, ended(End::Failed));
    assert!(
        !fake.calls.contains(&"submit".to_owned()),
        "{:?}",
        fake.calls
    );
    assert!(fake.calls.contains(&"release".to_owned()));
    assert!(
        out.notes.iter().any(|n| n.contains("not confirmed empty")),
        "{:?}",
        out.notes
    );
}
