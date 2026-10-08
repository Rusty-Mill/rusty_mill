//! FM-1 severity model: discrete Major/Minor fault events and the
//! driving-test verdict.
//!
//! The assessment guide's own aggregate shape (criteria spec, FM-1 row) is not
//! a weighted mean: *"up to 15 Minor faults; one Major fault = instant
//! failure"*. This module classifies the same engagement episodes the v1
//! dimensions score into discrete faults with a severity:
//!
//! - **Major** — the FM-2-shaped unrecoverable dive, all three conditions at
//!   once: committed as **last man** (no teammate goalside), against a ball
//!   the team does **not own**, on an **empty tank**. Beaten there, the play
//!   is unrecoverable — the guide's canonical instant-failure.
//! - **Minor** — the single-condition faults: the fueled last-man dive (F17),
//!   the covered-but-empty engagement (F4), and the double-commit (F9 — the
//!   2nd man joining a teammate's live challenge).
//!
//! Minors accumulate and only flip the verdict past the allowance; they are
//! *not* re-subtracted from the numeric score (the dimension averages already
//! price them in). Majors flip the verdict immediately and cap the headline
//! value (see [`crate::scoring::aggregate_with_faults`]).
//!
//! The classification thresholds deliberately mirror the extractor defaults
//! ([`crate::metrics::BoostEconomy::empty_boost`],
//! [`crate::metrics::CommitmentDiscipline::double_radius_uu`]) so a penalised
//! episode and a fault are the same event seen through two aggregates — keep
//! them in sync when calibrating.

use crate::context::{MatchContext, RelativePossession, Role};
use crate::metrics::{engagements, EngagementConfig};
use crate::PlayerId;

/// How bad a fault is under the guide's driving-test framing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Accumulates; tolerable up to [`SeverityConfig::minor_allowance`].
    Minor,
    /// One is an instant verdict failure and caps the headline value.
    Major,
}

/// One discrete fault event, timestamped so a UI can jump to it.
#[derive(Debug, Clone, PartialEq)]
pub struct Fault {
    /// Time of the committing frame (seconds).
    pub t: f32,
    pub severity: Severity,
    /// The criteria-spec row this fault instantiates (e.g. `"F17"`, `"FM-2"`).
    pub criterion: &'static str,
    pub detail: String,
}

/// Thresholds for fault classification and the verdict.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeverityConfig {
    pub engagement: EngagementConfig,
    /// Boost at or below which an engagement counts as entered empty — must
    /// match [`crate::metrics::BoostEconomy::empty_boost`].
    pub empty_boost: u8,
    /// Ball radius (uu) inside which the 2nd man counts as double-committed —
    /// must match [`crate::metrics::CommitmentDiscipline::double_radius_uu`].
    pub double_radius_uu: f32,
    /// Minor faults tolerated before the verdict fails. The guide states 15.
    pub minor_allowance: u32,
    /// Ceiling on the headline value once any Major is present. The guide
    /// gives no number ("instant failure"); 40.0 is an invented default — low
    /// enough that no Major-carrying match reads as good, high enough to keep
    /// ordering information below the cap.
    pub major_cap: f32,
}

impl Default for SeverityConfig {
    fn default() -> Self {
        Self {
            engagement: EngagementConfig::default(),
            empty_boost: 5,
            double_radius_uu: 1100.0,
            minor_allowance: 15,
            major_cap: 40.0,
        }
    }
}

/// The fault ledger for one player, rolled into the driving-test verdict.
#[derive(Debug, Clone, PartialEq)]
pub struct FaultSummary {
    pub minors: u32,
    pub majors: u32,
    pub verdict: Verdict,
    /// Every fault, sorted by time.
    pub faults: Vec<Fault>,
}

/// FM-1's pass/fail: `Pass` iff no Major and minors within the allowance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Fail,
}

impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Verdict::Pass => "PASS",
            Verdict::Fail => "FAIL",
        }
    }
}

impl FaultSummary {
    /// Roll a fault list into counts and a verdict.
    pub fn from_faults(mut faults: Vec<Fault>, minor_allowance: u32) -> Self {
        faults.sort_by(|a, b| a.t.total_cmp(&b.t));
        let majors = faults
            .iter()
            .filter(|f| f.severity == Severity::Major)
            .count() as u32;
        let minors = faults.len() as u32 - majors;
        let verdict = if majors == 0 && minors <= minor_allowance {
            Verdict::Pass
        } else {
            Verdict::Fail
        };
        Self {
            minors,
            majors,
            verdict,
            faults,
        }
    }

    /// The empty ledger — no faults, verdict `Pass`. What
    /// [`crate::scoring::aggregate`] carries when no severity pass was run.
    pub fn empty() -> Self {
        Self::from_faults(Vec::new(), 0)
    }
}

