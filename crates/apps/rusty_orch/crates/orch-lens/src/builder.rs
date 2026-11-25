//! Builder sessions (Let Em Loose #600, card #1404; design §4 to §6,
//! approved as D66; defaults D177): one Claude CLI session builds one
//! candidate for one design claim. Pure: config, a claim card and the
//! board's sessions in, an admitted session or a named refusal out.
//!
//! A builder is not a lens call: it has no provider fallback, no reply
//! protocol and no blind or informed phase, so it is its own job kind here
//! rather than a [`crate::schedule::Slot`]. Launching it (worktree, jail,
//! mux, slot) is #1318's.
//!
//! Admission (design §4.1, §5) refuses a new session when any of these
//! holds, checked in this order:
//!
//! - the claim is not open;
//! - the claim has a live candidate: its latest stored candidate's run has
//!   not failed. The card's own candidate (`Card::current`, BBP's view) wins
//!   over the session rows, so a candidate BBP stored and debited before orch
//!   saved its session still counts; the caller builds the card with that
//!   candidate visible;
//! - the claim already has `builders_per_claim` live sessions;
//! - `max_live_builders` sessions are live across the board;
//! - the claim's budget is spent (`spent < budget` fails). This is BBP's own
//!   budget refusal read ahead; there is no new budget rule.
//!
//! A session that ends without storing a candidate debits nothing (design
//! §4.7, §7), so the next admission simply starts a fresh one.
//!
//! Admission is check-then-insert: it is only safe while one caller holds
//! the store from loading the board's sessions through saving the admitted
//! one. orch-store's directory lock gives that today.
//!
//! `builder_wall_secs` is carried here; enforcing it is the launcher's
//! (#1318).

use std::collections::BTreeSet;
use std::fmt;

use rusty_bbp::{ArtId, ClaimStatus, MsgId};

use crate::card::Card;

/// The builder limits, from the lens config's optional `builders` block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Builders {
    /// Live sessions one claim may have at once.
    pub per_claim: u32,
    /// Live sessions at once across a board.
    pub max_live: u32,
    /// A session's wall clock, in seconds.
    pub wall_secs: u64,
}

impl Default for Builders {
    /// One builder per design claim (the Architect already proposes three
    /// designs), three live at once, 45 minutes each (design §5, §10; D177).
    fn default() -> Self {
        Self {
            per_claim: 1,
            max_live: 3,
            wall_secs: 2700,
        }
    }
}

/// How a session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    /// It stored this candidate, which names the claim (C-R4); the run
    /// started and was debited in the same transaction.
    Stored(ArtId),
    /// It gave up, or its wall clock ran out, with no candidate.
    Failed,
    /// A crash or restart cut it off; it debits nothing.
    Aborted,
}

/// Where a session is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Admitted; not launched yet.
    Admitted,
    /// Its process is running.
    Running,
    /// Ended at `at` (ms).
    Ended { end: End, at: u64 },
}

/// One frozen build profile's result on the session's candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileResult {
    pub profile: String,
    pub passed: bool,
}

/// What the runner reported for the session's candidate (design §6). An
/// `error` report is not recorded here: it changes nothing (addendum
/// § Failing runs block) and orch reruns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunResult {
    pub profiles: Vec<ProfileResult>,
    /// The erosion profile's delta, when it ran.
    pub erosion_delta: Option<i64>,
}

impl RunResult {
    /// True when any profile failed.
    pub fn failed(&self) -> bool {
        self.profiles.iter().any(|p| !p.passed)
    }
}

/// One builder session: a row of orch-store's `builder_session` table.
/// Wall time is `at - started` of its end; whether its claim was later
/// accepted, reversed or reopened, and whether a human touched it (D106),
/// are read from the claim fold, not stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub claim: MsgId,
    /// Its number within the claim, from 1.
    pub n: u32,
    /// Admitted at (ms).
    pub started: u64,
    pub phase: Phase,
    /// Tokens in and out so far, from the session's ACP usage.
    pub tokens_in: u64,
    pub tokens_out: u64,
    /// The run on its candidate, once reported.
    pub run: Option<RunResult>,
}

impl Session {
    /// True until the session ends.
    pub fn live(&self) -> bool {
        !matches!(self.phase, Phase::Ended { .. })
    }

