//! A lens's track record on one task: informational only (#599 Q2). It is
//! shown to the lens as one line and never weights a score.
//!
//! The rule (599-build-plan.md §2, D93, D92), per blind review job:
//!
//! - The job's direction is the net sign of its items; a job that posted
//!   nothing or posted both ways scores 0 and is not counted as scored.
//! - It is scored on the claim's latest settlement. `Void` and `Superseded`
//!   leave the record; open and escalated claims are pending.
//! - **Leave-one-out.** If removing the job's own items would undo the
//!   accept or drop line, the job was pivotal: a lens can't earn by causing
//!   the outcome it bet on. A pivotal job is scored against ground truth if
//!   the claim has one, and is otherwise unscored and counted as pivotal. A
//!   human `decide` or `choose` didn't depend on the score, so it is never
//!   pivotal; a later human reversal is that latest settlement.
//! - Right is `+w`, wrong `-w`, with `w = 1 + contested + 2·upset`.
//!   `contested`: other authors' scoring items both ways at settlement.
//!   `upset`: the score just before the job was non-zero and against the
//!   outcome it is scored on.
//!
//! The record is computed from stored [`Review`]s and each claim's [`Trail`]
//! whenever it is read; points are never stored.

use std::collections::{BTreeMap, BTreeSet};

/// Claim and message ids, as stored rows hold them; candidate ids.
pub use rusty_bbp::{ArtId, MsgId};
use rusty_bbp::{ClaimStatus, Direction};

/// Reviews before a lens's calibration report is due (Q2).
pub const CALIBRATION_AFTER: u32 = 20;

/// The profile prefix of a golden set (#597): Nano signs it off, so it is
/// ground truth. `holdout:` profiles are written by a model and are not.
const GOLDEN: &str = "golden:";

/// One blind review job a lens was assigned and answered, as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Review {
    pub lens: String,
    pub claim: MsgId,
    /// The claim's epoch when the job was made.
    pub epoch: u32,
    pub look_harder: bool,
    /// The evidence the job posted; empty for no post or a refused reply.
    pub items: Vec<MsgId>,
    /// A post's answer was lost, so the job may have posted more than
    /// `items`: its own contribution isn't known, and it is never scored
    /// against the board outcome.
    pub unsure: bool,
    /// Saved before the job's first post and cleared by the save after it:
    /// still set, the job may have posted under ids nobody knows, so it is
    /// scored like an unsure job (D92 write-ahead).
    pub pending: bool,
}

/// A claim's history as the record reads it: `bbp why --claim`, trimmed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Trail {
    pub steps: Vec<Step>,
}

/// One change to the claim and the claim just after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub change: Change,
    pub status: ClaimStatus,
    pub score: i64,
    pub epoch: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Item(Item),
    /// A status change and the rule behind it.
    Status(Rule),
    /// A final holdout report on the current code.
    Report(Report),
    Other,
}

/// A final holdout report on the claim's current code, with its profiles
/// that passed and failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub report: ArtId,
    pub passed: Vec<String>,
    pub failed: Vec<String>,
}

impl Report {
    /// What its golden profiles say: any failed means drop was right, all
    /// passed means accept was right; `None` without a golden profile.
    fn truth(&self) -> Option<Direction> {
        let golden = |ps: &[String]| ps.iter().any(|p| p.starts_with(GOLDEN));
        if golden(&self.failed) {
            return Some(Direction::Counter);
        }
        golden(&self.passed).then_some(Direction::Support)
    }
}

/// An evidence item on the claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub msg: MsgId,
    pub author: String,
    pub direction: Direction,
    /// Executed or cited: the kind a line needs (not an opinion).
    pub grounded: bool,
    /// The artifacts the item cites: a job citing a report can't be judged
    /// by that report.
    pub cites: Vec<ArtId>,
    pub delta: i64,
    pub scoring: bool,
}