/// Classify `player`'s faults from the same engagement primitive the
/// dimensions score.
///
/// Each of the player's own episodes yields at most one fault (the Major's
/// conditions subsume both minors' — a fueled dive can't also be an empty
/// entry); double-commit minors come from the *teammate's* episodes, exactly
/// as [`crate::metrics::CommitmentDiscipline`] counts its opportunities.
pub fn faults(ctx: &MatchContext, player: PlayerId, cfg: &SeverityConfig) -> Vec<Fault> {
    let mut out = Vec::new();

    for ep in engagements(ctx, player, &cfg.engagement) {
        let last_man_dive =
            ep.possession_at_entry != RelativePossession::Ours && !ep.covered_at_entry;
        let empty = ep.boost_at_entry <= cfg.empty_boost;
        match (last_man_dive, empty) {
            (true, true) => out.push(Fault {
                t: ep.t_enter,
                severity: Severity::Major,
                criterion: "FM-2",
                detail: "dived as last man, empty tank, against a ball the team does not own"
                    .into(),
            }),
            (true, false) => out.push(Fault {
                t: ep.t_enter,
                severity: Severity::Minor,
                criterion: "F17",
                detail: "committed as last man: no teammate goalside at the challenge".into(),
            }),
            (false, true) => out.push(Fault {
                t: ep.t_enter,
                severity: Severity::Minor,
                criterion: "F4",
                detail: "engaged the ball with an empty tank".into(),
            }),
            (false, false) => {}
        }
    }

    // Double-commits: judged per *teammate* episode, from the cover man's side.
    let mut teammates: Vec<PlayerId> = Vec::new();
    for (_, frame) in ctx.frames() {
        for f in frame.teammates_of(player) {
            if !teammates.contains(&f.player) {
                teammates.push(f.player);
            }
        }
    }
    for mate in teammates {
        for ep in engagements(ctx, mate, &cfg.engagement) {
            let Some((_, entry_frame)) = ctx.frame(ep.enter_idx) else {
                continue;
            };
            let Some(me) = entry_frame.player(player) else {
                continue;
            };
            if me.role != Role::SecondMan {
                continue;
            }
            let joined = (ep.enter_idx..=ep.exit_idx).any(|idx| {
                ctx.frame(idx)
                    .and_then(|(_, f)| f.player(player).map(|m| m.dist_to_ball))
                    .is_some_and(|d| d <= cfg.double_radius_uu)
            });
            if joined {
                out.push(Fault {
                    t: ep.t_enter,
                    severity: Severity::Minor,
                    criterion: "F9",
                    detail: "double-commit: joined the ball during the teammate's challenge".into(),
                });
            }
        }
    }

    out.sort_by(|a, b| a.t.total_cmp(&b.t));
    out
}