    /// The candidate it stored, if any.
    pub fn candidate(&self) -> Option<ArtId> {
        match self.phase {
            Phase::Ended {
                end: End::Stored(a),
                ..
            } => Some(a),
            _ => None,
        }
    }

    /// The admitted session moves to running.
    ///
    /// # Errors
    ///
    /// When it is not in [`Phase::Admitted`].
    pub fn launch(self) -> Result<Self, Transition> {
        match self.phase {
            Phase::Admitted => Ok(Self {
                phase: Phase::Running,
                ..self
            }),
            _ => Err(Transition::new(&self, "launch")),
        }
    }

    /// The live session ends at `at` (ms), no earlier than it started.
    ///
    /// # Errors
    ///
    /// When it has already ended, when `at` is before `started`, or when it
    /// stores a candidate without having been launched.
    pub fn end(self, end: End, at: u64) -> Result<Self, Transition> {
        let unlaunched_store = matches!(end, End::Stored(_)) && self.phase != Phase::Running;
        if !self.live() || at < self.started || unlaunched_store {
            return Err(Transition::new(&self, "end"));
        }
        Ok(Self {
            phase: Phase::Ended { end, at },
            ..self
        })
    }

    /// Record the run reported on the session's candidate.
    ///
    /// # Errors
    ///
    /// When the session stored no candidate.
    pub fn report(self, run: RunResult) -> Result<Self, Transition> {
        if self.candidate().is_none() {
            return Err(Transition::new(&self, "report"));
        }
        Ok(Self {
            run: Some(run),
            ..self
        })
    }

    /// A stored candidate whose run has not failed (pending or passed).
    fn live_candidate(&self) -> bool {
        self.candidate().is_some() && !self.run.as_ref().is_some_and(RunResult::failed)
    }
}

/// A session asked to do something its phase does not allow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    pub claim: MsgId,
    pub n: u32,
    pub what: &'static str,
}

impl Transition {
    fn new(s: &Session, what: &'static str) -> Self {
        Self {
            claim: s.claim,
            n: s.n,
            what,
        }
    }
}

impl fmt::Display for Transition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "builder session {} #{}: cannot {} in its phase",
            self.claim, self.n, self.what
        )
    }
}

impl std::error::Error for Transition {}

/// Why a new session was refused; each names its limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The claim is not open.
    NotOpen(ClaimStatus),
    /// The claim's latest candidate is stored and its run has not failed.
    LiveCandidate(ArtId),
    /// The claim has `builders_per_claim` live sessions.
    PerClaim { limit: u32 },
    /// `max_live_builders` sessions are live.
    Live { limit: u32 },
    /// `spent < budget` fails; BBP would refuse the run as
    /// `budget_exhausted`.
    Budget { spent: u64, budget: u64 },
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotOpen(s) => write!(f, "claim is {}", s.name()),
            Self::LiveCandidate(a) => write!(f, "claim has a live candidate {a}"),
            Self::PerClaim { limit } => write!(f, "builders_per_claim ({limit}) live"),
            Self::Live { limit } => write!(f, "max_live_builders ({limit}) live"),
            Self::Budget { spent, budget } => {
                write!(f, "budget exhausted: {spent} of {budget} spent")
            }
        }
    }
}

impl std::error::Error for Refusal {}

/// Admit one more builder session for `card`'s claim at `now` (ms), given
/// every session of the board.
///
/// # Errors
///
/// The first [`Refusal`] that holds, in the order the module lists them.
pub fn admit(
    limits: &Builders,
    card: &Card,
    sessions: &[Session],
    now: u64,
) -> Result<Session, Refusal> {
    if card.status != ClaimStatus::Open {
        return Err(Refusal::NotOpen(card.status));
    }
    let mine: Vec<&Session> = sessions.iter().filter(|s| s.claim == card.id).collect();
    if let Some(a) = live_candidate(card, &mine) {
        return Err(Refusal::LiveCandidate(a));
    }
    if live(mine.iter().copied()) >= limits.per_claim {
        return Err(Refusal::PerClaim {
            limit: limits.per_claim,
        });
    }
    if live(sessions.iter()) >= limits.max_live {
        return Err(Refusal::Live {
            limit: limits.max_live,
        });
    }
    if card.spent >= card.budget {
        return Err(Refusal::Budget {
            spent: card.spent,
            budget: card.budget,
        });
    }
    Ok(Session {
        claim: card.id,
        n: mine
            .iter()
            .map(|s| s.n)
            .max()
            .unwrap_or(0)
            .saturating_add(1),
        started: now,
        phase: Phase::Admitted,
        tokens_in: 0,
        tokens_out: 0,
        run: None,
    })
}