/// Why the claim's status changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rule {
    /// The accept or drop line, with the line it crossed, the distinct
    /// authors behind it and the minimum it needed. `holdout` is the
    /// [`Report`] that still spoke for the current code when it settled:
    /// BBP checks its suite and tested tree then.
    Line {
        at: i64,
        authors: u32,
        min_authors: u32,
        holdout: Option<ArtId>,
    },
    /// A human `decide` or `choose`.
    Human,
    /// A reopen, and the earlier items it woke into the new epoch.
    Reopen(Vec<MsgId>),
    Other,
}

/// How one job scored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scored {
    /// The claim was voided or superseded: not in the record.
    Excluded,
    /// Not settled yet.
    Pending,
    /// Counted, but not scored: no direction, or pivotal with no ground truth.
    Unscored {
        pivotal: bool,
    },
    Points {
        points: i64,
        pivotal: bool,
    },
}

/// How `review` scores on `trail`, its claim's history.
pub fn score(review: &Review, trail: &Trail) -> Scored {
    let Some(last) = trail.steps.last() else {
        return Scored::Pending;
    };
    let outcome = match last.status {
        ClaimStatus::Accepted => Direction::Support,
        ClaimStatus::Dropped => Direction::Counter,
        ClaimStatus::Void | ClaimStatus::Superseded => return Scored::Excluded,
        ClaimStatus::Open | ClaimStatus::Escalated => return Scored::Pending,
    };
    let Some(settled) = settlement(trail) else {
        return Scored::Pending;
    };
    let own = Own::of(review, trail);
    // An unsure job's own items aren't all known, so neither are its
    // direction, citation or weight: it is never scored, and on a line
    // settlement it counts as pivotal (D92).
    if review.unsure || review.pending {
        return Scored::Unscored {
            pivotal: line_settled(trail, settled),
        };
    }
    let Some(direction) = own.direction(trail) else {
        return Scored::Unscored { pivotal: false };
    };
    let pivotal = pivotal(trail, settled, &own);
    let truth = if pivotal {
        match golden(trail, settled).filter(|(_, report)| !own.cites(trail, *report)) {
            Some((t, _)) => t,
            None => return Scored::Unscored { pivotal: true },
        }
    } else {
        outcome
    };
    let w = 1 + contested(trail, settled, &review.lens) + 2 * upset(trail, &own, truth);
    let points = if direction == truth { w } else { -w };
    Scored::Points { points, pivotal }
}

/// The latest status step that changed the status. An event that leaves it
/// as it was, such as an unconfirmed action's escalation of an accepted
/// claim, settles nothing.
fn settlement(trail: &Trail) -> Option<usize> {
    let steps = &trail.steps;
    (0..steps.len()).rev().find(|&n| {
        let changed = n == 0 || steps[n - 1].status != steps[n].status;
        changed && matches!(steps[n].change, Change::Status(_))
    })
}

/// The steps holding the job's own items.
struct Own {
    lens: String,
    steps: Vec<usize>,
}

impl Own {
    fn of(review: &Review, trail: &Trail) -> Self {
        let mine: BTreeSet<MsgId> = review.items.iter().copied().collect();
        let steps = trail
            .steps
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                matches!(&s.change, Change::Item(i) if mine.contains(&i.msg) && i.author == review.lens)
            })
            .map(|(n, _)| n)
            .collect();
        Self {
            lens: review.lens.clone(),
            steps,
        }
    }

    /// Whether one of the job's items cites `report`: a lens can't be
    /// judged by the report it read (D92).
    fn cites(&self, trail: &Trail, report: ArtId) -> bool {
        self.steps
            .iter()
            .any(|n| item(&trail.steps[*n]).is_some_and(|i| i.cites.contains(&report)))
    }

    fn has(&self, n: usize) -> bool {
        self.steps.contains(&n)
    }

    /// The net sign of the job's items; `None` for none, or both ways.
    fn direction(&self, trail: &Trail) -> Option<Direction> {
        let ways = Ways::of(
            self.steps
                .iter()
                .filter_map(|n| item(&trail.steps[*n]).map(|i| i.direction)),
        );
        match (ways.support, ways.counter) {
            (true, false) => Some(Direction::Support),
            (false, true) => Some(Direction::Counter),
            _ => None,
        }
    }
}

