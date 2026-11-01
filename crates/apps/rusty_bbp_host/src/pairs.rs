//! `bbp pairs`: the 2b gate criteria evaluated over a run's pairs. The
//! evaluation is pure ([`evaluate`] over [`Pair`] facts); [`load`] reads the
//! facts from each run's log, harness output and saved `bbp mod` output, and
//! from its evidence record: a run that more than one `bbp mod` produced is a
//! failed, stopped run (`resumed`), and a record that is missing, unreadable
//! or names another build is refused ([`check_segments`]). It decides
//! nothing the criteria note does not say;
//! see "2b gate criteria" in `docs/bbp-concurrency-design.md` for the
//! definitions and the thresholds.

use crate::{report, tokens};
use rusty_bbp::*;
use rusty_serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
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
    /// Accounting failures that prevent a stopped run from being complete.
    pub accounting_errors: Vec<String>,
    /// Reached `closed`.
    pub closed: bool,
    /// The grader's verdict against the frozen rubric and pass mark.
    pub rubric_ok: bool,
    /// The first `planning` visit, from `bbp mod`'s `state_ms`; `None` for a
    /// resumed run, whose last output timed only its last segment.
    pub spec_wall_ms: Option<u64>,
    /// Dollars, all turns (revoked included), priced per model by the
    /// criteria file; `None` when any turn left no output.
    pub tokens: Option<f64>,
    /// `spent.messages` or `spent.bytes` above its total.
    pub overshoot: bool,
    pub unused_conflicts: u64,
    /// `unused_share` as (unused bytes, all spec bytes).
    pub unused_bytes: (u64, u64),
    /// The moderator's live state equalled the replay of the log at the end
    /// of the run (`bbp mod` prints it); `None` when the run did not say, or
    /// was resumed (the last output speaks for its last segment only).
    pub replay_equals_live: Option<bool>,
    /// State boundaries up to and including the end of the first `Planning`
    /// visit, each observed to within one poll.
    pub spec_boundaries: u64,
    /// The run's harnesses cost more than the per-run cap.
    pub over_cap: bool,
    /// The `reads_per_turn` the task was opened with (its `TaskOpened`
    /// event). A run whose limit was later extended does not load.
    pub reads_per_turn: u64,
    /// The models each role's turns used, from the harness output.
    pub role_models: BTreeMap<String, BTreeSet<String>>,
    /// The build commit the run's own `bbp mod` reported.
    pub build_sha: String,
    /// Granted turns with no model observed: no harness output, or output with
    /// no assistant message (a `result` line alone names no model).
    pub no_model_turns: u64,
    /// Turns that ended with no valid result (killed, interrupted, crashed):
    /// their cost is only known from below.
    pub killed_turns: u64,
    /// Why the run stopped: `cap` or `wall` from its `bbp mod` output, or
    /// `resumed: N bbp mod segments` (with `; last: cap|wall` when the last
    /// output stopped) when more than one `bbp mod` produced it. Reported with a
    /// failed pair.
    pub stop: Option<String>,
    /// The moderator stopped the run (cap or wall limit), or it was resumed
    /// (a later segment cannot show an earlier one's stop): it may have printed
    /// little or nothing a model can be read from.
    pub stopped: bool,
    /// The dollars the run is known to have cost, at least: complete turns at
    /// their result, the rest at what their printed messages show.
    pub usd_floor: Option<f64>,
}

/// What a failed pair is known to have cost, at least.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FailedCost {
    pub pair: String,
    pub usd_at_least: f64,
    /// Some turn costs are unknown; the reported dollars are only a lower bound.
    pub incomplete: bool,
    /// Failures encountered while retaining readable turns.
    pub accounting_errors: Vec<String>,
    /// Why each stopped run of the pair stopped, as `arm: reason`.
    pub stops: Vec<String>,
}

