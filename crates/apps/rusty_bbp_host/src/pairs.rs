//! `bbp pairs`: the 2b gate criteria evaluated over a run's pairs. The
//! evaluation is pure ([`evaluate`] over [`Pair`] facts); [`load`] reads the
//! facts from each run's log, harness output and saved `bbp mod` output. It
//! decides nothing the criteria note does not say; see "2b gate criteria" in
//! `docs/bbp-concurrency-design.md` for the definitions and the thresholds.

use crate::{report, tokens};
use rusty_bbp::*;
use rusty_serde::{Deserialize, Serialize};
use std::path::Path;

/// Thresholds of the criteria note.
pub const NOISE_BAND: (f64, f64) = (0.85, 1.15);
pub const WALL_RATIO_MAX: f64 = 0.75;
pub const MAX_INVERTED_SEPARABLE: usize = 1;
pub const POLL_ERROR_MAX: f64 = 0.05;
pub const COST_RATIO_MAX: f64 = 1.6;
pub const PAIRS: usize = 10;
pub const SEPARABLE: usize = 5;
pub const SHARED: usize = 5;
pub const NULL_PAIRS: usize = 3;
pub const MIN_MEASURABLE_SEPARABLE: usize = 4;
pub const UNUSED_SHARE_MIN: f64 = 0.15;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    /// Noise gate: the same task twice at one live turn. `control` is the
    /// first run, `treatment` the repeat.
    #[rusty_serde(rename = "null")]
    Null,
    #[rusty_serde(rename = "separable")]
    Separable,
    #[rusty_serde(rename = "shared")]
    Shared,
}

/// What one run measured. Built by [`load`]; the tests build it by hand.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunFacts {
    /// Reached `closed`.
    pub closed: bool,
    /// The grader's verdict against the frozen rubric and pass mark.
    pub rubric_ok: bool,
    /// The first `planning` visit, from `bbp mod`'s `state_ms`.
    pub spec_wall_ms: Option<u64>,
    /// Cost-weighted tokens, all turns (revoked included); `None` when any
    /// turn left no output.
    pub tokens: Option<f64>,
    /// `spent.messages` or `spent.bytes` above its total.
    pub overshoot: bool,
    pub unused_conflicts: u64,
    /// `unused_share` as (unused bytes, all spec bytes).
    pub unused_bytes: (u64, u64),
}