/// Which directions a set of items went.
#[derive(Default)]
struct Ways {
    support: bool,
    counter: bool,
}

impl Ways {
    fn of(directions: impl Iterator<Item = Direction>) -> Self {
        directions.fold(Self::default(), |w, d| match d {
            Direction::Support => Self { support: true, ..w },
            Direction::Counter => Self { counter: true, ..w },
        })
    }
}

fn item(s: &Step) -> Option<&Item> {
    match &s.change {
        Change::Item(i) => Some(i),
        _ => None,
    }
}

/// The scoring items live when step `settled` settled the claim, with
/// their step numbers: those posted scoring in its epoch, and any a reopen
/// into that epoch woke by name.
fn live(trail: &Trail, settled: usize) -> impl Iterator<Item = (usize, &Item)> {
    let woke = woken(trail, settled);
    trail.steps[..settled]
        .iter()
        .enumerate()
        .filter_map(|(n, s)| item(s).map(|i| (n, s, i)))
        .filter(move |(_, s, i)| {
            (s.epoch == trail.steps[settled].epoch && i.scoring) || woke.contains(&i.msg)
        })
        .map(|(n, _, i)| (n, i))
}

/// The items a reopen into the settlement's epoch woke.
fn woken(trail: &Trail, settled: usize) -> BTreeSet<MsgId> {
    let epoch = trail.steps[settled].epoch;
    trail.steps[..settled]
        .iter()
        .filter(|s| s.epoch == epoch)
        .filter_map(|s| match &s.change {
            Change::Status(Rule::Reopen(woke)) => Some(woke.iter().copied()),
            _ => None,
        })
        .flatten()
        .collect()
}

/// Whether the claim was settled by the accept or drop line.
fn line_settled(trail: &Trail, settled: usize) -> bool {
    matches!(
        trail.steps[settled].change,
        Change::Status(Rule::Line { .. })
    )
}

/// Whether the line that settled the claim would not have held without the
/// job's own items. A human settlement is never pivotal.
fn pivotal(trail: &Trail, settled: usize, own: &Own) -> bool {
    let step = &trail.steps[settled];
    let Change::Status(Rule::Line {
        at,
        authors,
        min_authors,
        ..
    }) = step.change
    else {
        return false;
    };
    let way = match step.status {
        ClaimStatus::Accepted => Direction::Support,
        _ => Direction::Counter,
    };
    let removed: i64 = live(trail, settled)
        .filter(|(n, _)| own.has(*n))
        .map(|(_, i)| i.delta)
        .sum();
    let others: Vec<&Item> = live(trail, settled)
        .filter(|(n, i)| !own.has(*n) && i.direction == way)
        .map(|(_, i)| i)
        .collect();
    let mine_that_way = live(trail, settled).any(|(n, i)| own.has(n) && i.direction == way);
    let lens_stays = others.iter().any(|i| i.author == own.lens);
    let authors = if mine_that_way && !lens_stays {
        authors.saturating_sub(1)
    } else {
        authors
    };
    let score = step.score - removed;
    let crossed = match way {
        Direction::Support => score >= at,
        Direction::Counter => score <= at,
    };
    let enough = authors >= min_authors;
    let grounded = others.iter().any(|i| i.grounded);
    !(crossed && enough && grounded)
}

/// Ground truth for a line settlement, and the report it comes from: the
/// golden profiles of the holdout report the settlement names, the one BBP
/// found still speaking for the current code then (D92, D112). None
/// without one, or when it has no golden profile.
fn golden(trail: &Trail, settled: usize) -> Option<(Direction, ArtId)> {
    let Change::Status(Rule::Line {
        holdout: Some(id), ..
    }) = &trail.steps[settled].change
    else {
        return None;
    };
    trail.steps[..settled].iter().find_map(|s| match &s.change {
        Change::Report(r) if r.report == *id => r.truth().map(|t| (t, r.report)),
        _ => None,
    })
}

