use std::time::{Duration, Instant};

use super::*;
use crate::{State, parse};

fn state(status: &Status) -> Vec<State> {
    status.shown().iter().map(|(_, r)| r.state).collect()
}

fn report(body: &str) -> Report {
    parse(body).expect("valid")
}

#[test]
fn without_hooks_the_program_is_shown() {
    let mut status = Status::default();
    status.program(report("state=working"), Instant::now());
    assert_eq!(state(&status), [State::Working]);
}

#[test]
fn a_hook_report_wins_over_the_program() {
    let now = Instant::now();
    let mut status = Status::default();
    status.program(report("state=working"), now);
    status.hook(report("state=blocked:kind=permission"), now);
    status.program(report("state=idle"), now + Duration::from_secs(1));
    assert_eq!(state(&status), [State::Blocked]);
}

#[test]
fn the_program_takes_over_only_after_silence_and_a_new_state() {
    let now = Instant::now();
    let later = now + HOOK_HOLD;
    let mut status = Status::default();
    status.hook(report("state=idle"), now);
    // Silent long enough, but the program agrees: the hooks keep it.
    status.program(report("state=idle"), later);
    status.hook(report("state=idle"), now);
    // Disagreeing too soon changes nothing.
    status.program(report("state=working"), later - Duration::from_secs(1));
    assert_eq!(state(&status), [State::Idle]);
    // Disagreeing after the hold takes the block.
    status.program(report("state=working"), later);
    assert_eq!(state(&status), [State::Working]);
    // The next hook report takes it back.
    status.hook(report("state=done"), later);
    assert_eq!(state(&status), [State::Done]);
}

#[test]
fn hooks_with_no_records_leave_the_program_shown() {
    let now = Instant::now();
    let mut status = Status::default();
    status.program(report("state=error"), now);
    status.hook(report("state=clear"), now);
    assert_eq!(state(&status), [State::Error]);
}

#[test]
fn prompt_and_reset_reach_both_producers() {
    let now = Instant::now();
    let mut status = Status::default();
    status.program(report("state=done"), now);
    status.hook(report("state=working"), now);
    status.prompt();
    // The hooks have nothing left, so the program's done record shows.
    assert_eq!(state(&status), [State::Done]);
    status.hook(report("state=idle"), now);
    status.reset();
    assert!(status.shown().iter().next().is_none());
    status.program(report("state=working"), now);
    assert_eq!(state(&status), [State::Working]);
}