impl RunFacts {
    fn succeeded(&self) -> bool {
        self.closed && self.rubric_ok
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pair {
    pub id: String,
    pub kind: Kind,
    pub control: RunFacts,
    pub treatment: RunFacts,
    /// Rubric items an unused spec version met and the approved one did not,
    /// from the grader; `None` until graded.
    pub lost_rubric_items: Option<u64>,
}

impl Pair {
    fn ok(&self) -> bool {
        self.control.succeeded() && self.treatment.succeeded()
    }

    fn wall_ratio(&self) -> Option<f64> {
        let (c, t) = (self.control.spec_wall_ms?, self.treatment.spec_wall_ms?);
        (c > 0).then(|| t as f64 / c as f64)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Verdict {
    Holds,
    Fails,
    Inconclusive,
}

fn verdict(holds: bool) -> Verdict {
    if holds {
        Verdict::Holds
    } else {
        Verdict::Fails
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Evaluation {
    /// Phase 1: every null ratio inside [`NOISE_BAND`]. `Inconclusive` until
    /// all [`NULL_PAIRS`] are in. A `Fails` stops the run.
    pub noise_gate: Verdict,
    pub null_ratios: Vec<f64>,
    /// 1: no N=2 run overshot.
    pub c1_safety: Verdict,
    /// 2: at least 8 of 10 pairs succeed and N=2 fails at most one more than N=1.
    pub c2_comparable: Verdict,
    /// 3: median `spec_wall` ratio on separable pairs, with the variance and
    /// poll-error conditions.
    pub c3_parallelism: Verdict,
    pub separable_ratios: Vec<f64>,
    /// 4: an unused spec version conflicts, in at least 3 of 5 shared pairs. An
    /// upper bound: each flagged version is read by hand before it counts.
    pub c4_contention: Verdict,
    /// 5: lost rubric items in at least 3 shared pairs, median `unused_share`
    /// at least [`UNUSED_SHARE_MIN`].
    pub c5_loss: Verdict,
    pub median_unused_share: Option<f64>,
    /// 7: median tokens per accepted outcome, N=2 over N=1.
    pub c7_cost: Verdict,
    pub cost_ratios: Vec<f64>,
    /// The decision table row that applies (first match).
    pub decision: &'static str,
    /// The sha256 of the criteria file the tokens were weighted by; set by `load`.
    pub criteria_sha256: Option<String>,
}

fn median(mut v: Vec<f64>) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let m = v.len() / 2;
    Some(if v.len() % 2 == 1 {
        v[m]
    } else {
        (v[m - 1] + v[m]) / 2.0
    })
}

fn noise_gate(pairs: &[Pair]) -> (Verdict, Vec<f64>) {
    let ratios: Vec<f64> = pairs
        .iter()
        .filter(|p| p.kind == Kind::Null)
        .filter_map(Pair::wall_ratio)
        .collect();
    let outside = ratios
        .iter()
        .any(|r| *r < NOISE_BAND.0 || *r > NOISE_BAND.1);
    let v = match (outside, ratios.len() >= NULL_PAIRS) {
        (true, _) => Verdict::Fails,
        (false, true) => Verdict::Holds,
        (false, false) => Verdict::Inconclusive,
    };
    (v, ratios)
}

fn c2(pairs: &[&Pair]) -> Verdict {
    let ok = pairs.iter().filter(|p| p.ok()).count();
    let fail = |f: fn(&Pair) -> &RunFacts| pairs.iter().filter(|p| !f(p).succeeded()).count();
    let (f1, f2) = (fail(|p| &p.control), fail(|p| &p.treatment));
    if ok >= PAIRS - 2 && f2 <= f1 + 1 {
        return if pairs.len() >= PAIRS {
            Verdict::Holds
        } else {
            Verdict::Inconclusive
        };
    }
    // Already failed: more pairs cannot rescue it once enough are in.
    if pairs.len() >= PAIRS || pairs.len() - ok > 2 {
        Verdict::Fails
    } else {
        Verdict::Inconclusive
    }
}

/// `poll` must be at most [`POLL_ERROR_MAX`] of the control's `spec_wall`.
fn measurable(p: &Pair, poll_ms: u64) -> Option<f64> {
    let c = p.control.spec_wall_ms?;
    if !p.ok() || poll_ms as f64 > POLL_ERROR_MAX * c as f64 {
        return None;
    }
    p.wall_ratio()
}

fn c3(separable: &[&Pair], gate: Verdict, poll_ms: u64) -> (Verdict, Vec<f64>) {
    let ratios: Vec<f64> = separable
        .iter()
        .filter_map(|p| measurable(p, poll_ms))
        .collect();
    if gate != Verdict::Holds || ratios.len() < MIN_MEASURABLE_SEPARABLE {
        return (Verdict::Inconclusive, ratios);
    }
    let inverted = ratios.iter().filter(|r| **r > 1.0).count();
    let holds = median(ratios.clone()).is_some_and(|m| m <= WALL_RATIO_MAX)
        && inverted <= MAX_INVERTED_SEPARABLE;
    (verdict(holds), ratios)
}

fn share(r: &RunFacts) -> f64 {
    match r.unused_bytes {
        (_, 0) => 0.0,
        (u, a) => u as f64 / a as f64,
    }
}

fn c4(shared: &[&Pair]) -> Verdict {
    let ok: Vec<&&Pair> = shared.iter().filter(|p| p.ok()).collect();
    let hit = ok
        .iter()
        .filter(|p| p.treatment.unused_conflicts > 0)
        .count();
    if hit >= 3 {
        Verdict::Holds
    } else if shared.len() < SHARED {
        Verdict::Inconclusive
    } else {
        Verdict::Fails
    }
}

fn c5(shared: &[&Pair]) -> (Verdict, Option<f64>) {
    let ok: Vec<&&Pair> = shared.iter().filter(|p| p.ok()).collect();
    let med = median(ok.iter().map(|p| share(&p.treatment)).collect());
    if ok.iter().any(|p| p.lost_rubric_items.is_none()) || shared.len() < SHARED {
        return (Verdict::Inconclusive, med);
    }
    let lost = ok
        .iter()
        .filter(|p| p.lost_rubric_items.is_some_and(|n| n >= 1))
        .count();
    let holds = lost >= 3 && med.is_some_and(|m| m >= UNUSED_SHARE_MIN);
    (verdict(holds), med)
}

fn c7(pairs: &[&Pair]) -> (Verdict, Vec<f64>) {
    let ok: Vec<&&Pair> = pairs.iter().filter(|p| p.ok()).collect();
    let ratios: Vec<f64> = ok
        .iter()
        .filter_map(|p| match (p.control.tokens, p.treatment.tokens) {
            (Some(c), Some(t)) if c > 0.0 => Some(t / c),
            _ => None,
        })
        .collect();
    // A pair with unknown tokens makes the median unknown, not smaller.
    if ratios.len() < ok.len() || ratios.is_empty() {
        return (Verdict::Inconclusive, ratios);
    }
    let holds = median(ratios.clone()).is_some_and(|m| m <= COST_RATIO_MAX);
    (verdict(holds), ratios)
}

/// The decision table of the criteria note, first match.
fn decide(e: &Evaluation) -> &'static str {
    use self::Verdict::*;
    if e.noise_gate == Fails {
        return "noise gate failed: stop; no phase 2 run; the owner decides";
    }
    if e.c1_safety == Fails {
        return "1 fails: fix 2a; no decision";
    }
    if e.c2_comparable == Fails {
        return "2 fails: inconclusive; no decision; rerun with a corrected rubric or task set";
    }
    if e.c3_parallelism == Inconclusive {
        return "3 inconclusive: no decision (noise, measurement error or too few measurable pairs)";
    }
    if e.c3_parallelism == Fails {
        return "3 fails: stop concurrency work; max_live_turns = 1 stays the default";
    }
    if e.c7_cost == Fails {
        return "3 holds, 7 fails: parallelism pays in time but not in cost; no CRDT";
    }
    if e.c4_contention == Fails {
        return "3 and 7 hold, 4 fails: 2a alone is enough for spec authoring; no CRDT";
    }
    if e.c5_loss == Fails {
        return "3, 7 and 4 hold, 5 fails: section leases or 2a; no CRDT";
    }
    let all = [e.c2_comparable, e.c4_contention, e.c5_loss, e.c7_cost];
    if all.iter().all(|v| *v == Holds) && e.c1_safety == Holds {
        return "1 to 5 and 7 hold: build rusty_crdt and 2b, after criterion 6's precondition (verdict binding landed); criterion 4 still needs the flagged versions read by hand";
    }
    "incomplete: some criteria are not yet measurable; no decision"
}

/// Evaluate the criteria over `pairs` with the run's poll interval.
pub fn evaluate(pairs: &[Pair], poll_ms: u64) -> Evaluation {
    let (gate, null_ratios) = noise_gate(pairs);
    let phase2: Vec<&Pair> = pairs.iter().filter(|p| p.kind != Kind::Null).collect();
    let of = |k| -> Vec<&Pair> { phase2.iter().copied().filter(|p| p.kind == k).collect() };
    let (separable, shared) = (of(Kind::Separable), of(Kind::Shared));
    let c1 = if phase2.iter().any(|p| p.treatment.overshoot) {
        Verdict::Fails
    } else if phase2.len() >= PAIRS {
        Verdict::Holds
    } else {
        Verdict::Inconclusive
    };
    let (c3v, separable_ratios) = c3(&separable, gate, poll_ms);
    let (c5v, median_unused_share) = c5(&shared);
    let (c7v, cost_ratios) = c7(&phase2);
    let mut e = Evaluation {
        noise_gate: gate,
        null_ratios,
        c1_safety: c1,
        c2_comparable: c2(&phase2),
        c3_parallelism: c3v,
        separable_ratios,
        c4_contention: c4(&shared),
        c5_loss: c5v,
        median_unused_share,
        c7_cost: c7v,
        cost_ratios,
        decision: "",
        criteria_sha256: None,
    };
    e.decision = decide(&e);
    e
}

// ---------------------------------------------------------------- loading

/// One run in the manifest.
#[derive(Clone, Debug, Deserialize)]
pub struct RunRef {
    pub dir: String,
    pub task: String,
    /// The saved stdout of `bbp mod` for this run.
    pub mod_output: String,
    /// The grader's verdict for this run's approved spec (`bbp grade join`).
    /// Absent before grading, which reads as not met.
    #[rusty_serde(default)]
    pub rubric_ok: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PairRef {
    pub id: String,
    pub kind: Kind,
    pub control: RunRef,
    pub treatment: RunRef,
    #[rusty_serde(default)]
    pub lost_rubric_items: Option<u64>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Manifest {
    /// The `--poll-ms` the runs used.
    pub poll_ms: u64,
    /// The committed criteria file whose weights price the tokens.
    pub criteria: String,
    pub pairs: Vec<PairRef>,
}

#[derive(Deserialize)]
struct ModOut {
    state_ms: Vec<(String, u64)>,
}

/// The first `Planning` visit of a `bbp mod` output, in ms.
pub fn spec_wall_ms(mod_output: &str) -> Result<Option<u64>, String> {
    let m: ModOut = rusty_serde::json::from_str(mod_output).map_err(|e| e.to_string())?;
    Ok(m.state_ms
        .into_iter()
        .find(|(s, _)| s == "Planning")
        .map(|(_, ms)| ms))
}

fn facts(r: &RunRef, w: &tokens::Weights) -> Result<RunFacts, String> {
    let task = TaskId(r.task.clone());
    let rep = report::of(Path::new(&r.dir), &task, None)?;
    let d = crate::open_driver(Path::new(&r.dir), &task)?;
    let events = d.store.events(&task, 0).map_err(|e| format!("{e:?}"))?;
    let tok = tokens::of(Path::new(&r.dir), &task, &events);
    let text =
        std::fs::read_to_string(&r.mod_output).map_err(|e| format!("{}: {e}", r.mod_output))?;
    let st = &d.state;
    Ok(RunFacts {
        closed: st.state == State::Closed,
        rubric_ok: r.rubric_ok,
        spec_wall_ms: spec_wall_ms(&text)?,
        tokens: (tok.without_output == 0)
            .then(|| tok.turns.iter().map(|t| t.usage.weighted(w)).sum()),
        overshoot: st.spent.messages > st.budget.messages || st.spent.bytes > st.budget.bytes,
        unused_conflicts: rep.unused_conflicts,
        unused_bytes: rep.unused_bytes,
    })
}

/// Read `manifest` and evaluate every pair in it.
pub fn load(manifest: &Path) -> Result<Evaluation, String> {
    let text =
        std::fs::read_to_string(manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let m: Manifest =
        rusty_serde::json::from_str(&text).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let (c, sha) = tokens::read_criteria(Path::new(&m.criteria))?;
    let pairs = m
        .pairs
        .iter()
        .map(|p| {
            Ok(Pair {
                id: p.id.clone(),
                kind: p.kind,
                control: facts(&p.control, &c.weights)?,
                treatment: facts(&p.treatment, &c.weights)?,
                lost_rubric_items: p.lost_rubric_items,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut e = evaluate(&pairs, m.poll_ms);
    e.criteria_sha256 = Some(sha);
    Ok(e)
}

/// [`load`] as one line of JSON.
pub fn json(manifest: &Path) -> Result<String, String> {
    rusty_serde::json::to_string(&load(manifest)?).map_err(|e| e.to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// A run that closed and passed its rubric, with these walls and tokens.
    fn run(wall: u64, tokens: u64) -> RunFacts {
        RunFacts {
            closed: true,
            rubric_ok: true,
            spec_wall_ms: Some(wall),
            tokens: Some(tokens as f64),
            ..RunFacts::default()
        }
    }

    fn pair(id: &str, kind: Kind, control: RunFacts, treatment: RunFacts) -> Pair {
        Pair {
            id: id.into(),
            kind,
            control,
            treatment,
            lost_rubric_items: None,
        }
    }

    fn nulls(ratios: [f64; 3]) -> Vec<Pair> {
        ratios
            .iter()
            .enumerate()
            .map(|(i, r)| {
                pair(
                    &format!("n{i}"),
                    Kind::Null,
                    run(100_000, 10),
                    run((100_000.0 * r) as u64, 10),
                )
            })
            .collect()
    }

    /// 5 separable pairs at these ratios, equal tokens.
    fn separable(ratios: [f64; 5]) -> Vec<Pair> {
        ratios
            .iter()
            .enumerate()
            .map(|(i, r)| {
                pair(
                    &format!("s{i}"),
                    Kind::Separable,
                    run(100_000, 100),
                    run((100_000.0 * r) as u64, 100),
                )
            })
            .collect()
    }

    fn shared(unused_conflicts: u64, lost: u64) -> Vec<Pair> {
        (0..5)
            .map(|i| {
                let mut t = run(100_000, 100);
                t.unused_conflicts = unused_conflicts;
                t.unused_bytes = (30, 100);
                let mut p = pair(&format!("h{i}"), Kind::Shared, run(100_000, 100), t);
                p.lost_rubric_items = Some(lost);
                p
            })
            .collect()
    }

    fn all(sep: [f64; 5], conflicts: u64, lost: u64) -> Vec<Pair> {
        [
            nulls([1.0, 0.95, 1.1]),
            separable(sep),
            shared(conflicts, lost),
        ]
        .concat()
    }

    #[test]
    fn a_null_ratio_outside_the_band_fails_the_gate_and_stops_the_run() {
        let e = evaluate(&nulls([1.0, 1.16, 1.0]), 500);
        assert_eq!(e.noise_gate, Verdict::Fails);
        assert!(e.decision.starts_with("noise gate failed"));
        // The band is inclusive at both ends.
        assert_eq!(
            evaluate(&nulls([0.85, 1.15, 1.0]), 500).noise_gate,
            Verdict::Holds
        );
        assert_eq!(
            evaluate(&nulls([0.84, 1.0, 1.0]), 500).noise_gate,
            Verdict::Fails
        );
    }

    #[test]
    fn two_null_pairs_are_not_a_passed_gate() {
        let two = nulls([1.0, 1.0, 1.0])[..2].to_vec();
        assert_eq!(evaluate(&two, 500).noise_gate, Verdict::Inconclusive);
    }

    #[test]
    fn criterion_three_needs_the_median_and_at_most_one_inverted_pair() {
        let holds = evaluate(&all([0.5, 0.6, 0.7, 0.75, 1.0], 1, 1), 500);
        assert_eq!(holds.c3_parallelism, Verdict::Holds, "{holds:?}");
        // Median 0.7 but two pairs slower than the control.
        let two_inverted = evaluate(&all([0.5, 0.6, 0.7, 1.2, 1.3], 1, 1), 500);
        assert_eq!(two_inverted.c3_parallelism, Verdict::Fails);
        // Median above 0.75.
        let slow = evaluate(&all([0.8, 0.8, 0.8, 0.8, 0.8], 1, 1), 500);
        assert_eq!(slow.c3_parallelism, Verdict::Fails);
    }

    #[test]
    fn a_noisy_control_makes_criterion_three_inconclusive_not_failed() {
        let mut p = all([0.5; 5], 1, 1);
        p[1].treatment.spec_wall_ms = Some(130_000); // null ratio 1.3
        let e = evaluate(&p, 500);
        assert_eq!(e.noise_gate, Verdict::Fails);
        assert_eq!(e.c3_parallelism, Verdict::Inconclusive);
    }

    #[test]
    fn poll_error_above_five_percent_of_the_control_excludes_a_pair() {
        // Poll 500 ms against a 9 s control: 5.6%, unmeasurable. One pair out
        // leaves 4, still enough; two out leaves 3, inconclusive.
        let mut p = all([0.5; 5], 1, 1);
        let sep: Vec<usize> = p
            .iter()
            .enumerate()
            .filter(|(_, x)| x.kind == Kind::Separable)
            .map(|(i, _)| i)
            .collect();
        p[sep[0]].control.spec_wall_ms = Some(9_000);
        assert_eq!(evaluate(&p, 500).separable_ratios.len(), 4);
        assert_eq!(evaluate(&p, 500).c3_parallelism, Verdict::Holds);
        p[sep[1]].control.spec_wall_ms = Some(9_000);
        assert_eq!(evaluate(&p, 500).separable_ratios.len(), 3);
        assert_eq!(evaluate(&p, 500).c3_parallelism, Verdict::Inconclusive);
    }

    #[test]
    fn a_failed_pair_is_never_counted_as_fast() {
        let mut p = all([0.5; 5], 1, 1);
        let i = p.iter().position(|x| x.kind == Kind::Separable).unwrap();
        p[i].treatment.rubric_ok = false;
        let e = evaluate(&p, 500);
        assert_eq!(e.separable_ratios.len(), 4, "the failed pair is out");
    }

    #[test]
    fn three_holding_and_seven_failing_is_its_own_row_whatever_four_and_five_show() {
        let mut p = all([0.5; 5], 0, 0); // 4 and 5 would fail
        for x in p.iter_mut().filter(|x| x.kind != Kind::Null) {
            x.treatment.tokens = Some(200.0); // 2.0 times the control
        }
        let e = evaluate(&p, 500);
        assert_eq!(e.c3_parallelism, Verdict::Holds);
        assert_eq!(e.c7_cost, Verdict::Fails);
        assert!(e.decision.starts_with("3 holds, 7 fails"), "{}", e.decision);
    }

    #[test]
    fn cost_at_the_bound_holds_and_unknown_tokens_are_not_a_pass() {
        let mut p = all([0.5; 5], 1, 1);
        for x in p.iter_mut().filter(|x| x.kind != Kind::Null) {
            x.treatment.tokens = Some(160.0); // exactly 1.6
        }
        assert_eq!(evaluate(&p, 500).c7_cost, Verdict::Holds);
        let i = p.iter().position(|x| x.kind == Kind::Shared).unwrap();
        p[i].treatment.tokens = None;
        assert_eq!(evaluate(&p, 500).c7_cost, Verdict::Inconclusive);
    }

    #[test]
    fn everything_holding_points_at_the_build_with_its_preconditions() {
        let e = evaluate(&all([0.5; 5], 1, 1), 500);
        assert_eq!(
            [
                e.c1_safety,
                e.c2_comparable,
                e.c3_parallelism,
                e.c4_contention,
                e.c5_loss,
                e.c7_cost
            ],
            [Verdict::Holds; 6],
            "{e:?}"
        );
        assert!(e.decision.starts_with("1 to 5 and 7 hold"));
        assert!(e.decision.contains("by hand"));
    }

    #[test]
    fn contention_without_loss_stops_short_of_the_crdt() {
        let e = evaluate(&all([0.5; 5], 1, 0), 500);
        assert_eq!(e.c5_loss, Verdict::Fails);
        assert!(e.decision.starts_with("3, 7 and 4 hold, 5 fails"));
        let e = evaluate(&all([0.5; 5], 0, 0), 500);
        assert!(e.decision.starts_with("3 and 7 hold, 4 fails"));
    }

    #[test]
    fn an_overshooting_n2_run_fails_safety_first() {
        let mut p = all([0.5; 5], 1, 1);
        let i = p.iter().position(|x| x.kind == Kind::Shared).unwrap();
        p[i].treatment.overshoot = true;
        let e = evaluate(&p, 500);
        assert_eq!(e.c1_safety, Verdict::Fails);
        assert!(e.decision.starts_with("1 fails"));
    }

    #[test]
    fn eight_of_ten_pairs_succeeding_is_enough_and_seven_is_not() {
        let mut p = all([0.5; 5], 1, 1);
        let idx: Vec<usize> = p
            .iter()
            .enumerate()
            .filter(|(_, x)| x.kind != Kind::Null)
            .map(|(i, _)| i)
            .collect();
        p[idx[0]].treatment.closed = false;
        p[idx[1]].control.closed = false;
        assert_eq!(evaluate(&p, 500).c2_comparable, Verdict::Holds);
        // A third failed pair leaves 7 of 10.
        p[idx[2]].treatment.closed = false;
        assert_eq!(evaluate(&p, 500).c2_comparable, Verdict::Fails);
    }

    #[test]
    fn n2_failing_two_more_than_n1_fails_even_with_eight_pairs_ok() {
        let mut p = all([0.5; 5], 1, 1);
        let idx: Vec<usize> = p
            .iter()
            .enumerate()
            .filter(|(_, x)| x.kind != Kind::Null)
            .map(|(i, _)| i)
            .collect();
        p[idx[0]].treatment.closed = false;
        p[idx[1]].treatment.closed = false;
        assert_eq!(evaluate(&p, 500).c2_comparable, Verdict::Fails);
    }

    #[test]
    fn the_median_of_an_even_count_is_the_mean_of_the_middle_two() {
        assert_eq!(median(vec![1.0, 3.0, 2.0, 10.0]), Some(2.5));
        assert_eq!(median(vec![]), None);
    }

    #[test]
    fn spec_wall_is_the_first_planning_visit() {
        let out = r#"{"state":"Closed","turns_launched":3,"runs":1,"elapsed_ms":9,"state_ms":[["Planning",4100],["PlanGate",10],["Planning",50]]}"#;
        assert_eq!(spec_wall_ms(out), Ok(Some(4100)));
        assert_eq!(spec_wall_ms(r#"{"state_ms":[["Build",1]]}"#), Ok(None));
        assert!(spec_wall_ms("not json").is_err());
    }
}