/// The claim's latest candidate, when its run has not failed: the card's
/// own when it has one, else the latest storing session's.
fn live_candidate(card: &Card, mine: &[&Session]) -> Option<ArtId> {
    match card.current {
        Some(a) => (!mine.iter().any(|s| s.candidate() == Some(a) && failed(s))).then_some(a),
        None => latest_live_session(mine),
    }
}

fn failed(s: &Session) -> bool {
    s.run.as_ref().is_some_and(RunResult::failed)
}

/// The candidate of the claim's latest storing session, when it is live.
fn latest_live_session(mine: &[&Session]) -> Option<ArtId> {
    let latest = mine
        .iter()
        .filter(|s| s.candidate().is_some())
        .max_by_key(|s| s.n)?;
    latest
        .live_candidate()
        .then(|| latest.candidate())
        .flatten()
}

fn live<'a>(sessions: impl Iterator<Item = &'a Session>) -> u32 {
    let n = sessions.filter(|s| s.live()).count();
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// After a restart nothing is still running: every live session ends as
/// aborted at `now` (ms). Its own row debits nothing, but BBP may have
/// stored and debited its candidate just before the crash; [`admit`] reads
/// that from the card, so no second builder joins it.
pub fn recover(sessions: Vec<Session>, now: u64) -> Vec<Session> {
    sessions
        .into_iter()
        .map(|s| match s.live() {
            true => Session {
                phase: Phase::Ended {
                    end: End::Aborted,
                    at: now,
                },
                ..s
            },
            false => s,
        })
        .collect()
}

/// Check a board's sessions as loaded: every `n` is at least 1, `(claim,
/// n)` is unique, and only a session with a candidate has a run.
///
/// # Errors
///
/// A message naming the first bad row.
pub fn check(sessions: &[Session]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for s in sessions {
        if s.n == 0 {
            return Err(format!("builder session {} has number 0", s.claim));
        }
        if !seen.insert((s.claim, s.n)) {
            return Err(format!("builder session {} #{} twice", s.claim, s.n));
        }
        if let Phase::Ended { at, .. } = s.phase {
            if at < s.started {
                return Err(format!(
                    "builder session {} #{} ends before it starts",
                    s.claim, s.n
                ));
            }
        }
        if s.run.is_some() && s.candidate().is_none() {
            return Err(format!(
                "builder session {} #{} has a run but no candidate",
                s.claim, s.n
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::fixtures;

    fn session(claim: u64, n: u32, phase: Phase) -> Session {
        Session {
            claim: MsgId(claim),
            n,
            started: 1,
            phase,
            tokens_in: 0,
            tokens_out: 0,
            run: None,
        }
    }

    fn ended(claim: u64, n: u32, end: End) -> Session {
        session(claim, n, Phase::Ended { end, at: 5 })
    }

    fn stored(claim: u64, n: u32, art: u64, passed: Option<bool>) -> Session {
        Session {
            run: passed.map(|p| RunResult {
                profiles: vec![ProfileResult {
                    profile: "suite:unit".to_owned(),
                    passed: p,
                }],
                erosion_delta: None,
            }),
            ..ended(claim, n, End::Stored(ArtId(art)))
        }
    }

    fn claim(spent: u64, budget: u64) -> Card {
        let mut c = fixtures::card(vec![]);
        c.spent = spent;
        c.budget = budget;
        c
    }

    fn admit_default(card: &Card, sessions: &[Session]) -> Result<Session, Refusal> {
        admit(&Builders::default(), card, sessions, 42)
    }

    fn wide() -> Builders {
        Builders {
            per_claim: 4,
            max_live: 4,
            ..Builders::default()
        }
    }

    #[test]
    fn defaults_are_the_approved_ones() {
        let d = Builders::default();
        assert_eq!((d.per_claim, d.max_live, d.wall_secs), (1, 3, 2700));
    }

    #[test]
    fn first_session_is_number_one_and_admitted() {
        let s = admit_default(&claim(0, 16), &[]).unwrap();
        assert_eq!(
            s,
            Session {
                started: 42,
                ..session(7, 1, Phase::Admitted)
            }
        );
    }

    #[test]
    fn only_open_claims_get_builders() {
        let mut c = claim(0, 16);
        c.status = ClaimStatus::Accepted;
        assert_eq!(
            admit_default(&c, &[]),
            Err(Refusal::NotOpen(ClaimStatus::Accepted))
        );
    }

    #[test]
    fn a_claim_with_a_live_candidate_gets_no_second_builder() {
        let pending = [stored(7, 1, 3, None)];
        assert_eq!(
            admit(&wide(), &claim(1, 16), &pending, 9),
            Err(Refusal::LiveCandidate(ArtId(3)))
        );
        let passed = [stored(7, 1, 3, Some(true))];
        assert_eq!(
            admit(&wide(), &claim(1, 16), &passed, 9),
            Err(Refusal::LiveCandidate(ArtId(3)))
        );
    }

    #[test]
    fn a_failed_latest_candidate_is_retried() {
        let s = [stored(7, 1, 3, Some(false))];
        assert_eq!(admit_default(&claim(1, 16), &s).unwrap().n, 2);
        // Only the latest candidate decides.
        let s = [stored(7, 1, 3, Some(true)), stored(7, 2, 4, Some(false))];
        assert_eq!(admit_default(&claim(2, 16), &s).unwrap().n, 3);
        let s = [stored(7, 1, 3, Some(false)), stored(7, 2, 4, None)];
        assert_eq!(
            admit_default(&claim(2, 16), &s),
            Err(Refusal::LiveCandidate(ArtId(4)))
        );
    }

    #[test]
    fn a_session_without_a_candidate_leaves_the_claim_free() {
        let s = [ended(7, 1, End::Failed), ended(7, 2, End::Aborted)];
        assert_eq!(admit_default(&claim(0, 16), &s).unwrap().n, 3);
    }

    #[test]
    fn per_claim_limit_under_at_and_over() {
        let l = Builders {
            per_claim: 2,
            max_live: 9,
            ..Builders::default()
        };
        let one = [session(7, 1, Phase::Running)];
        assert_eq!(admit(&l, &claim(0, 16), &one, 9).unwrap().n, 2);
        let two = [one[0].clone(), session(7, 2, Phase::Admitted)];
        assert_eq!(
            admit(&l, &claim(0, 16), &two, 9),
            Err(Refusal::PerClaim { limit: 2 })
        );
        let three = [
            two[0].clone(),
            two[1].clone(),
            session(7, 3, Phase::Running),
        ];
        assert_eq!(
            admit(&l, &claim(0, 16), &three, 9),
            Err(Refusal::PerClaim { limit: 2 })
        );
    }

    #[test]
    fn max_live_under_at_and_over() {
        let two = [
            session(8, 1, Phase::Running),
            session(9, 1, Phase::Admitted),
        ];
        assert!(admit_default(&claim(0, 16), &two).is_ok());
        let three = [
            two[0].clone(),
            two[1].clone(),
            session(10, 1, Phase::Running),
        ];
        assert_eq!(
            admit_default(&claim(0, 16), &three),
            Err(Refusal::Live { limit: 3 })
        );
        let four = [
            three[0].clone(),
            three[1].clone(),
            three[2].clone(),
            session(11, 1, Phase::Running),
        ];
        assert_eq!(
            admit_default(&claim(0, 16), &four),
            Err(Refusal::Live { limit: 3 })
        );
    }

    #[test]
    fn ended_sessions_are_not_live() {
        let s = [
            ended(8, 1, End::Failed),
            ended(9, 1, End::Aborted),
            stored(10, 1, 3, None),
        ];
        assert!(admit_default(&claim(0, 16), &s).is_ok());
    }

    #[test]
    fn a_spent_budget_gets_none() {
        assert!(admit_default(&claim(15, 16), &[]).is_ok());
        assert_eq!(
            admit_default(&claim(16, 16), &[]),
            Err(Refusal::Budget {
                spent: 16,
                budget: 16
            })
        );
        assert_eq!(
            admit_default(&claim(17, 16), &[]),
            Err(Refusal::Budget {
                spent: 17,
                budget: 16
            })
        );
    }

    #[test]
    fn refusal_names_its_limit() {
        assert_eq!(
            Refusal::PerClaim { limit: 1 }.to_string(),
            "builders_per_claim (1) live"
        );
        assert_eq!(
            Refusal::Live { limit: 3 }.to_string(),
            "max_live_builders (3) live"
        );
        assert_eq!(
            Refusal::Budget {
                spent: 4,
                budget: 4
            }
            .to_string(),
            "budget exhausted: 4 of 4 spent"
        );
        assert_eq!(
            Refusal::NotOpen(ClaimStatus::Dropped).to_string(),
            "claim is DROPPED"
        );
    }

    #[test]
    fn transitions() {
        let s = session(7, 1, Phase::Admitted);
        let r = s.clone().launch().unwrap();
        assert_eq!(r.phase, Phase::Running);
        assert_eq!(r.clone().launch().unwrap_err().what, "launch");
        assert_eq!(
            r.clone()
                .report(stored(7, 1, 1, Some(true)).run.unwrap())
                .unwrap_err()
                .what,
            "report"
        );
        let e = r.end(End::Stored(ArtId(4)), 8).unwrap();
        assert_eq!(e.candidate(), Some(ArtId(4)));
        assert_eq!(e.clone().end(End::Failed, 9).unwrap_err().what, "end");
        let run = RunResult {
            profiles: vec![],
            erosion_delta: Some(-2),
        };
        assert_eq!(e.report(run.clone()).unwrap().run, Some(run));
        assert!(
            s.clone().end(End::Aborted, 3).is_ok(),
            "admitted can be aborted"
        );
        assert_eq!(
            s.clone().end(End::Stored(ArtId(4)), 3).unwrap_err().what,
            "end",
            "an unlaunched session stores nothing"
        );
        assert_eq!(
            s.end(End::Failed, 0).unwrap_err().what,
            "end",
            "before start"
        );
    }

    #[test]
    fn the_cards_candidate_counts_even_without_a_stored_row() {
        // BBP stored and debited art:9, then orch crashed before saving.
        let rows = recover(vec![session(7, 1, Phase::Running)], 50);
        let mut c = claim(1, 16);
        c.current = Some(ArtId(9));
        assert_eq!(
            admit_default(&c, &rows),
            Err(Refusal::LiveCandidate(ArtId(9)))
        );
    }

    #[test]
    fn the_cards_candidate_frees_the_claim_once_its_run_failed() {
        let mut c = claim(1, 16);
        c.current = Some(ArtId(3));
        let failed = [stored(7, 1, 3, Some(false))];
        assert_eq!(admit_default(&c, &failed).unwrap().n, 2);
        let passed = [stored(7, 1, 3, Some(true))];
        assert_eq!(
            admit_default(&c, &passed),
            Err(Refusal::LiveCandidate(ArtId(3)))
        );
    }

    #[test]
    fn recover_aborts_only_live_sessions() {
        let s = vec![
            session(7, 1, Phase::Running),
            session(7, 2, Phase::Admitted),
            ended(8, 1, End::Failed),
        ];
        let r = recover(s.clone(), 99);
        let aborted = Phase::Ended {
            end: End::Aborted,
            at: 99,
        };
        assert_eq!(r[0].phase, aborted);
        assert_eq!(r[1].phase, aborted);
        assert_eq!(r[2], s[2]);
        assert!(
            admit_default(&claim(0, 16), &r).is_ok(),
            "an abort frees the claim"
        );
    }

    #[test]
    fn check_refuses_bad_rows() {
        let ok = [session(7, 1, Phase::Running), session(8, 1, Phase::Running)];
        assert!(check(&ok).is_ok());
        let twice = [
            session(7, 1, Phase::Running),
            session(7, 1, Phase::Admitted),
        ];
        assert!(check(&twice).is_err());
        assert!(check(&[session(7, 0, Phase::Running)]).is_err());
        let orphan_run = Session {
            run: stored(7, 1, 1, Some(true)).run,
            ..ended(7, 1, End::Failed)
        };
        assert!(check(&[orphan_run]).is_err());
        let backwards = Session {
            started: 10,
            ..ended(7, 1, End::Failed)
        };
        assert!(check(&[backwards]).is_err(), "ends before it starts");
    }
}
