//! The cost breaker (#598 gap 4): pause a task whose spend keeps rising
//! while its claims stop moving.
//!
//! A window is the stretch between two [`Mark`]s of one task: the meter's
//! calls and spend, and each claim's score and status from BBP. It closes at
//! `window_calls` calls or `window_spend` micro-dollars, whichever comes
//! first. A closed window trips when no claim settled in it and the claims'
//! scores moved by less than `min_score_move` in total. No state is kept
//! here: the runner holds the window's opening mark and judges each new one
//! against it, starting a new window whenever [`judge`] says it closed.
//!
//! `observe` (the default, card #598 Q4) only reports a would-trip; `enforce`
//! asks the runner to pause the task (`bbp pause --reason breaker`).

use std::collections::BTreeMap;

use rusty_bbp::{ClaimStatus, MsgId};

use crate::card::Card;

/// Whether a trip pauses the task.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Observe,
    Enforce,
}

/// The breaker's config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Breaker {
    pub window_calls: u32,
    /// In micro-dollars.
    pub window_spend: u64,
    pub min_score_move: u64,
    pub mode: Mode,
}

impl Default for Breaker {
    /// 40 calls or $5 per window, a total score move of 4, observe only.
    fn default() -> Self {
        Self {
            window_calls: 40,
            window_spend: 5_000_000,
            min_score_move: 4,
            mode: Mode::Observe,
        }
    }
}

/// One claim at a mark: its score and whether it has settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimMark {
    pub score: i64,
    pub settled: bool,
}

/// A task at one moment: the meter's calls and settled spend for it, and
/// every claim's score and status.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Mark {
    pub calls: u32,
    pub spent: u64,
    pub claims: BTreeMap<MsgId, ClaimMark>,
}

impl Mark {
    /// The mark for a task's meter account and its claim cards.
    pub fn of(calls: u32, spent: u64, cards: &[Card]) -> Self {
        let claims = cards
            .iter()
            .map(|c| {
                let settled = matches!(c.status, ClaimStatus::Accepted | ClaimStatus::Dropped);
                (
                    c.id,
                    ClaimMark {
                        score: c.score,
                        settled,
                    },
                )
            })
            .collect();
        Self {
            calls,
            spent,
            claims,
        }
    }
}

/// What the runner does after judging a mark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The window is still open; keep its opening mark.
    Open,
    /// The window closed with progress; open the next one at this mark.
    Healthy,
    /// Observe mode: log this and open the next window at this mark.
    WouldTrip(String),
    /// Enforce mode: pause the task with this detail.
    Pause(String),
}

/// Judge `now` against the window opened at `start`. Pure.
pub fn judge(b: &Breaker, start: &Mark, now: &Mark) -> Outcome {
    let calls = now.calls.saturating_sub(start.calls);
    let spent = now.spent.saturating_sub(start.spent);
    if calls < b.window_calls && spent < b.window_spend {
        return Outcome::Open;
    }
    if settled_in(start, now) {
        return Outcome::Healthy;
    }
    let moved = score_move(start, now);
    if moved >= b.min_score_move {
        return Outcome::Healthy;
    }
    let detail = format!(
        "cost breaker: {calls} calls and ${} with no claim settled and a total score move of {moved} (under {})",
        dollars(spent),
        b.min_score_move
    );
    match b.mode {
        Mode::Observe => Outcome::WouldTrip(detail),
        Mode::Enforce => Outcome::Pause(detail),
    }
}

/// True when a claim settled since `start` (a new claim counts from open).
fn settled_in(start: &Mark, now: &Mark) -> bool {
    now.claims
        .iter()
        .any(|(id, c)| c.settled && !start.claims.get(id).is_some_and(|s| s.settled))
}

/// The sum of |Δscore| over the claims at `now`; a new claim moved from 0.
fn score_move(start: &Mark, now: &Mark) -> u64 {
    now.claims
        .iter()
        .map(|(id, c)| {
            let was = start.claims.get(id).map_or(0, |s| s.score);
            c.score.abs_diff(was)
        })
        .fold(0u64, u64::saturating_add)
}

/// Micro-dollars as dollars with cents.
fn dollars(micro: u64) -> String {
    format!("{}.{:02}", micro / 1_000_000, micro % 1_000_000 / 10_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(calls: u32, spent: u64, claims: &[(u64, i64, bool)]) -> Mark {
        Mark {
            calls,
            spent,
            claims: claims
                .iter()
                .map(|&(id, score, settled)| (MsgId(id), ClaimMark { score, settled }))
                .collect(),
        }
    }

    fn enforce() -> Breaker {
        Breaker {
            mode: Mode::Enforce,
            ..Breaker::default()
        }
    }

    #[test]
    fn a_window_stays_open_under_both_limits() {
        let start = mark(0, 0, &[(1, 0, false)]);
        let now = mark(39, 4_999_999, &[(1, 0, false)]);
        assert_eq!(judge(&enforce(), &start, &now), Outcome::Open);
    }

    #[test]
    fn a_flat_window_trips_at_the_call_limit_or_the_spend_limit() {
        let start = mark(10, 0, &[(1, 2, false), (2, -1, false)]);
        let by_calls = mark(50, 0, &[(1, 3, false), (2, -3, false)]);
        let Outcome::Pause(d) = judge(&enforce(), &start, &by_calls) else {
            panic!("trips")
        };
        assert!(
            d.contains("40 calls") && d.contains("score move of 3"),
            "{d}"
        );
        let by_spend = mark(11, 5_000_000, &[(1, 2, false), (2, -1, false)]);
        assert!(matches!(
            judge(&enforce(), &start, &by_spend),
            Outcome::Pause(d) if d.contains("$5.00")
        ));
    }

    #[test]
    fn observe_mode_reports_a_would_trip() {
        let start = mark(0, 0, &[]);
        let now = mark(40, 0, &[]);
        assert!(matches!(
            judge(&Breaker::default(), &start, &now),
            Outcome::WouldTrip(_)
        ));
    }

    #[test]
    fn a_settled_claim_or_enough_score_movement_is_progress() {
        let start = mark(0, 0, &[(1, 7, false)]);
        let settled = mark(40, 0, &[(1, 8, true)]);
        assert_eq!(judge(&enforce(), &start, &settled), Outcome::Healthy);
        let moved = mark(40, 0, &[(1, 5, false), (2, 2, false)]);
        assert_eq!(judge(&enforce(), &start, &moved), Outcome::Healthy);
        let new_settled = mark(40, 0, &[(1, 7, false), (3, 0, true)]);
        assert_eq!(judge(&enforce(), &start, &new_settled), Outcome::Healthy);
    }

    #[test]
    fn a_claim_settled_before_the_window_is_not_progress_in_it() {
        let start = mark(0, 0, &[(1, 8, true)]);
        let now = mark(40, 0, &[(1, 8, true)]);
        assert!(matches!(judge(&enforce(), &start, &now), Outcome::Pause(_)));
    }
}