/// 1 if other authors had scoring items both ways at settlement.
fn contested(trail: &Trail, settled: usize, lens: &str) -> i64 {
    let ways = Ways::of(
        live(trail, settled)
            .filter(|(_, i)| i.author != lens)
            .map(|(_, i)| i.direction),
    );
    i64::from(ways.support && ways.counter)
}

/// 1 if the score just before the job's first item was non-zero and
/// against `truth`. Score 0 has no tide.
fn upset(trail: &Trail, own: &Own, truth: Direction) -> i64 {
    let Some(first) = own.steps.first() else {
        return 0;
    };
    let before = first.checked_sub(1).map_or(0, |n| trail.steps[n].score);
    let against = match truth {
        Direction::Support => before < 0,
        Direction::Counter => before > 0,
    };
    i64::from(against)
}

/// Each lens's tally over `reviews`, reading each claim's trail from
/// `trails`; a review whose claim has no trail is pending.
pub fn tallies(reviews: &[Review], trails: &BTreeMap<MsgId, Trail>) -> BTreeMap<String, Tally> {
    let mut out: BTreeMap<String, Tally> = BTreeMap::new();
    for r in reviews {
        let scored = trails
            .get(&r.claim)
            .map_or(Scored::Pending, |t| score(r, t));
        out.entry(r.lens.clone()).or_default().add(scored);
    }
    out
}

/// One lens's record on one task.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tally {
    pub points: i64,
    /// Blind review jobs assigned and answered (the denominator).
    pub assigned: u32,
    pub scored: u32,
    pub pending: u32,
    /// Pivotal jobs, scored or not.
    pub pivotal: u32,
    /// Pivotal jobs scored against ground truth.
    pub pivotal_with_truth: u32,
}

impl Tally {
    pub fn add(&mut self, s: Scored) {
        match s {
            Scored::Excluded => return,
            Scored::Pending => self.pending += 1,
            Scored::Unscored { pivotal } => self.pivotal += u32::from(pivotal),
            Scored::Points { points, pivotal } => {
                self.points += points;
                self.scored += 1;
                self.pivotal += u32::from(pivotal);
                self.pivotal_with_truth += u32::from(pivotal);
            }
        }
        self.assigned += 1;
    }

    /// Whether the lens's calibration report is due.
    pub fn calibration_due(&self) -> bool {
        self.scored >= CALIBRATION_AFTER
    }