// ---------------------------------------------------------------------------
// Tests — synthetic timelines built by hand (same shapes as metrics.rs).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{ContextConfig, MatchContext, PossessionSpan};
    use crate::{BallState, PlayerState, Pose, Quat, Team, Timeline, Vec3, WorldState};

    fn pos(x: f32, y: f32) -> Pose {
        Pose {
            position: Vec3::new(x, y, 17.0),
            rotation: Quat::IDENTITY,
        }
    }

    fn ball_at(x: f32, y: f32) -> BallState {
        BallState {
            pose: Pose {
                position: Vec3::new(x, y, 93.0),
                rotation: Quat::IDENTITY,
            },
            velocity: Vec3::ZERO,
        }
    }

    fn player_with_boost(id: u32, team: Team, p: Pose, boost: u8) -> PlayerState {
        PlayerState {
            player: PlayerId(id),
            team,
            pose: p,
            velocity: Vec3::ZERO,
            boost,
            demolished: false,
            name: None,
        }
    }

    /// Ball fixed at y=3000; player 0 (blue) at `challenger_y`, teammate parked
    /// at `(2000, teammate_y)`.
    fn frame(t: f32, challenger_y: f32, teammate_y: f32, challenger_boost: u8) -> WorldState {
        WorldState {
            t,
            ball: Some(ball_at(0.0, 3000.0)),
            players: vec![
                player_with_boost(0, Team::Blue, pos(0.0, challenger_y), challenger_boost),
                player_with_boost(1, Team::Blue, pos(2000.0, teammate_y), 33),
            ],
        }
    }

    /// One approach → challenge → retreat arc for player 0.
    fn approach_timeline(teammate_y: f32, boost: u8) -> Timeline {
        let ys = [
            -1000.0, 500.0, 1800.0, 2400.0, 2600.0, 2700.0, 2600.0, 2400.0, 800.0, -1000.0,
        ];
        ys.iter()
            .enumerate()
            .map(|(i, y)| frame(i as f32 * 0.1, *y, teammate_y, boost))
            .collect()
    }

    fn ctx(timeline: &Timeline) -> MatchContext<'_> {
        MatchContext::derive(timeline, ContextConfig::default())
    }

    // -- classification -------------------------------------------------------

    #[test]
    fn empty_uncovered_dive_is_the_major() {
        // Teammate upfield (no cover), tank empty: FM-2's compound shape.
        let timeline = approach_timeline(4000.0, 0);
        let fs = faults(&ctx(&timeline), PlayerId(0), &SeverityConfig::default());
        assert_eq!(fs.len(), 1);
        assert_eq!(fs[0].severity, Severity::Major);
        assert_eq!(fs[0].criterion, "FM-2");
    }

    #[test]
    fn fueled_last_man_dive_is_minor_f17() {
        let timeline = approach_timeline(4000.0, 60);
        let fs = faults(&ctx(&timeline), PlayerId(0), &SeverityConfig::default());
        assert_eq!(fs.len(), 1, "one episode, one fault");
        assert_eq!(fs[0].severity, Severity::Minor);
        assert_eq!(fs[0].criterion, "F17");
    }

    #[test]
    fn covered_empty_engagement_is_minor_f4() {
        let timeline = approach_timeline(-2000.0, 0); // teammate goalside
        let fs = faults(&ctx(&timeline), PlayerId(0), &SeverityConfig::default());
        assert_eq!(fs.len(), 1);
        assert_eq!(fs[0].severity, Severity::Minor);
        assert_eq!(fs[0].criterion, "F4");
    }

    #[test]
    fn clean_covered_fueled_engagement_is_faultless() {
        let timeline = approach_timeline(-2000.0, 60);
        let fs = faults(&ctx(&timeline), PlayerId(0), &SeverityConfig::default());
        assert!(fs.is_empty());
    }

    #[test]
    fn own_possession_downgrades_the_dive_to_f4() {
        // The uncovered empty entry, but blue owns the ball throughout —
        // playing your own ball is not a dive, yet the empty tank still counts.
        let timeline = approach_timeline(4000.0, 0);
        let spans = [PossessionSpan {
            team: Team::Blue,
            start: 0.0,
            end: 10.0,
        }];
        let c = MatchContext::derive_with_possession(&timeline, ContextConfig::default(), &spans);
        let fs = faults(&c, PlayerId(0), &SeverityConfig::default());
        assert_eq!(fs.len(), 1);
        assert_eq!(fs[0].criterion, "F4");
        assert_eq!(fs[0].severity, Severity::Minor);
    }

    // -- double-commit --------------------------------------------------------

    /// Teammate (player 1) sits engaged on the ball; player 0 covers at
    /// `(cover_x, cover_y)`.
    fn teammate_engaged(t: f32, cover_x: f32, cover_y: f32) -> WorldState {
        WorldState {
            t,
            ball: Some(ball_at(0.0, 3000.0)),
            players: vec![
                player_with_boost(0, Team::Blue, pos(cover_x, cover_y), 33),
                player_with_boost(1, Team::Blue, pos(0.0, 2600.0), 33),
            ],
        }
    }

    #[test]
    fn joining_the_teammates_challenge_is_minor_f9() {
        let mut timeline: Timeline = (0..4)
            .map(|i| teammate_engaged(i as f32 * 0.1, 2000.0, -2000.0))
            .collect();
        timeline.extend((4..10).map(|i| teammate_engaged(i as f32 * 0.1, 300.0, 2900.0)));
        let fs = faults(&ctx(&timeline), PlayerId(0), &SeverityConfig::default());
        assert_eq!(fs.iter().filter(|f| f.criterion == "F9").count(), 1);
        assert!(fs.iter().all(|f| f.severity == Severity::Minor));
    }

    #[test]
    fn patient_cover_man_is_faultless() {
        let timeline: Timeline = (0..10)
            .map(|i| teammate_engaged(i as f32 * 0.1, 2000.0, -2000.0))
            .collect();
        let fs = faults(&ctx(&timeline), PlayerId(0), &SeverityConfig::default());
        assert!(fs.is_empty());
    }

    // -- verdict --------------------------------------------------------------

    fn minor(t: f32) -> Fault {
        Fault {
            t,
            severity: Severity::Minor,
            criterion: "F17",
            detail: String::new(),
        }
    }

    #[test]
    fn verdict_tolerates_the_allowance_and_fails_past_it() {
        let allowance = SeverityConfig::default().minor_allowance;
        assert_eq!(allowance, 15, "the guide's stated number");

        let at = FaultSummary::from_faults((0..15).map(|i| minor(i as f32)).collect(), allowance);
        assert_eq!(at.verdict, Verdict::Pass, "15 minors are tolerable");
        assert_eq!((at.minors, at.majors), (15, 0));

        let past = FaultSummary::from_faults((0..16).map(|i| minor(i as f32)).collect(), allowance);
        assert_eq!(past.verdict, Verdict::Fail, "the 16th minor fails");
    }

    #[test]
    fn one_major_is_instant_failure() {
        let one = FaultSummary::from_faults(
            vec![Fault {
                t: 0.0,
                severity: Severity::Major,
                criterion: "FM-2",
                detail: String::new(),
            }],
            15,
        );
        assert_eq!(one.verdict, Verdict::Fail);
        assert_eq!((one.minors, one.majors), (0, 1));
    }

    #[test]
    fn empty_ledger_passes_and_faults_sort_by_time() {
        assert_eq!(FaultSummary::empty().verdict, Verdict::Pass);
        let fs = FaultSummary::from_faults(vec![minor(5.0), minor(1.0)], 15);
        assert!(fs.faults[0].t < fs.faults[1].t);
    }
}