impl RunFacts {
    pub fn succeeded(&self) -> bool {
        self.closed && self.rubric_ok && !self.over_cap && !self.stopped && self.killed_turns == 0
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
    /// The sha256 of the criteria file the tokens were priced by; set by `load`.
    pub criteria_sha256: Option<String>,
    /// The commit the run was built from; set by `load`, which refuses a dirty
    /// tree or a commit that does not contain `min_commit`.
    pub git_sha: Option<String>,
    /// The commit this `bbp` binary was built from; `load` refuses unless it
    /// is `git_sha`.
    pub bbp_build_sha: Option<String>,
    /// Failed pairs and what each cost, at least, reported apart from 7.
    pub failed_pair_costs: Vec<FailedCost>,
    /// What every run together is known to have cost, at least.
    pub total_usd_at_least: f64,
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

/// Safety: no N=2 run overshot a budget, and the moderator's live state equalled
/// the replay of the log in every run, the null pairs' repeats included. Known
/// only with all ten pairs in.
fn c1(phase2: &[&Pair], all: &[Pair]) -> Verdict {
    let runs = || all.iter().flat_map(|p| [&p.control, &p.treatment]);
    if phase2.iter().any(|p| p.treatment.overshoot)
        || runs().any(|r| r.replay_equals_live == Some(false))
    {
        Verdict::Fails
    } else if phase2.len() >= PAIRS && runs().all(|r| r.replay_equals_live == Some(true)) {
        Verdict::Holds
    } else {
        Verdict::Inconclusive
    }
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

/// The poll error, `poll x (boundaries in both arms' windows)`, must be at most
/// [`POLL_ERROR_MAX`] of the control's `spec_wall`.
fn measurable(p: &Pair, poll_ms: u64) -> Option<f64> {
    let c = p.control.spec_wall_ms?;
    // Each state boundary in either arm's window is seen to within one poll.
    let bound = poll_ms * (p.control.spec_boundaries + p.treatment.spec_boundaries);
    if !p.ok() || bound as f64 > POLL_ERROR_MAX * c as f64 {
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

/// The decision table of the criteria note, first match. The texts are the
/// note's Action column word for word; a test reads the note and compares.
fn decide(e: &Evaluation) -> &'static str {
    use self::Verdict::*;
    if e.noise_gate == Fails {
        return "Stop; no phase 2 run; the owner decides";
    }
    if e.c1_safety == Fails {
        return "Fix 2a; no decision";
    }
    if e.c2_comparable == Fails {
        return "Inconclusive; no decision; rerun with a corrected rubric or task set";
    }
    // Without 8 of 10 pairs succeeding and a clean safety result "the screen is
    // inconclusive and no decision is taken": no later row is reachable.
    if e.c1_safety != Holds || e.c2_comparable != Holds {
        return "Incomplete; no decision until 1 and 2 are known to hold";
    }
    if e.c3_parallelism == Inconclusive {
        return "No decision; rerun the separable pairs with longer tasks or more repetitions";
    }
    if e.c3_parallelism == Fails {
        return "Stop concurrency work; keep `max_live_turns = 1` as the default and the only tested mode";
    }
    // Each row below names criteria that must hold, not merely not fail.
    match (e.c7_cost, e.c4_contention, e.c5_loss) {
        (Fails, _, _) => "Parallelism pays in time but not in cost; keep `max_live_turns = 1` the default, 2a stays opt-in; no CRDT",
        (Holds, Fails, _) => "2a alone is enough for spec authoring; no CRDT",
        (Holds, Holds, Fails) => "Contention exists but loses nothing the rubric values; section leases (option 4) or 2a, no CRDT",
        (Holds, Holds, Holds) => "Build `rusty_crdt` and 2b (Appendix A and B), after criterion 6's precondition: the verdict-binding change lands first",
        _ => "Incomplete; no decision until 3, 7, 4 and 5 are known to hold",
    }
}

/// What each failed pair is known to have cost, at least: it is out of
/// criterion 7, but its spend is real.
fn failed_pair_costs(pairs: &[Pair]) -> Vec<FailedCost> {
    pairs
        .iter()
        .filter(|p| !p.ok())
        .map(|p| FailedCost {
            pair: p.id.clone(),
            incomplete: [&p.control, &p.treatment]
                .iter()
                .any(|r| r.tokens.is_none() || r.killed_turns > 0),
            accounting_errors: [&p.control, &p.treatment]
                .iter()
                .flat_map(|r| r.accounting_errors.clone())
                .collect(),
            stops: [("control", &p.control), ("treatment", &p.treatment)]
                .iter()
                .filter_map(|(arm, r)| r.stop.as_ref().map(|s| format!("{arm}: {s}")))
                .collect(),
            usd_at_least: [&p.control, &p.treatment]
                .iter()
                .filter_map(|r| r.usd_floor)
                .sum(),
        })
        .filter(|f| f.usd_at_least > 0.0 || f.incomplete || !f.stops.is_empty())
        .collect()
}

/// Evaluate the criteria over `pairs` with the run's poll interval.
pub fn evaluate(pairs: &[Pair], poll_ms: u64) -> Evaluation {
    let (gate, null_ratios) = noise_gate(pairs);
    let phase2: Vec<&Pair> = pairs.iter().filter(|p| p.kind != Kind::Null).collect();
    let of = |k| -> Vec<&Pair> { phase2.iter().copied().filter(|p| p.kind == k).collect() };
    let (separable, shared) = (of(Kind::Separable), of(Kind::Shared));
    let c1 = c1(&phase2, pairs);
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
        git_sha: None,
        bbp_build_sha: None,
        failed_pair_costs: failed_pair_costs(pairs),
        total_usd_at_least: pairs
            .iter()
            .flat_map(|p| [&p.control, &p.treatment])
            .filter_map(|r| r.usd_floor)
            .sum(),
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
    /// The `reads_per_turn` every run must have been opened with.
    pub reads_per_turn: u64,
    /// The per-run cost cap C the runs used (`bbp mod --max-usd`). A run whose
    /// harnesses cost more than this counts as failed.
    pub cap_usd: f64,
    /// The committed criteria file whose prices cost the tokens.
    pub criteria: String,
    /// The git checkout the run's binaries were built from.
    pub repo: String,
    pub pairs: Vec<PairRef>,
}

#[derive(Deserialize)]
struct ModOut {
    state_ms: Vec<(String, u64)>,
    #[rusty_serde(default)]
    replay_equals_live: Option<bool>,
    #[rusty_serde(default)]
    bbp_build_sha: Option<String>,
    #[rusty_serde(default)]
    stop: Option<String>,
}

/// What `bbp mod` printed that the criteria use.
#[derive(Clone, Debug, PartialEq)]
pub struct ModFacts {
    /// The first `Planning` visit, in ms.
    pub spec_wall_ms: Option<u64>,
    /// State boundaries of that visit's window: its end, and its start unless
    /// it was the first state the moderator saw (that start is exact).
    pub spec_boundaries: u64,
    pub replay_equals_live: Option<bool>,
    /// `cap` or `wall` when the moderator stopped the run.
    pub stop: Option<String>,
}

pub fn mod_facts(mod_output: &str) -> Result<ModFacts, String> {
    let m: ModOut = rusty_serde::json::from_str(mod_output).map_err(|e| e.to_string())?;
    if m.stop
        .as_deref()
        .is_some_and(|s| !matches!(s, "cap" | "wall"))
    {
        return Err("invalid moderator stop: expected cap or wall".into());
    }
    let at = m.state_ms.iter().position(|(s, _)| s == "Planning");
    Ok(ModFacts {
        spec_wall_ms: at.map(|i| m.state_ms[i].1),
        spec_boundaries: at.map_or(0, |i| 1 + u64::from(i > 0)),
        replay_equals_live: m.replay_equals_live,
        stop: m.stop,
    })
}

/// The commit the `bbp` that produced this `bbp mod` output was built from:
/// the run's own `bbp_build_sha`, which must be a clean full commit id. An
/// absent field (a `bbp` that predates the pin), a `-dirty` build, an `unknown`
/// one and anything else that is not 40 lowercase hex digits are refused.
pub fn build_sha_of(mod_output: &str) -> Result<String, String> {
    let m: ModOut = rusty_serde::json::from_str(mod_output).map_err(|e| e.to_string())?;
    let sha = m
        .bbp_build_sha
        .ok_or("the run reported no bbp_build_sha: its bbp predates the build pin")?;
    if sha.ends_with("-dirty") {
        return Err(format!("the run's bbp was built from a dirty tree ({sha})"));
    }
    if sha == "unknown" {
        return Err("the run's bbp was built from an unknown commit".into());
    }
    if sha.len() != 40 || !sha.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return Err(format!(
            "the run's bbp_build_sha {sha:?} is not a commit id"
        ));
    }
    Ok(sha)
}

/// The first `Planning` visit of a `bbp mod` output, in ms.
pub fn spec_wall_ms(mod_output: &str) -> Result<Option<u64>, String> {
    Ok(mod_facts(mod_output)?.spec_wall_ms)
}

/// What the harness output says about models: per role the models its turns
/// used, and how many granted turns observed none.
fn model_evidence(tok: &tokens::TaskTokens) -> (BTreeMap<String, BTreeSet<String>>, u64) {
    let mut role_models: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut none = tok.without_output;
    for t in &tok.turns {
        none += u64::from(t.models.is_empty());
        role_models
            .entry(t.role.clone())
            .or_default()
            .extend(t.models.keys().cloned());
    }
    (role_models, none)
}

pub fn facts(
    r: &RunRef,
    c: &tokens::Criteria,
    sha: &str,
    cap_usd: f64,
) -> Result<RunFacts, String> {
    let task = TaskId(r.task.clone());
    let segments =
        crate::moderator::require_evidence_marker(Path::new(&r.dir), &task, cap_usd, sha)?;
    let rep = report::of(Path::new(&r.dir), &task, None)?;
    let d = crate::open_driver(Path::new(&r.dir), &task)?;
    let events = d.store.events(&task, 0).map_err(|e| format!("{e:?}"))?;
    let text =
        std::fs::read_to_string(&r.mod_output).map_err(|e| format!("{}: {e}", r.mod_output))?;
    let m = mod_facts(&text)?;
    let build_sha = build_sha_of(&text)?;
    let resumed = check_segments(&segments, &build_sha)?;
    let stop = match (resumed, &m.stop) {
        (Some(n), None) => Some(format!("resumed: {n} bbp mod segments")),
        (Some(n), Some(last)) => Some(format!("resumed: {n} bbp mod segments; last: {last}")),
        (None, s) => s.clone(),
    };
    let stopped = stop.is_some();
    // A resumed run's saved output describes only its last segment: it timed
    // the resume point as a fresh start, and its replay check says nothing of
    // an earlier segment. Both are unknown, so criterion 1 cannot hold on it.
    let (spec_wall_ms, spec_boundaries, replay_equals_live) = match resumed {
        Some(_) => (None, 0, None),
        None => (m.spec_wall_ms, m.spec_boundaries, m.replay_equals_live),
    };
    // A run the moderator stopped is a failed run whatever its log holds: a log
    // the tool cannot read (which may be why the cap stopped it) must not drop
    // the manifest. Any other run's log has to read.
    let tok = if stopped {
        tokens::of_stopped(Path::new(&r.dir), &task, &events, c)
    } else {
        tokens::of(Path::new(&r.dir), &task, &events, Some(c))?
    };
    let st = &d.state;
    let (role_models, no_model_turns) = model_evidence(&tok);
    Ok(RunFacts {
        accounting_errors: tok.errors.clone(),
        closed: st.state == State::Closed,
        rubric_ok: r.rubric_ok,
        spec_wall_ms,
        spec_boundaries,
        replay_equals_live,
        over_cap: m.stop.as_deref() == Some("cap") || tok.usd_floor.is_some_and(|f| f > cap_usd),
        tokens: (tok.without_output == 0).then_some(tok.usd).flatten(),
        overshoot: st.spent.messages > st.budget.messages || st.spent.bytes > st.budget.bytes,
        unused_conflicts: rep.unused_conflicts,
        unused_bytes: rep.unused_bytes,
        reads_per_turn: opened_reads_per_turn(&events)?,
        role_models,
        build_sha,
        no_model_turns,
        killed_turns: tok.unknown_turns,
        stop,
        stopped,
        usd_floor: tok.usd_floor,
    })
}

/// Every `bbp mod` segment that produced a run must be the build its saved
/// output reports; no record, or one naming another build, is refused. A run
/// that more than one segment produced is judged by its last output alone,
/// its earlier stops and state timings lost (#1326), so it counts as a
/// failed, stopped run under the failed-pair rule: `Some(segments)` when it
/// was resumed, `None` when one `bbp mod` produced it.
pub fn check_segments(segments: &[String], build_sha: &str) -> Result<Option<usize>, String> {
    if segments.is_empty() {
        return Err("no bbp mod segment recorded: nothing shows what produced the run".into());
    }
    if let Some((i, other)) = segments.iter().enumerate().find(|(_, s)| *s != build_sha) {
        return Err(format!(
            "its bbp mod segment {} was built from {other}, but its saved output reports {build_sha}",
            i + 1
        ));
    }
    Ok((segments.len() > 1).then_some(segments.len()))
}

/// The `reads_per_turn` a run was opened with. The final card is not the
/// run's limit: a human extension changes it after turns already ran under the
/// opened one, so the arms of a pair could differ while both cards match the
/// manifest. A run with any `reads_per_turn` extension, even one back to the
/// opened value, ran under more than one limit and refuses (#1327).
fn opened_reads_per_turn(events: &[Event]) -> Result<u64, String> {
    let Some(Event::TaskOpened { budget, .. }) = events.first() else {
        return Err("the log does not start with TaskOpened".to_owned());
    };
    let extended = events.iter().find_map(|e| match e {
        Event::BudgetExtended {
            field: BudgetField::ReadsPerTurn,
            limit,
        } => Some(*limit),
        _ => None,
    });
    match extended {
        Some(limit) => Err(format!(
            "reads_per_turn was opened at {} and extended to {limit}: the run ran under more than one limit",
            budget.reads_per_turn
        )),
        None => Ok(budget.reads_per_turn),
    }
}

/// Every run must have been produced by a `bbp` built from `head`.
pub fn check_builds(pairs: &[Pair], head: &str) -> Result<(), String> {
    let mut first: Option<(String, &str)> = None;
    for p in pairs {
        for (arm, r) in [("control", &p.control), ("treatment", &p.treatment)] {
            let who = format!("{}/{arm}", p.id);
            let sha = r.build_sha.as_str();
            match &first {
                Some((other, s)) if *s != sha => {
                    return Err(format!(
                        "runs were built from different commits: {other} from {s}, {who} from {sha}"
                    ))
                }
                None => first = Some((who.clone(), sha)),
                _ => {}
            }
            if sha != head {
                return Err(format!(
                    "{who} was built from {sha}, not the pinned HEAD {head}"
                ));
            }
        }
    }
    Ok(())
}

/// Settings every run must share. `reads_per_turn` is what the manifest says
/// for every run; each role uses exactly one model in a run, observed (an arm
/// with no model evidence, or a granted turn that observed none, cannot show
/// that the arms matched, and its dollars would be zero), the same one in
/// both arms of a pair, and an extra role of the N=2 arm uses a model the
/// control arm also used. A run that breaks this is not comparable, so the
/// evaluation refuses rather than reports.
pub fn check_settings(pairs: &[Pair], reads_per_turn: u64) -> Result<(), String> {
    for p in pairs {
        for (arm, r) in [("control", &p.control), ("treatment", &p.treatment)] {
            if r.reads_per_turn != reads_per_turn {
                return Err(format!(
                    "{}/{arm}: reads_per_turn is {}, the manifest says {reads_per_turn}",
                    p.id, r.reads_per_turn
                ));
            }
            // A stopped run is a failed run and may have printed nothing a model
            // can be read from; every other run must show its models.
            if r.stopped {
                continue;
            }
            if r.role_models.is_empty() {
                return Err(format!("{}/{arm}: no model evidence in the run", p.id));
            }
            if let Some(role) = r.role_models.iter().find(|(_, ms)| ms.is_empty()) {
                return Err(format!(
                    "{}/{arm}: no model observed for role {}",
                    p.id, role.0
                ));
            }
            if r.no_model_turns > 0 {
                return Err(format!(
                    "{}/{arm}: {} granted turn(s) with no model observation",
                    p.id, r.no_model_turns
                ));
            }
            if let Some((role, ms)) = r.role_models.iter().find(|(_, ms)| ms.len() > 1) {
                return Err(format!(
                    "{}/{arm}: role {role} used several models: {ms:?}",
                    p.id
                ));
            }
        }
        if p.control.stopped || p.treatment.stopped {
            continue;
        }
        let control: BTreeSet<&String> = p.control.role_models.values().flatten().collect();
        for (role, ms) in &p.treatment.role_models {
            match p.control.role_models.get(role) {
                Some(c) if c != ms => {
                    return Err(format!(
                        "{}: role {role} used {c:?} in the control and {ms:?} in the treatment",
                        p.id
                    ))
                }
                None if !ms.iter().all(|m| control.contains(m)) => {
                    return Err(format!(
                        "{}: extra role {role} used {ms:?}, which the control never used",
                        p.id
                    ))
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn git(repo: &Path, args: &[&str]) -> Result<(bool, String), String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    Ok((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).trim().to_owned(),
    ))
}

/// The commit a run is pinned to: HEAD of `repo`, which must be clean as
/// `git status --porcelain --untracked-files=normal` sees it (the flag overrides
/// `status.showUntrackedFiles`; excludes files, index bits such as
/// assume-unchanged, and submodule ignore settings can still hide a change) and
/// must contain `min_commit` (the merge of the verdict-binding fix). A criteria file
/// with no `min_commit` refuses: the pin is part of the criteria.
pub fn pin(repo: &Path, min_commit: Option<&str>, build_sha: &str) -> Result<String, String> {
    let min =
        min_commit.ok_or("the criteria file does not pin min_commit (the verdict-binding fix)")?;
    let (ok, head) = git(repo, &["rev-parse", "HEAD"])?;
    if !ok {
        return Err(format!("{}: not a git checkout", repo.display()));
    }
    // `--untracked-files=normal` overrides a user's `status.showUntrackedFiles=no`
    // (#1328), and a status git could not produce is not a clean tree.
    let (ok, dirty) = git(repo, &["status", "--porcelain", "--untracked-files=normal"])?;
    if !ok {
        return Err(format!("{}: git status failed", repo.display()));
    }
    if !dirty.is_empty() {
        return Err(format!("{}: the working tree is not clean", repo.display()));
    }
    let (contains, _) = git(repo, &["merge-base", "--is-ancestor", min, "HEAD"])?;
    if !contains {
        return Err(format!("HEAD {head} does not contain min_commit {min}"));
    }
    if build_sha != head {
        return Err(format!(
            "this bbp was built from {build_sha}, not from HEAD {head}: rebuild from the pinned checkout"
        ));
    }
    Ok(head)
}

/// Read `manifest` and evaluate every pair in it.
pub fn load(manifest: &Path) -> Result<Evaluation, String> {
    load_as(manifest, crate::BUILD_SHA)
}

/// [`load`] for an evaluator built from `evaluator_build` (tests pass one; a
/// test binary is not built from the checkout it inspects).
pub fn load_as(manifest: &Path, evaluator_build: &str) -> Result<Evaluation, String> {
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
                control: facts(&p.control, &c, &sha, m.cap_usd)
                    .map_err(|e| format!("{}/control: {e}", p.id))?,
                treatment: facts(&p.treatment, &c, &sha, m.cap_usd)
                    .map_err(|e| format!("{}/treatment: {e}", p.id))?,
                lost_rubric_items: p.lost_rubric_items,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let sha_run = pin(Path::new(&m.repo), c.min_commit.as_deref(), evaluator_build)?;
    check_builds(&pairs, &sha_run)?;
    check_settings(&pairs, m.reads_per_turn)?;
    let mut e = evaluate(&pairs, m.poll_ms);
    e.criteria_sha256 = Some(sha);
    e.git_sha = Some(sha_run);
    e.bbp_build_sha = Some(evaluator_build.to_owned());
    Ok(e)
}

/// [`load`] as one line of JSON.
pub fn json(manifest: &Path) -> Result<String, String> {
    rusty_serde::json::to_string(&load(manifest)?).map_err(|e| e.to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// A run that closed and passed its rubric, with these walls and tokens.
    fn run(wall: u64, tokens: u64) -> RunFacts {
        RunFacts {
            closed: true,
            rubric_ok: true,
            spec_wall_ms: Some(wall),
            spec_boundaries: 1,
            replay_equals_live: Some(true),
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
        assert!(e.decision == row(0));
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
        assert!(e.decision == row(6), "{}", e.decision);
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
        assert_eq!(e.decision, row(9));
    }

    #[test]
    fn contention_without_loss_stops_short_of_the_crdt() {
        let e = evaluate(&all([0.5; 5], 1, 0), 500);
        assert_eq!(e.c5_loss, Verdict::Fails);
        assert!(e.decision == row(8));
        let e = evaluate(&all([0.5; 5], 0, 0), 500);
        assert!(e.decision == row(7));
    }

    #[test]
    fn an_overshooting_n2_run_fails_safety_first() {
        let mut p = all([0.5; 5], 1, 1);
        let i = p.iter().position(|x| x.kind == Kind::Shared).unwrap();
        p[i].treatment.overshoot = true;
        let e = evaluate(&p, 500);
        assert_eq!(e.c1_safety, Verdict::Fails);
        assert!(e.decision == row(1));
    }

    /// #1326 review: a resumed run's replay check is unknown, so safety
    /// cannot hold on ten otherwise good pairs; it is inconclusive.
    #[test]
    fn a_resumed_run_leaves_safety_inconclusive() {
        let mut p = all([0.5; 5], 1, 1);
        assert_eq!(evaluate(&p, 500).c1_safety, Verdict::Holds);
        let i = p.iter().position(|x| x.kind == Kind::Shared).unwrap();
        p[i].treatment.stopped = true;
        p[i].treatment.stop = Some("resumed: 2 bbp mod segments".into());
        p[i].treatment.replay_equals_live = None;
        assert_eq!(evaluate(&p, 500).c1_safety, Verdict::Inconclusive);
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

    /// A scratch git repo with two commits, in its own directory per `tag`;
    /// returns (dir, first sha, second sha).
    fn repo(tag: &str) -> (std::path::PathBuf, String, String) {
        let d = std::env::temp_dir().join(format!("pin-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let g = |a: &[&str]| {
            let o = std::process::Command::new("git")
                .arg("-C")
                .arg(&d)
                .args(["-c", "user.name=t", "-c", "user.email=t@t"])
                .args(a)
                .output()
                .unwrap();
            assert!(o.status.success(), "{a:?}");
            String::from_utf8_lossy(&o.stdout).trim().to_owned()
        };
        g(&["init", "-q"]);
        std::fs::write(d.join("a"), "1").unwrap();
        g(&["add", "a"]);
        g(&["commit", "-q", "-m", "one"]);
        let first = g(&["rev-parse", "HEAD"]);
        std::fs::write(d.join("a"), "2").unwrap();
        g(&["commit", "-q", "-am", "two"]);
        let second = g(&["rev-parse", "HEAD"]);
        (d, first, second)
    }

    #[test]
    fn a_run_is_pinned_to_a_clean_head_built_into_the_binary_that_contains_the_minimum_commit() {
        let (d, first, second) = repo("pinned");
        assert_eq!(pin(&d, Some(&first), &second).unwrap(), second);
        assert_eq!(pin(&d, Some(&second), &second).unwrap(), second);
        // A binary built from another commit, or from a dirty tree, refuses.
        assert!(pin(&d, Some(&first), &first)
            .unwrap_err()
            .contains("built from"));
        assert!(pin(&d, Some(&first), &format!("{second}-dirty")).is_err());
        assert!(pin(&d, Some(&first), "unknown").is_err());
        // A commit HEAD does not contain (here: a made-up sha) refuses.
        assert!(pin(&d, Some(&"0".repeat(40)), &second).is_err());
        // No pin in the criteria refuses.
        assert!(pin(&d, None, &second).unwrap_err().contains("min_commit"));
        // A dirty tree refuses.
        std::fs::write(d.join("a"), "dirty").unwrap();
        assert!(pin(&d, Some(&first), &second)
            .unwrap_err()
            .contains("not clean"));
        // Not a repo refuses.
        assert!(pin(
            &std::env::temp_dir().join("no-such-repo-dir"),
            Some(&first),
            &second
        )
        .is_err());
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn an_untracked_file_is_dirty_even_when_git_config_hides_untracked_files() {
        // #1328: `status.showUntrackedFiles=no` (here in the repository's own
        // config, as a user's global config would set it) made `git status
        // --porcelain` print nothing for an untracked file.
        let (d, first, second) = repo("untracked");
        let cfg = std::process::Command::new("git")
            .arg("-C")
            .arg(&d)
            .args(["config", "status.showUntrackedFiles", "no"])
            .status()
            .unwrap();
        assert!(cfg.success());
        assert_eq!(pin(&d, Some(&first), &second).unwrap(), second);
        std::fs::write(d.join("new.rs"), "// untracked\n").unwrap();
        assert!(pin(&d, Some(&first), &second)
            .unwrap_err()
            .contains("not clean"));
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn a_status_git_cannot_read_is_not_a_clean_tree() {
        // A failed `git status` printed nothing, which read as clean. A corrupt
        // index fails status while `rev-parse HEAD` still answers.
        let (d, first, second) = repo("corrupt-index");
        std::fs::write(d.join(".git/index"), "not an index").unwrap();
        let err = pin(&d, Some(&first), &second).unwrap_err();
        assert!(err.contains("git status"), "{err}");
        std::fs::remove_dir_all(d).unwrap();
    }

    fn with_models(mut r: RunFacts, rpt: u64, roles: &[(&str, &[&str])]) -> RunFacts {
        r.reads_per_turn = rpt;
        r.role_models = roles
            .iter()
            .map(|(role, ms)| {
                (
                    (*role).to_owned(),
                    ms.iter().map(|m| (*m).to_owned()).collect(),
                )
            })
            .collect();
        r
    }

    fn model_pair(c: &[(&str, &[&str])], t: &[(&str, &[&str])]) -> Pair {
        pair(
            "p",
            Kind::Separable,
            with_models(run(1, 1), 30, c),
            with_models(run(1, 1), 30, t),
        )
    }

    #[test]
    fn one_model_per_role_identical_across_arms_is_accepted() {
        let p = model_pair(
            &[("planner", &["m1"]), ("coder", &["m2"])],
            &[
                ("planner", &["m1"]),
                ("coder", &["m2"]),
                ("planner2", &["m1"]),
            ],
        );
        assert_eq!(check_settings(&[p], 30), Ok(()));
    }

    #[test]
    fn a_role_on_two_models_or_a_different_model_in_the_other_arm_refuses() {
        let two = model_pair(&[("planner", &["m1", "m2"])], &[("planner", &["m1"])]);
        assert!(check_settings(&[two], 30)
            .unwrap_err()
            .contains("several models"));
        let differs = model_pair(&[("planner", &["m1"])], &[("planner", &["m2"])]);
        let err = check_settings(&[differs], 30).unwrap_err();
        assert!(
            err.contains("control") && err.contains("treatment"),
            "{err}"
        );
        // An extra N=2 role on a model the control never used refuses.
        let extra = model_pair(
            &[("planner", &["m1"])],
            &[("planner", &["m1"]), ("planner2", &["m9"])],
        );
        assert!(check_settings(&[extra], 30)
            .unwrap_err()
            .contains("extra role"));
    }

    #[test]
    fn a_run_whose_reads_per_turn_is_not_the_manifests_refuses() {
        let mut p = model_pair(&[("planner", &["m1"])], &[("planner", &["m1"])]);
        assert_eq!(check_settings(&[p.clone()], 30), Ok(()));
        assert!(check_settings(&[p.clone()], 20)
            .unwrap_err()
            .contains("reads_per_turn"));
        p.treatment.reads_per_turn = 10;
        assert!(check_settings(&[p], 30).unwrap_err().contains("treatment"));
    }

    // ---- the decision table, row by row

    /// An evaluation in which every criterion holds.
    fn all_hold() -> Evaluation {
        let mut e = evaluate(&[], 500);
        e.noise_gate = Verdict::Holds;
        e.c1_safety = Verdict::Holds;
        e.c2_comparable = Verdict::Holds;
        e.c3_parallelism = Verdict::Holds;
        e.c4_contention = Verdict::Holds;
        e.c5_loss = Verdict::Holds;
        e.c7_cost = Verdict::Holds;
        e
    }

    /// One condition per row, in table order, with the text its row starts with.
    type Row = (&'static str, fn(&mut Evaluation), usize);
    const ROWS: [Row; 9] = [
        ("noise gate", |e| e.noise_gate = Verdict::Fails, 0),
        ("1 fails", |e| e.c1_safety = Verdict::Fails, 1),
        ("2 fails", |e| e.c2_comparable = Verdict::Fails, 2),
        (
            "3 inconclusive",
            |e| e.c3_parallelism = Verdict::Inconclusive,
            4,
        ),
        ("3 fails", |e| e.c3_parallelism = Verdict::Fails, 5),
        ("7 fails", |e| e.c7_cost = Verdict::Fails, 6),
        ("4 fails", |e| e.c4_contention = Verdict::Fails, 7),
        ("5 fails", |e| e.c5_loss = Verdict::Fails, 8),
        (
            "incomplete",
            |e| e.c4_contention = Verdict::Inconclusive,
            10,
        ),
    ];

    /// Row `i` of the note's decision table.
    fn row(i: usize) -> String {
        note_actions()[i].clone()
    }

    #[test]
    fn all_holding_builds_after_the_preconditions() {
        assert_eq!(decide(&all_hold()), row(9));
    }

    #[test]
    fn every_decision_row_fires_alone() {
        for (name, cond, want) in ROWS {
            let mut e = all_hold();
            cond(&mut e);
            assert_eq!(decide(&e), row(want), "{name}");
        }
    }

    #[test]
    fn the_first_matching_row_wins_over_every_later_one() {
        // The last row is the fallback, not a condition, so it is never applied
        // as a "later" row.
        let conditions = &ROWS[..ROWS.len() - 1];
        for (i, (name, cond, want)) in conditions.iter().enumerate() {
            // Every later row's condition holds too. They are applied latest
            // first so an earlier one wins any field two of them share, and
            // this row's last so it wins its own.
            let mut e = all_hold();
            for (_, later, _) in conditions[i + 1..].iter().rev() {
                later(&mut e);
            }
            cond(&mut e);
            assert_eq!(decide(&e), row(*want), "{name}");
        }
    }

    #[test]
    fn a_row_is_not_rescued_by_a_later_criterion_holding() {
        // 3 fails with 4, 5 and 7 all holding is still "3 fails".
        let mut e = all_hold();
        e.c3_parallelism = Verdict::Fails;
        assert!(decide(&e) == row(5));
        // 3 holding and 7 failing stays "7 fails" whatever 4 and 5 show.
        let mut e = all_hold();
        e.c7_cost = Verdict::Fails;
        e.c4_contention = Verdict::Holds;
        assert!(decide(&e) == row(6));
        e.c4_contention = Verdict::Fails;
        e.c5_loss = Verdict::Fails;
        assert!(decide(&e) == row(6));
    }

    #[test]
    fn an_inconclusive_criterion_is_never_a_build() {
        for f in [
            (|e: &mut Evaluation| e.c1_safety = Verdict::Inconclusive) as fn(&mut Evaluation),
            |e| e.c2_comparable = Verdict::Inconclusive,
            |e| e.c4_contention = Verdict::Inconclusive,
            |e| e.c5_loss = Verdict::Inconclusive,
            |e| e.c7_cost = Verdict::Inconclusive,
        ] {
            let mut e = all_hold();
            f(&mut e);
            assert!(decide(&e).starts_with("Incomplete"), "{}", decide(&e));
        }
    }

    // ---- review probes C, D, E: no decisive row from inconclusive criteria

    /// Nulls, `sep` separable pairs at `ratio`, and 5 shared pairs.
    fn partial(sep: usize, ratio: f64, tokens_ratio: f64) -> Vec<Pair> {
        let mut p = nulls([1.0, 0.95, 1.1]);
        p.extend((0..sep).map(|i| {
            let mut t = run((100_000.0 * ratio) as u64, 100);
            t.tokens = Some(100.0 * tokens_ratio);
            pair(&format!("s{i}"), Kind::Separable, run(100_000, 100), t)
        }));
        p.extend(shared(1, 1));
        p
    }

    #[test]
    fn probe_c_four_separable_pairs_with_a_failing_ratio_is_not_a_decision() {
        let e = evaluate(&partial(4, 0.9, 1.0), 500);
        assert_eq!(e.c3_parallelism, Verdict::Fails, "the ratio does fail");
        assert_eq!(
            (e.c1_safety, e.c2_comparable),
            (Verdict::Inconclusive, Verdict::Inconclusive)
        );
        assert!(e.decision.starts_with("Incomplete"), "{}", e.decision);
    }

    #[test]
    fn probe_d_four_separable_pairs_with_a_cost_failure_is_not_a_decision() {
        let mut p = partial(4, 0.5, 2.0);
        for x in p.iter_mut().filter(|x| x.kind != Kind::Null) {
            x.treatment.tokens = Some(200.0);
        }
        let e = evaluate(&p, 500);
        assert_eq!(e.c7_cost, Verdict::Fails);
        assert!(e.decision.starts_with("Incomplete"), "{}", e.decision);
    }

    #[test]
    fn probe_e_unknown_tokens_with_a_failing_contention_is_not_a_decision() {
        let mut p = all([0.5; 5], 0, 1);
        let i = p.iter().position(|x| x.kind == Kind::Shared).unwrap();
        p[i].treatment.tokens = None;
        let e = evaluate(&p, 500);
        assert_eq!(
            (e.c7_cost, e.c4_contention),
            (Verdict::Inconclusive, Verdict::Fails)
        );
        assert!(e.decision.starts_with("Incomplete"), "{}", e.decision);
    }

    // ---- boundaries the review's surviving mutations crossed

    fn verdicts_of(p: &[Pair]) -> Evaluation {
        evaluate(p, 500)
    }

    #[test]
    fn criterion_one_needs_the_replay_to_equal_the_live_state_in_every_run() {
        let mut p = all([0.5; 5], 1, 1);
        p[4].control.replay_equals_live = None;
        assert_eq!(verdicts_of(&p).c1_safety, Verdict::Inconclusive);
        p[4].treatment.replay_equals_live = Some(false);
        assert_eq!(verdicts_of(&p).c1_safety, Verdict::Fails);
        assert!(verdicts_of(&p).decision == row(1));
    }

    #[test]
    fn a_run_over_the_cap_is_a_failed_run() {
        let mut r = run(1, 1);
        assert!(r.succeeded());
        r.over_cap = true;
        assert!(!r.succeeded());
        let mut p = all([0.5; 5], 1, 1);
        for x in p.iter_mut().filter(|x| x.kind == Kind::Shared) {
            x.treatment.over_cap = true;
        }
        assert_eq!(verdicts_of(&p).c2_comparable, Verdict::Fails);
    }

    #[test]
    fn the_poll_bound_counts_the_boundaries_of_both_arms() {
        let mut p = all([0.5; 5], 1, 1);
        for x in p.iter_mut().filter(|x| x.kind == Kind::Separable) {
            x.control.spec_wall_ms = Some(20_000);
            x.treatment.spec_wall_ms = Some(10_000);
        }
        assert_eq!(verdicts_of(&p).separable_ratios.len(), 5);
        // One more boundary on the treatment side: 1500 ms > 5% of 20 s.
        for x in p.iter_mut().filter(|x| x.kind == Kind::Separable) {
            x.treatment.spec_boundaries = 2;
        }
        assert_eq!(verdicts_of(&p).c3_parallelism, Verdict::Inconclusive);
    }

    #[test]
    fn criterion_one_needs_all_ten_pairs() {
        let nine = [
            nulls([1.0; 3]),
            separable([0.5; 5]),
            shared(1, 1)[..4].to_vec(),
        ]
        .concat();
        assert_eq!(verdicts_of(&nine).c1_safety, Verdict::Inconclusive);
        assert_eq!(verdicts_of(&all([0.5; 5], 1, 1)).c1_safety, Verdict::Holds);
    }

    #[test]
    fn criterion_two_needs_ten_pairs_and_fails_early_only_past_two_failures() {
        let nine_ok = [
            nulls([1.0; 3]),
            separable([0.5; 5]),
            shared(1, 1)[..4].to_vec(),
        ]
        .concat();
        assert_eq!(verdicts_of(&nine_ok).c2_comparable, Verdict::Inconclusive);
        let with_failed = |n: usize| {
            let mut p = nine_ok.clone();
            let idx: Vec<usize> = p
                .iter()
                .enumerate()
                .filter(|(_, x)| x.kind != Kind::Null)
                .map(|(i, _)| i)
                .collect();
            for i in &idx[..n] {
                p[*i].control.closed = false;
                p[*i].treatment.closed = false;
            }
            p
        };
        assert_eq!(
            verdicts_of(&with_failed(2)).c2_comparable,
            Verdict::Inconclusive,
            "two failed: not yet"
        );
        assert_eq!(
            verdicts_of(&with_failed(3)).c2_comparable,
            Verdict::Fails,
            "three failed: already lost"
        );
    }

    #[test]
    fn criterion_three_boundaries() {
        // A ratio of exactly 1.0 is not inverted: five at 1.0 fail on the
        // median, but [0.5 x3, 1.0 x2] holds.
        assert_eq!(
            verdicts_of(&all([0.5, 0.5, 0.5, 1.0, 1.0], 1, 1)).c3_parallelism,
            Verdict::Holds
        );
        // A median of exactly 0.75 holds; just above it fails.
        assert_eq!(
            verdicts_of(&all([0.75; 5], 1, 1)).c3_parallelism,
            Verdict::Holds
        );
        assert_eq!(
            verdicts_of(&all([0.76; 5], 1, 1)).c3_parallelism,
            Verdict::Fails
        );
        // Two ratios above 1.0 fail even with a low median; one is allowed.
        assert_eq!(
            verdicts_of(&all([0.5, 0.5, 0.5, 1.01, 1.01], 1, 1)).c3_parallelism,
            Verdict::Fails
        );
        assert_eq!(
            verdicts_of(&all([0.5, 0.5, 0.5, 0.5, 1.01], 1, 1)).c3_parallelism,
            Verdict::Holds
        );
    }

    #[test]
    fn a_poll_of_exactly_five_percent_of_the_control_is_still_measurable() {
        // One boundary a side at 500 ms: 1000 ms against a 20 s control is 5%.
        let mut p = all([0.5; 5], 1, 1);
        for x in p.iter_mut().filter(|x| x.kind == Kind::Separable) {
            x.control.spec_wall_ms = Some(20_000);
            x.treatment.spec_wall_ms = Some(10_000);
        }
        assert_eq!(verdicts_of(&p).separable_ratios.len(), 5);
        // Just under: 19_999 ms makes every pair unmeasurable.
        for x in p.iter_mut().filter(|x| x.kind == Kind::Separable) {
            x.control.spec_wall_ms = Some(19_999);
        }
        assert_eq!(verdicts_of(&p).c3_parallelism, Verdict::Inconclusive);
    }

    /// (succeeded, unused conflicts, lost rubric items, unused/all spec bytes)
    type SharedSpec = (bool, u64, Option<u64>, (u64, u64));

    fn shared_with(spec: &[SharedSpec]) -> Vec<Pair> {
        spec.iter()
            .enumerate()
            .map(|(i, (ok, conflicts, lost, bytes))| {
                let mut t = run(100_000, 100);
                t.closed = *ok;
                t.unused_conflicts = *conflicts;
                t.unused_bytes = *bytes;
                let mut p = pair(&format!("h{i}"), Kind::Shared, run(100_000, 100), t);
                p.lost_rubric_items = *lost;
                p
            })
            .collect()
    }

    fn with_rest(shared: Vec<Pair>) -> Vec<Pair> {
        [nulls([1.0; 3]), separable([0.5; 5]), shared].concat()
    }

    #[test]
    fn criterion_four_boundaries() {
        let hit = (true, 1, Some(1), (30, 100));
        let miss = (true, 0, Some(1), (30, 100));
        let v = |s: &[_]| verdicts_of(&with_rest(shared_with(s))).c4_contention;
        assert_eq!(
            v(&[hit, hit, hit, miss, miss]),
            Verdict::Holds,
            "three of five"
        );
        assert_eq!(
            v(&[hit, hit, miss, miss, miss]),
            Verdict::Fails,
            "two of five"
        );
        // A hit in a failed pair does not count.
        let failed_hit = (false, 1, Some(1), (30, 100));
        assert_eq!(v(&[hit, hit, failed_hit, miss, miss]), Verdict::Fails);
        // With fewer than five shared pairs the answer is not known yet.
        assert_eq!(v(&[hit, hit, miss, miss]), Verdict::Inconclusive);
        assert_eq!(
            v(&[hit, hit, hit, miss]),
            Verdict::Holds,
            "three hits are enough early"
        );
    }

    #[test]
    fn criterion_five_boundaries() {
        let lost = |n, share| (true, 1, Some(n), (share, 100));
        let v = |s: &[_]| verdicts_of(&with_rest(shared_with(s))).c5_loss;
        // Three pairs with lost items and a median share of exactly 0.15.
        let at = lost(1, 15);
        let none = lost(0, 15);
        assert_eq!(v(&[at, at, at, none, none]), Verdict::Holds);
        assert_eq!(
            v(&[at, at, none, none, none]),
            Verdict::Fails,
            "two lost is not three"
        );
        // The median share is a floor: just under fails.
        assert_eq!(
            v(&[
                lost(1, 14),
                lost(1, 14),
                lost(1, 14),
                lost(0, 14),
                lost(0, 14)
            ]),
            Verdict::Fails
        );
        // Lost items with a negligible share fail on the share.
        assert_eq!(v(&[lost(1, 1); 5]), Verdict::Fails);
        // An ungraded pair makes it unknown, not a pass.
        let ungraded = (true, 1, None, (15, 100));
        assert_eq!(v(&[at, at, at, at, ungraded]), Verdict::Inconclusive);
    }

    #[test]
    fn failed_pairs_do_not_count_toward_the_cost_ratio() {
        // Four good pairs at 1.0x and six failed pairs at 5x: only the good
        // ones are the median.
        let mut p = nulls([1.0; 3]);
        for i in 0..10 {
            let ok = i < 4;
            let mut t = run(100_000, 500);
            t.closed = ok;
            p.push(pair(
                &format!("p{i}"),
                if i < 5 { Kind::Separable } else { Kind::Shared },
                run(100_000, 100),
                t,
            ));
            if ok {
                p.last_mut().unwrap().treatment.tokens = Some(100.0);
            }
        }
        assert_eq!(verdicts_of(&p).c7_cost, Verdict::Holds);
    }

    // ---- build identity of the producers

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn out(sha: Option<&str>) -> String {
        match sha {
            Some(s) => format!(
                r#"{{"state":"Closed","state_ms":[["Planning",10]],"bbp_build_sha":"{s}"}}"#
            ),
            None => r#"{"state":"Closed","state_ms":[["Planning",10]]}"#.to_owned(),
        }
    }

    #[test]
    fn a_run_reports_the_commit_its_bbp_was_built_from() {
        assert_eq!(build_sha_of(&out(Some(A))).unwrap(), A);
    }

    #[test]
    fn a_run_with_no_clean_build_identity_is_refused() {
        let missing = build_sha_of(&out(None)).unwrap_err();
        assert!(missing.contains("bbp_build_sha"), "{missing}");
        let dirty = build_sha_of(&out(Some(&format!("{A}-dirty")))).unwrap_err();
        assert!(dirty.contains("dirty"), "{dirty}");
        let unknown = build_sha_of(&out(Some("unknown"))).unwrap_err();
        assert!(unknown.contains("unknown"), "{unknown}");
        for bad in ["", "abc", &A.to_uppercase(), &format!("{A}0")] {
            assert!(build_sha_of(&out(Some(bad))).is_err(), "{bad:?}");
        }
    }

    fn built(sha: &str) -> RunFacts {
        RunFacts {
            build_sha: sha.to_owned(),
            ..run(1, 1)
        }
    }

    #[test]
    fn every_run_must_be_built_from_the_pinned_head() {
        let ok = pair("p", Kind::Separable, built(A), built(A));
        assert_eq!(check_builds(&[ok], A), Ok(()));
        let old = pair("p", Kind::Separable, built(A), built(B));
        let err = check_builds(&[old], A).unwrap_err();
        assert!(err.contains("p/treatment") && err.contains(B), "{err}");
    }

    #[test]
    fn runs_built_from_different_commits_are_refused_even_if_one_matches() {
        let mixed = pair("p", Kind::Separable, built(A), built(B));
        let err = check_builds(&[mixed], A).unwrap_err();
        assert!(err.contains("p/treatment"), "{err}");
        // Two pairs, each internally consistent, built from different commits.
        let a = pair("a", Kind::Separable, built(A), built(A));
        let b = pair("b", Kind::Separable, built(B), built(B));
        let err = check_builds(&[a, b], A).unwrap_err();
        assert!(err.contains("b/control"), "{err}");
        // A run that reported nothing is not a match for any head.
        let none = pair("p", Kind::Separable, built(A), built(""));
        assert!(check_builds(&[none], A).is_err());
    }

    // ---- model evidence

    #[test]
    fn an_arm_with_no_model_evidence_refuses() {
        // Two arms with no roles at all: matching emptiness is not agreement.
        let none = model_pair(&[], &[]);
        let err = check_settings(&[none], 30).unwrap_err();
        assert!(err.contains("no model"), "{err}");
        // A role present with no model observed.
        let empty = model_pair(&[("planner", &[])], &[("planner", &[])]);
        let err = check_settings(&[empty], 30).unwrap_err();
        assert!(err.contains("no model") && err.contains("planner"), "{err}");
    }

    #[test]
    fn a_granted_turn_with_no_model_observation_refuses_even_if_others_have_one() {
        let mut p = model_pair(&[("planner", &["m1"])], &[("planner", &["m1"])]);
        assert_eq!(check_settings(&[p.clone()], 30), Ok(()));
        p.treatment.no_model_turns = 1;
        let err = check_settings(&[p], 30).unwrap_err();
        assert!(
            err.contains("p/treatment") && err.contains("no model"),
            "{err}"
        );
    }

    fn turn(role: &str, stream: &str) -> tokens::TurnTokens {
        let result = tokens::last_result(stream)
            .map(|r| r.models)
            .unwrap_or_default();
        tokens::TurnTokens {
            turn: 1,
            role: role.to_owned(),
            revoked: false,
            complete: true,
            unknown: None,
            usage: tokens::Usage::default(),
            usd: None,
            usd_floor: None,
            log: tokens::scan(stream).0,
            cache_ttl: None,
            models: if result.is_empty() {
                tokens::by_model(stream)
            } else {
                result
            },
        }
    }

    fn task_tokens(turns: Vec<tokens::TurnTokens>, without_output: u64) -> tokens::TaskTokens {
        tokens::TaskTokens {
            errors: Vec::new(),
            turns,
            sum: tokens::Usage::default(),
            usd: None,
            usd_floor: None,
            total: 0,
            revoked: 0,
            without_output,
            unknown_turns: 0,
            cache_ttl: None,
        }
    }

    const ASSISTANT: &str = r#"{"type":"assistant","message":{"id":"a","model":"m1","usage":{"input_tokens":1,"output_tokens":1}}}"#;
    const RESULT_ONLY: &str = r#"{"type":"result","usage":{"input_tokens":5,"output_tokens":5}}"#;

    #[test]
    fn a_result_with_no_model_usage_names_no_model() {
        let t = turn("planner", RESULT_ONLY);
        assert!(t.models.is_empty());
        let (models, none) = model_evidence(&task_tokens(vec![t], 0));
        assert_eq!(none, 1);
        assert!(models["planner"].is_empty(), "{models:?}");
    }

    #[test]
    fn model_evidence_counts_turns_that_observed_no_model_or_left_no_output() {
        let seen = turn("planner", &format!("{ASSISTANT}\n{RESULT_ONLY}"));
        let blind = turn("coder", RESULT_ONLY);
        let (models, none) = model_evidence(&task_tokens(vec![seen, blind], 1));
        assert_eq!(none, 2, "one result-only turn and one with no output");
        assert_eq!(
            models["planner"].iter().collect::<Vec<_>>(),
            [&"m1".to_owned()]
        );
        assert!(models["coder"].is_empty());
    }

    // ---- second review: findings 6, 7, 8

    #[test]
    fn criterion_one_includes_the_null_pairs_repeat_runs() {
        let mut p = all([0.5; 5], 1, 1);
        assert_eq!(evaluate(&p, 500).c1_safety, Verdict::Holds);
        let null = p.iter().position(|x| x.kind == Kind::Null).unwrap();
        p[null].treatment.replay_equals_live = Some(false);
        assert_eq!(evaluate(&p, 500).c1_safety, Verdict::Fails);
        p[null].treatment.replay_equals_live = None;
        assert_eq!(evaluate(&p, 500).c1_safety, Verdict::Inconclusive);
        p[null].treatment.replay_equals_live = Some(true);
        p[null].control.replay_equals_live = Some(false);
        assert_eq!(evaluate(&p, 500).c1_safety, Verdict::Fails);
    }

    fn visits(states: &[&str]) -> String {
        let v: Vec<String> = states.iter().map(|s| format!("[\"{s}\",100]")).collect();
        format!(r#"{{"state_ms":[{}]}}"#, v.join(","))
    }

    #[test]
    fn the_planning_window_has_the_boundaries_it_has() {
        // Planning first: the start is exact, so only its end is a boundary.
        let first = mod_facts(&visits(&["Planning", "PlanGate"])).unwrap();
        assert_eq!(first.spec_boundaries, 1);
        // Entered from another state: its start and its end.
        let later = mod_facts(&visits(&["Assigned", "Planning", "PlanGate"])).unwrap();
        assert_eq!(later.spec_boundaries, 2);
        // A first Planning visit after earlier states is still two, not index + 1.
        let third = mod_facts(&visits(&["Assigned", "Open", "Planning", "PlanGate"])).unwrap();
        assert_eq!(third.spec_boundaries, 2);
        let none = mod_facts(&visits(&["Assigned"])).unwrap();
        assert_eq!((none.spec_wall_ms, none.spec_boundaries), (None, 0));
    }

    const NOTE: &str = include_str!("../../../../docs/bbp-concurrency-design.md");

    /// The decision table of the note: the Action column, in order.
    fn note_actions() -> Vec<String> {
        let at = NOTE
            .find("| Outcome | Action |")
            .expect("the decision table");
        NOTE[at..]
            .lines()
            .skip(2)
            .take_while(|l| l.starts_with('|'))
            .map(|l| {
                let cells: Vec<&str> = l.trim_matches('|').splitn(2, '|').collect();
                cells[1].trim().to_owned()
            })
            .collect()
    }

    #[test]
    fn the_code_says_what_the_note_says_row_by_row() {
        let actions = note_actions();
        let rows: [fn(&mut Evaluation); 11] = [
            |e| e.noise_gate = Verdict::Fails,
            |e| e.c1_safety = Verdict::Fails,
            |e| e.c2_comparable = Verdict::Fails,
            |e| e.c1_safety = Verdict::Inconclusive,
            |e| e.c3_parallelism = Verdict::Inconclusive,
            |e| e.c3_parallelism = Verdict::Fails,
            |e| e.c7_cost = Verdict::Fails,
            |e| e.c4_contention = Verdict::Fails,
            |e| e.c5_loss = Verdict::Fails,
            |_| {},
            |e| e.c4_contention = Verdict::Inconclusive,
        ];
        assert_eq!(actions.len(), rows.len(), "{actions:#?}");
        for (i, (cond, action)) in rows.iter().zip(&actions).enumerate() {
            let mut e = all_hold();
            cond(&mut e);
            assert_eq!(&decide(&e), action, "row {i}");
        }
    }

    #[test]
    fn rows_four_and_five_need_their_criteria_to_hold_not_merely_not_fail() {
        use Verdict::*;
        let all = [Holds, Fails, Inconclusive];
        for c7 in all {
            for c4 in all {
                for c5 in all {
                    let mut e = all_hold();
                    (e.c7_cost, e.c4_contention, e.c5_loss) = (c7, c4, c5);
                    let got = decide(&e);
                    let want = match (c7, c4, c5) {
                        (Fails, _, _) => note_actions()[6].clone(),
                        (Holds, Fails, _) => note_actions()[7].clone(),
                        (Holds, Holds, Fails) => note_actions()[8].clone(),
                        (Holds, Holds, Holds) => note_actions()[9].clone(),
                        _ => note_actions()[10].clone(),
                    };
                    assert_eq!(got, want, "{c7:?} {c4:?} {c5:?}");
                }
            }
        }
    }

    // ---- round two: a killed turn fails its pair; its cost stays counted

    fn killed(mut r: RunFacts, floor: f64) -> RunFacts {
        r.killed_turns = 1;
        r.usd_floor = Some(floor);
        r
    }

    #[test]
    fn a_run_with_a_killed_turn_is_a_failed_run() {
        assert!(run(1, 1).succeeded());
        assert!(!killed(run(1, 1), 0.5).succeeded());
    }

    #[test]
    fn with_healthy_controls_criterion_two_allows_one_killed_builder_pair() {
        let lose = |n: usize| {
            let mut p = all([0.5; 5], 1, 1);
            for x in p.iter_mut().filter(|x| x.kind == Kind::Shared).take(n) {
                x.treatment = killed(run(1, 1), 1.0);
            }
            evaluate(&p, 500)
        };
        assert_eq!(lose(0).c2_comparable, Verdict::Holds);
        // N=2 failures may exceed N=1's by one.
        assert_eq!(lose(1).c2_comparable, Verdict::Holds);
        assert_eq!(lose(2).c2_comparable, Verdict::Fails);
        assert!(lose(2).decision.starts_with("Inconclusive"));
    }

    #[test]
    fn overlapping_control_failure_allows_two_killed_builder_pairs() {
        let mut p = all([0.5; 5], 1, 1);
        for x in p.iter_mut().filter(|x| x.kind == Kind::Shared).take(2) {
            x.treatment = killed(run(1, 1), 1.0);
        }
        p.iter_mut()
            .find(|x| x.kind == Kind::Shared)
            .unwrap()
            .control
            .rubric_ok = false;

        let e = evaluate(&p, 500);
        assert_eq!(e.c2_comparable, Verdict::Holds);
        assert_eq!(e.c4_contention, Verdict::Holds);
        assert_eq!(e.c5_loss, Verdict::Holds);
        assert!(e.decision.starts_with("Build `rusty_crdt`"));
    }

    #[test]
    fn a_pair_with_a_killed_turn_is_left_out_of_the_cost_ratio_and_its_cost_is_reported_apart() {
        let mut p = all([0.5; 5], 1, 1);
        for x in &mut p {
            x.control.usd_floor = Some(1.0);
            x.treatment.usd_floor = Some(1.0);
        }
        let honest = evaluate(&p, 500);
        assert_eq!(honest.failed_pair_costs, vec![]);
        // One shared pair's treatment run lost a builder to a revocation: its
        // known cost is $3, its tokens would have made the ratio 9x.
        let i = p.iter().position(|x| x.kind == Kind::Shared).unwrap();
        let id = p[i].id.clone();
        p[i].treatment = killed(
            RunFacts {
                tokens: Some(900.0),
                ..run(100_000, 100)
            },
            3.0,
        );
        let e = evaluate(&p, 500);
        assert_eq!(
            e.cost_ratios.len(),
            honest.cost_ratios.len() - 1,
            "left out of 7"
        );
        assert_eq!(e.c7_cost, honest.c7_cost);
        assert_eq!(
            e.failed_pair_costs,
            vec![FailedCost {
                pair: id,
                usd_at_least: 4.0,
                incomplete: true,
                accounting_errors: Vec::new(),
                stops: Vec::new(),
            }]
        );
        // The killed run's cost is in the total all the same.
        assert!(
            near_f(e.total_usd_at_least, 2.0 * p.len() as f64 + 2.0),
            "{}",
            e.total_usd_at_least
        );
    }

    /// #1326: a resumed run is a failed pair, reported with why it stopped,
    /// even when its known cost is zero.
    #[test]
    fn a_resumed_run_fails_its_pair_and_the_report_says_resumed() {
        let mut resumed = run(100, 100);
        resumed.stopped = true;
        resumed.stop = Some("resumed: 2 bbp mod segments".into());
        assert!(!resumed.succeeded());
        let p = vec![pair("p1", Kind::Separable, run(100, 100), resumed)];
        let e = evaluate(&p, 500);
        assert_eq!(e.failed_pair_costs.len(), 1, "{:?}", e.failed_pair_costs);
        assert_eq!(
            e.failed_pair_costs[0].stops,
            vec!["treatment: resumed: 2 bbp mod segments".to_owned()]
        );
    }

    fn near_f(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    // ---- third review, finding 2: a stopped run is a failed run, not a refusal

    #[test]
    fn a_stopped_run_needs_no_model_evidence_to_load() {
        let mut p = model_pair(&[("planner", &["m1"])], &[("planner", &["m1"])]);
        // The treatment was stopped before it printed a message.
        p.treatment.stopped = true;
        p.treatment.role_models.clear();
        p.treatment.no_model_turns = 1;
        assert_eq!(check_settings(&[p.clone()], 30), Ok(()));
        // A run that was not stopped still needs it.
        p.treatment.stopped = false;
        assert!(check_settings(&[p], 30).is_err());
    }
    #[test]
    fn unreadable_turn_does_not_erase_failed_pair_spend() {
        let mut stopped = run(1, 0);
        stopped.stopped = true;
        stopped.killed_turns = 1;
        stopped.tokens = None;
        stopped.usd_floor = Some(10.0);
        stopped
            .accounting_errors
            .push("turn 2: unreadable log".into());
        let p = pair("mixed", Kind::Shared, run(1, 0), stopped);
        let e = evaluate(&[p], 500);
        assert_eq!(e.failed_pair_costs[0].usd_at_least, 10.0);
        assert!(e.failed_pair_costs[0].incomplete);
        assert_eq!(e.failed_pair_costs[0].accounting_errors.len(), 1);
        assert_eq!(e.total_usd_at_least, 10.0);
    }

    #[test]
    fn criterion_one_counts_overshoot_in_the_n2_runs_only() {
        let mut p = all([0.5; 5], 1, 1);
        let null = p.iter().position(|x| x.kind == Kind::Null).unwrap();
        // A null pair's repeat is an N=1 run: a budget overshoot there is not
        // the N=2 safety property.
        p[null].treatment.overshoot = true;
        assert_eq!(evaluate(&p, 500).c1_safety, Verdict::Holds);
        let shared = p.iter().position(|x| x.kind == Kind::Shared).unwrap();
        p[shared].treatment.overshoot = true;
        assert_eq!(evaluate(&p, 500).c1_safety, Verdict::Fails);
    }
}