    /// The line put in the lens's context.
    pub fn line(&self) -> String {
        format!(
            "record: {} points over {} blind reviews ({} scored, {} pivotal)",
            self.points, self.assigned, self.scored, self.pivotal
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LENS: &str = "sec";

    fn ev(msg: u64, author: &str, direction: Direction, delta: i64) -> Change {
        Change::Item(Item {
            msg: MsgId(msg),
            author: author.into(),
            direction,
            grounded: true,
            cites: vec![],
            delta,
            scoring: true,
        })
    }

    /// A trail from changes, keeping a running score. A line settles by its
    /// sign, another status change settles `end`, and a reopen starts the
    /// next epoch, open, scoring the items it woke.
    fn trail(changes: Vec<Change>, end: ClaimStatus) -> Trail {
        let mut score = 0;
        let mut status = ClaimStatus::Open;
        let mut epoch = 0;
        let mut deltas = BTreeMap::new();
        let steps = changes
            .into_iter()
            .map(|change| {
                match &change {
                    Change::Item(i) => {
                        deltas.insert(i.msg, i.delta);
                        if i.scoring {
                            score += i.delta;
                        }
                    }
                    Change::Status(Rule::Reopen(woke)) => {
                        epoch += 1;
                        status = ClaimStatus::Open;
                        score = woke.iter().map(|m| deltas[m]).sum();
                    }
                    Change::Status(Rule::Line { at, .. }) if *at < 0 => {
                        status = ClaimStatus::Dropped;
                    }
                    Change::Status(Rule::Line { .. }) => status = ClaimStatus::Accepted,
                    Change::Status(_) => status = end,
                    _ => {}
                }
                Step {
                    change,
                    status,
                    score,
                    epoch,
                }
            })
            .collect();
        Trail { steps }
    }

    impl Change {
        /// An open step at score 0, epoch 0.
        fn into_step(self) -> Step {
            Step {
                change: self,
                status: ClaimStatus::Open,
                score: 0,
                epoch: 0,
            }
        }
    }

    /// Report `id`, with `profile` passed or failed.
    fn report(id: u64, profile: &str, passed: bool) -> Change {
        let (passed, failed) = if passed {
            (vec![profile.into()], vec![])
        } else {
            (vec![], vec![profile.into()])
        };
        Change::Report(Report {
            report: ArtId(id),
            passed,
            failed,
        })
    }

    /// `ev`, posted while the claim was closed: not scoring.
    fn closed_ev(msg: u64, author: &str, direction: Direction, delta: i64) -> Change {
        match ev(msg, author, direction, delta) {
            Change::Item(i) => Change::Item(Item {
                scoring: false,
                ..i
            }),
            other => other,
        }
    }

    fn line(at: i64, authors: u32) -> Change {
        Change::Status(Rule::Line {
            at,
            authors,
            min_authors: 2,
            holdout: None,
        })
    }

    /// A line that settled while report `holdout` spoke for the code.
    fn line_on(at: i64, authors: u32, holdout: u64) -> Change {
        Change::Status(Rule::Line {
            at,
            authors,
            min_authors: 2,
            holdout: Some(ArtId(holdout)),
        })
    }

    fn review(items: &[u64]) -> Review {
        Review {
            lens: LENS.into(),
            claim: MsgId(1),
            epoch: 0,
            look_harder: false,
            items: items.iter().copied().map(MsgId).collect(),
            unsure: false,
            pending: false,
        }
    }

    use Direction::{Counter, Support};

    #[test]
    fn a_non_pivotal_review_scores_on_the_outcome() {
        // Four supporters, 3 each: without the lens the line still holds.
        let t = trail(
            vec![
                ev(2, "a", Support, 3),
                ev(3, LENS, Support, 3),
                ev(4, "b", Support, 3),
                ev(5, "c", Support, 3),
                line(8, 4),
            ],
            ClaimStatus::Accepted,
        );
        assert_eq!(
            score(&review(&[3]), &t),
            Scored::Points {
                points: 1,
                pivotal: false
            }
        );
        let wrong = trail(
            vec![
                ev(2, "a", Counter, -3),
                ev(3, LENS, Support, 3),
                ev(4, "b", Counter, -3),
                ev(5, "c", Counter, -3),
                ev(6, "d", Counter, -3),
                line(-8, 4),
            ],
            ClaimStatus::Dropped,
        );
        // Others only countered and the tide (-3) went with the outcome:
        // w = 1, and the lens bet wrong.
        assert_eq!(
            score(&review(&[3]), &wrong),
            Scored::Points {
                points: -1,
                pivotal: false
            }
        );
    }

    #[test]
    fn a_pivotal_review_with_no_ground_truth_is_unscored_and_counted() {
        let t = trail(
            vec![ev(2, "a", Support, 4), ev(3, LENS, Support, 4), line(8, 2)],
            ClaimStatus::Accepted,
        );
        let s = score(&review(&[3]), &t);
        assert_eq!(s, Scored::Unscored { pivotal: true });
        let mut tally = Tally::default();
        tally.add(s);
        assert_eq!((tally.assigned, tally.scored, tally.pivotal), (1, 0, 1));
        assert_eq!(tally.points, 0);
    }

    #[test]
    fn a_pivotal_review_with_ground_truth_is_scored_against_it() {
        // The lens's counter drops the claim, and a golden set failed on it.
        let t = trail(
            vec![
                report(50, "golden:cases", false),
                ev(2, "a", Counter, -4),
                ev(3, LENS, Counter, -4),
                line_on(-8, 2, 50),
            ],
            ClaimStatus::Dropped,
        );
        let s = score(&review(&[3]), &t);
        // Upset: the score before the job was -4, with the truth: w = 1.
        assert_eq!(
            s,
            Scored::Points {
                points: 1,
                pivotal: true
            }
        );
        let mut tally = Tally::default();
        tally.add(s);
        assert_eq!((tally.pivotal, tally.pivotal_with_truth), (1, 1));
        // Not for a job citing that report; citing another is fine.
        let cite = |id: u64| {
            let mut t = t.clone();
            if let Change::Item(i) = &mut t.steps[2].change {
                i.cites = vec![ArtId(id)];
            }
            score(&review(&[3]), &t)
        };
        assert_eq!(cite(50), Scored::Unscored { pivotal: true });
        assert_eq!(cite(51), s);
        // A holdout failure is not ground truth.
        let mut holdout = t.clone();
        holdout.steps[0].change = report(50, "holdout:api", false);
        assert_eq!(
            score(&review(&[3]), &holdout),
            Scored::Unscored { pivotal: true }
        );
    }

    /// Truth comes only from the report the settlement names: BBP leaves
    /// out one from a retired suite, another tree or another candidate.
    #[test]
    fn a_golden_report_is_truth_only_if_the_settlement_names_it() {
        let with = |settle: Change| {
            let all = vec![
                report(50, "golden:cases", false),
                ev(2, "a", Counter, -4),
                ev(3, LENS, Counter, -4),
                settle,
            ];
            score(&review(&[3]), &trail(all, ClaimStatus::Dropped))
        };
        let unscored = Scored::Unscored { pivotal: true };
        assert_eq!(with(line(-8, 2)), unscored);
        assert_eq!(with(line_on(-8, 2, 51)), unscored);
    }

    /// A passed golden set says accept was right; the report the
    /// settlement names is the one that counts.
    #[test]
    fn a_passed_golden_set_is_truth_that_accept_was_right() {
        let with = |reports: Vec<Change>, named: u64| {
            let mut all = reports;
            all.extend([
                ev(2, "a", Support, 4),
                ev(3, LENS, Support, 4),
                line_on(8, 2, named),
            ]);
            score(&review(&[3]), &trail(all, ClaimStatus::Accepted))
        };
        let right = Scored::Points {
            points: 1,
            pivotal: true,
        };
        assert_eq!(with(vec![report(50, "golden:cases", true)], 50), right);
        // Failed, then passed on a later run: the later pass is named.
        let fail_then_pass = || {
            vec![
                report(50, "golden:cases", false),
                report(51, "golden:cases", true),
            ]
        };
        assert_eq!(with(fail_then_pass(), 51), right);
        // Passed, then failed: accept was wrong. Upset: the score before
        // the job was 4, against the drop: w = 3.
        assert_eq!(
            with(
                vec![
                    report(50, "golden:cases", true),
                    report(51, "golden:cases", false)
                ],
                51
            ),
            Scored::Points {
                points: -3,
                pivotal: true
            }
        );
        // The named report has no golden profile: no truth.
        assert_eq!(
            with(
                vec![
                    report(50, "golden:cases", true),
                    report(51, "holdout:api", false)
                ],
                51
            ),
            Scored::Unscored { pivotal: true }
        );
    }

    #[test]
    fn an_item_woken_by_its_own_reopen_counts_in_the_new_epoch() {
        // Dropped; the lens's executed support (posted closed) reopened it
        // and then carried the accept line with one other supporter.
        let t = trail(
            vec![
                ev(2, "x", Counter, -8),
                line(-8, 2),
                closed_ev(3, LENS, Support, 8),
                Change::Status(Rule::Reopen(vec![MsgId(3)])),
                ev(4, "a", Support, 8),
                line(8, 2),
            ],
            ClaimStatus::Accepted,
        );
        // Without the woken item the score is 0 and the authors 1: pivotal.
        assert_eq!(score(&review(&[3]), &t), Scored::Unscored { pivotal: true });
    }

    /// A reopen counts exactly the items it names: another author's woken
    /// item keeps the line without the lens, and an unwoken one doesn't.
    #[test]
    fn a_reopen_counts_exactly_the_items_it_woke() {
        let t = |woke: Vec<MsgId>, authors| {
            trail(
                vec![
                    ev(2, "x", Counter, -8),
                    line(-8, 2),
                    closed_ev(6, "c", Support, 4),
                    Change::Status(Rule::Reopen(woke)),
                    ev(3, LENS, Support, 4),
                    ev(4, "a", Support, 4),
                    line(8, authors),
                ],
                ClaimStatus::Accepted,
            )
        };
        assert!(matches!(
            score(&review(&[3]), &t(vec![MsgId(6)], 3)),
            Scored::Points { pivotal: false, .. }
        ));
        assert_eq!(
            score(&review(&[3]), &t(vec![], 2)),
            Scored::Unscored { pivotal: true }
        );
    }

    #[test]
    fn a_dep_reopen_leaves_a_closed_item_out() {
        let changes = |woke: Vec<MsgId>| {
            vec![
                ev(2, "x", Counter, -8),
                line(-8, 2),
                closed_ev(3, LENS, Support, 1),
                Change::Status(Rule::Reopen(woke)),
                ev(4, "a", Support, 5),
                ev(5, "b", Support, 5),
                line(8, 2),
            ]
        };
        // A dep reopen wakes nothing: the lens's closed item never counted.
        let dep = trail(changes(vec![]), ClaimStatus::Accepted);
        assert!(matches!(
            score(&review(&[3]), &dep),
            Scored::Points { pivotal: false, .. }
        ));
    }

    #[test]
    fn a_lens_that_flips_the_outcome_it_bet_on_earns_nothing_from_the_flip() {
        // The board leaned drop; the lens's big support pushed it over the
        // accept line. Without ground truth the flip is worth nothing, not
        // the upset bonus a non-pivotal win would get.
        let t = trail(
            vec![
                ev(2, "a", Counter, -2),
                ev(3, "b", Support, 3),
                ev(4, LENS, Support, 8),
                line(8, 2),
            ],
            ClaimStatus::Accepted,
        );
        let mut tally = Tally::default();
        tally.add(score(&review(&[4]), &t));
        assert_eq!((tally.points, tally.scored, tally.pivotal), (0, 0, 1));
    }

    #[test]
    fn losing_an_author_undoes_the_line_only_below_the_minimum() {
        let changes = |min| {
            vec![
                ev(2, "a", Support, 5),
                ev(3, LENS, Support, 1),
                ev(4, "b", Support, 5),
                Change::Status(Rule::Line {
                    at: 8,
                    authors: 3,
                    min_authors: min,
                    holdout: None,
                }),
            ]
        };
        let enough = trail(changes(2), ClaimStatus::Accepted);
        assert!(matches!(
            score(&review(&[3]), &enough),
            Scored::Points { pivotal: false, .. }
        ));
        let tight = trail(changes(3), ClaimStatus::Accepted);
        assert_eq!(
            score(&review(&[3]), &tight),
            Scored::Unscored { pivotal: true }
        );
    }

    #[test]
    fn a_human_settlement_is_never_pivotal_and_a_reversal_is_the_outcome() {
        // The line accepted with the lens's help, then Nano dropped it.
        let t = trail(
            vec![
                ev(2, "a", Support, 4),
                ev(3, LENS, Counter, -1),
                ev(4, "b", Support, 5),
                line(8, 2),
                Change::Status(Rule::Human),
            ],
            ClaimStatus::Dropped,
        );
        // Not contested (others only supported). Upset: the score was 4
        // before the job, against the drop: w = 3.
        assert_eq!(
            score(&review(&[3]), &t),
            Scored::Points {
                points: 3,
                pivotal: false
            }
        );
    }

    #[test]
    fn a_status_event_that_changes_nothing_is_not_the_settlement() {
        let mut t = trail(
            vec![ev(2, "a", Support, 4), ev(3, LENS, Support, 4), line(8, 2)],
            ClaimStatus::Accepted,
        );
        let last = t.steps[2].clone();
        // An accepted claim's action mark times out: escalated, still accepted.
        t.steps.push(Step {
            change: Change::Status(Rule::Other),
            ..last
        });
        assert_eq!(score(&review(&[3]), &t), Scored::Unscored { pivotal: true });
    }

    /// A non-pivotal win, but one of the job's posts was lost and may have
    /// landed: its direction, citation and weight aren't known, so it is
    /// never scored, and on a line it counts as pivotal.
    #[test]
    fn a_job_with_a_lost_post_is_never_scored_against_the_outcome() {
        let t = trail(
            vec![
                ev(2, "a", Support, 3),
                ev(3, LENS, Support, 3),
                ev(4, "b", Support, 3),
                ev(5, "c", Support, 3),
                line(8, 4),
            ],
            ClaimStatus::Accepted,
        );
        let unsure = Review {
            unsure: true,
            ..review(&[3])
        };
        assert_eq!(score(&unsure, &t), Scored::Unscored { pivotal: true });
        let nothing_known = Review {
            unsure: true,
            ..review(&[])
        };
        assert_eq!(
            score(&nothing_known, &t),
            Scored::Unscored { pivotal: true }
        );
        // Not on a human settlement either, nor against golden truth: the
        // lost item may have gone the other way or cited the report.
        let mut human = t.clone();
        human.steps[4].change = Change::Status(Rule::Human);
        assert_eq!(score(&unsure, &human), Scored::Unscored { pivotal: false });
        let mut golden = t.clone();
        golden.steps[4].change = line_on(8, 4, 50);
        golden
            .steps
            .insert(0, report(50, "golden:cases", true).into_step());
        assert_eq!(score(&unsure, &golden), Scored::Unscored { pivotal: true });
    }

    /// A row still pending: the job crashed or failed its save after it may
    /// have posted. Its items aren't known, so it scores like an unsure job.
    #[test]
    fn a_job_still_pending_is_never_scored_against_the_outcome() {
        let t = trail(
            vec![
                ev(2, "a", Support, 3),
                ev(3, LENS, Support, 3),
                ev(4, "b", Support, 3),
                ev(5, "c", Support, 3),
                line(8, 4),
            ],
            ClaimStatus::Accepted,
        );
        assert!(matches!(
            score(&review(&[]), &t),
            Scored::Unscored { pivotal: false }
        ));
        let pending = Review {
            pending: true,
            ..review(&[])
        };
        assert_eq!(score(&pending, &t), Scored::Unscored { pivotal: true });
    }

    #[test]
    fn contested_claims_weigh_more() {
        let t = trail(
            vec![
                ev(2, LENS, Support, 3),
                ev(3, "a", Support, 4),
                ev(4, "b", Counter, -1),
                ev(5, "c", Support, 5),
                line(8, 3),
            ],
            ClaimStatus::Accepted,
        );
        assert_eq!(
            score(&review(&[2]), &t),
            Scored::Points {
                points: 2,
                pivotal: false
            }
        );
    }

    #[test]
    fn empty_mixed_open_and_void_reviews() {
        let accepted = trail(
            vec![
                ev(2, "a", Support, 5),
                ev(3, LENS, Support, 1),
                ev(4, LENS, Counter, -1),
                ev(5, "b", Support, 5),
                line(8, 2),
            ],
            ClaimStatus::Accepted,
        );
        let mut tally = Tally::default();
        tally.add(score(&review(&[]), &accepted));
        tally.add(score(&review(&[3, 4]), &accepted));
        let open = trail(vec![ev(3, LENS, Support, 1)], ClaimStatus::Open);
        tally.add(score(&review(&[3]), &open));
        let void = trail(
            vec![ev(3, LENS, Support, 1), Change::Status(Rule::Other)],
            ClaimStatus::Void,
        );
        tally.add(score(&review(&[3]), &void));
        assert_eq!(
            tally,
            Tally {
                points: 0,
                assigned: 3,
                scored: 0,
                pending: 1,
                pivotal: 0,
                pivotal_with_truth: 0,
            }
        );
        assert_eq!(
            tally.line(),
            "record: 0 points over 3 blind reviews (0 scored, 0 pivotal)"
        );
        assert!(!tally.calibration_due());
    }
}
