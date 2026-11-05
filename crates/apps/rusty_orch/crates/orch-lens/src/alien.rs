//! The alien review lens's pure core (#708, design D75): hunt defects the
//! ordinary review lenses are unlikely to find. Shadow-only: nothing here
//! posts to the board.
//!
//! One hunt on one candidate: the evidence already on the card is the
//! *available* set; an enumerate reply ([`parse_suspects`]) names suspects,
//! each a site crossed with a defect [`Mechanism`]; [`rank`] keeps the most
//! unavailable, coherent ones; a hunt reply per kept suspect
//! ([`parse_probe`]) is one reproducer test or nothing. [`eligible`] says
//! whether a claim gets a hunt at all. Running probes is the shell's job.

use std::collections::BTreeSet;

use rusty_bbp::{MsgId, Sha256};
use rusty_json::{Map, Value};

/// Most suspects one enumerate reply may name.
pub const MAX_SUSPECTS: usize = 12;
/// Suspects kept for a directed hunt.
pub const TOP_K: usize = 3;
/// Longest suspect site or sketch, in characters.
pub const MAX_FIELD: usize = 300;
/// Longest reproducer test, in bytes.
pub const MAX_PROBE: usize = 16 * 1024;
/// Longest candidate diff text a hunt is run on, in bytes (about 32k
/// tokens). A longer one is skipped, never truncated (D87).
pub const MAX_DIFF: usize = 128 * 1024;

/// One defect mechanism a suspect can name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mechanism {
    pub id: &'static str,
    /// What to look for, for the enumerate prompt.
    pub look_for: &'static str,
    /// Lowercase words that tag an ordinary evidence note with this
    /// mechanism, for [`unavailability`].
    pub keywords: &'static [&'static str],
}

/// The Rust list (design §4). Languages without a list are not eligible.
pub const RUST: &[Mechanism] = &[
    m(
        "rs-cast",
        "lossy `as` casts and integer overflow or truncation on lengths, offsets and sizes",
        &["overflow", "truncat", "cast", "wrap"],
    ),
    m(
        "rs-panic",
        "reachable unwrap, expect, indexing, slicing or division panics on external input",
        &["panic", "unwrap", "out of bounds", "divide by zero"],
    ),
    m(
        "rs-utf8",
        "byte vs char boundaries, invalid UTF-8, non-UTF-8 paths and OS strings",
        &["utf-8", "utf8", "char boundary", "osstr"],
    ),
    m(
        "rs-path",
        "path traversal, symlinks, `..`, absolute paths, check-then-use races",
        &["traversal", "symlink", "toctou", "canonical"],
    ),
    m(
        "rs-err",
        "errors swallowed, mapped to success, or lost with ok() or let _ =",
        &["swallow", "ignored error", "silently"],
    ),
    m(
        "rs-drop",
        "resource leaks or partial writes on early return, Drop side effects",
        &["leak", "partial write", "drop"],
    ),
    m(
        "rs-atomic",
        "non-atomic file replace, missing fsync, crash between two writes",
        &["atomic", "fsync", "crash"],
    ),
    m(
        "rs-conc",
        "lock order, missed wakeups, races across threads or processes",
        &["race", "deadlock", "lock order"],
    ),
    m(
        "rs-time",
        "clocks going backwards, timeouts, deadline arithmetic",
        &["clock", "timeout", "deadline"],
    ),
    m(
        "rs-limit",
        "empty, one, maximum and huge inputs; unbounded allocation",
        &["empty input", "boundary", "unbounded", "huge"],
    ),
    m(
        "rs-unsafe",
        "unsafe aliasing, lifetime and FFI assumptions",
        &["unsafe", "aliasing", "ffi"],
    ),
    m(
        "rs-spec",
        "the diff contradicts the claim text, a dependency claim or the intent",
        &["contradict", "spec", "intent"],
    ),
];

const fn m(
    id: &'static str,
    look_for: &'static str,
    keywords: &'static [&'static str],
) -> Mechanism {
    Mechanism {
        id,
        look_for,
        keywords,
    }
}

/// What [`eligible`] reads about one claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim<'a> {
    pub task: &'a str,
    pub id: MsgId,
    /// The current code candidate's diffs touch a `.rs` file.
    pub rust_candidate: bool,
    /// Every review lens has recorded its blind outcome and owed look-harder.
    pub reviews_settled: bool,
    /// #596's value score, when a policy provides one.
    pub value: Option<u64>,
    pub reversible: bool,
    /// The alien lens already hunted this candidate.
    pub hunted: bool,
}

/// The trigger's settings (design §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Trigger {
    /// Phase A's 80th percentile of `value`, once frozen. `None` until
    /// then: irreversible claims only, and no control sample.
    pub min_value: Option<u64>,
    /// One in this many claims below `min_value` is hunted as a control.
    pub control_one_in: u64,
}

/// Does this claim get a hunt now?
pub fn eligible(claim: &Claim, trigger: Trigger) -> bool {
    if !claim.rust_candidate || !claim.reviews_settled || claim.hunted {
        return false;
    }
    if !claim.reversible {
        return true;
    }
    let (Some(min), Some(value)) = (trigger.min_value, claim.value) else {
        return false;
    };
    value >= min || control(claim.task, claim.id, trigger.control_one_in)
}

/// The deterministic control sample: `sha256(task, claim) mod n == 0`.
/// `n == 0` samples nothing.
pub fn control(task: &str, claim: MsgId, n: u64) -> bool {
    n != 0 && residue(task, claim, n) == 0
}

/// `sha256(task "\n" claim)`, the whole digest as a big-endian integer,
/// mod `n` (non-zero). Reduced byte by byte in `u128`, so no overflow.
fn residue(task: &str, claim: MsgId, n: u64) -> u64 {
    let digest = Sha256::of(format!("{task}\n{}", claim.0).as_bytes());
    let n = u128::from(n);
    let r = digest
        .0
        .iter()
        .fold(0u128, |r, &b| (r * 256 + u128::from(b)) % n);
    u64::try_from(r).unwrap_or(u64::MAX) // r < n <= u64::MAX
}

/// Does this diff text touch a `.rs` file? Any `--- a/`, `+++ b/`,
/// `rename from`, `rename to` or `diff --git` path ending in `.rs` counts;
/// `/dev/null` does not (D87).
pub fn touches_rs(diff: &str) -> bool {
    diff.lines()
        .flat_map(header_paths)
        .any(|p| p.ends_with(".rs"))
}

/// The paths one diff header line names; none for any other line.
fn header_paths(line: &str) -> Vec<&str> {
    const ONE: [&str; 4] = ["--- a/", "+++ b/", "rename from ", "rename to "];
    if let Some(rest) = ONE.iter().find_map(|p| line.strip_prefix(p)) {
        return vec![rest.trim_end()];
    }
    let Some(rest) = line.strip_prefix("diff --git ") else {
        return Vec::new();
    };
    rest.split_whitespace()
        .filter_map(|p| p.strip_prefix("a/").or(p.strip_prefix("b/")))
        .collect()
}

/// One suspect from an enumerate reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suspect {
    /// A `module::fn` or a file path in the diff.
    pub site: String,
    pub mechanism: &'static str,
    pub sketch: String,
    /// The model's own 1..=5 rating.
    pub coherence: u8,
}

/// Parse an enumerate reply: `{"suspects": [{"site", "mechanism",
/// "sketch", "coherence"}]}`, at most [`MAX_SUSPECTS`]. A suspect naming a
/// mechanism outside `list` is dropped; any other fault refuses the reply.
pub fn parse_suspects(text: &str, list: &[Mechanism]) -> Result<Vec<Suspect>, String> {
    let v = Value::parse(text).map_err(|e| format!("reply is not JSON: {e}"))?;
    let top = only(&v, &["suspects"], "reply")?;
    let items = top
        .get("suspects")
        .and_then(Value::as_array)
        .ok_or("reply has no \"suspects\" array")?;
    if items.len() > MAX_SUSPECTS {
        return Err(format!("{} suspects exceeds {MAX_SUSPECTS}", items.len()));
    }
    let mut out = Vec::new();
    for (i, item) in items.iter().enumerate() {
        if let Some(s) = suspect(item, list).map_err(|e| format!("suspect {i}: {e}"))? {
            out.push(s);
        }
    }
    Ok(out)
}

fn suspect(v: &Value, list: &[Mechanism]) -> Result<Option<Suspect>, String> {
    let o = only(v, &["site", "mechanism", "sketch", "coherence"], "suspect")?;
    let site = field(o, "site")?;
    if !qualified(&site) {
        return Err(format!("site {site:?} is not a module::fn or a path"));
    }
    let coherence = o
        .get("coherence")
        .and_then(Value::as_u64)
        .filter(|c| (1..=5).contains(c))
        .ok_or("coherence: 1 to 5 required")? as u8;
    let sketch = field(o, "sketch")?;
    let named = field(o, "mechanism")?;
    let Some(mechanism) = list.iter().find(|m| m.id == named) else {
        return Ok(None);
    };
    Ok(Some(Suspect {
        site,
        mechanism: mechanism.id,
        sketch,
        coherence,
    }))
}

/// A site precise enough to match on: a path, or `module::fn`, never a
/// bare name like `new`.
fn qualified(site: &str) -> bool {
    !site.contains(char::is_whitespace) && (site.contains("::") || site.contains('/'))
}

/// Parse a hunt reply: `{"probe": null}` or `{"probe": "<test source>"}`.
pub fn parse_probe(text: &str) -> Result<Option<String>, String> {
    let v = Value::parse(text).map_err(|e| format!("reply is not JSON: {e}"))?;
    let top = only(&v, &["probe"], "reply")?;
    match top.get("probe") {
        Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.trim().is_empty() => Err("probe is empty".to_owned()),
        Some(Value::String(s)) if s.len() > MAX_PROBE => {
            Err(format!("probe is {} bytes, over {MAX_PROBE}", s.len()))
        }
        Some(Value::String(s)) => Ok(Some(s.clone())),
        _ => Err("probe: a string or null required".to_owned()),
    }
}

/// How far the available set is from a suspect (design §5): 2 when no
/// note names its site and none is tagged with its mechanism, 1 when one of
/// the two matches, 0 when both do.
pub fn unavailability(s: &Suspect, notes: &[String], list: &[Mechanism]) -> u8 {
    let lower: Vec<String> = notes.iter().map(|n| n.to_lowercase()).collect();
    let site = s.site.to_lowercase();
    let named = lower.iter().any(|n| names(n, &site));
    let keywords = list
        .iter()
        .find(|m| m.id == s.mechanism)
        .map_or(&[][..], |m| m.keywords);
    let tagged = lower.iter().any(|n| keywords.iter().any(|k| n.contains(k)));
    2 - u8::from(named) - u8::from(tagged)
}

/// Whether `note` names `site` whole. The note is split into site-like
/// tokens (letters, digits and `_ - / . :`); a token names the site when,
/// less trailing punctuation and a `:line[:col]` suffix, it is the site
/// itself or a path-qualified `…::site`. So `store::write` is named by
/// `crate::store::write` and `Store::Write,`, but not by `store::write_all`
/// or `store::write::inner`; `src/a.rs` by `src/a.rs:42`, but not by
/// `x/src/a.rs`, `integration-src/a.rs` or `src/a.rs.old.rs`.
fn names(note: &str, site: &str) -> bool {
    let token = |c: char| c.is_alphanumeric() || "_-/.:".contains(c);
    note.split(|c: char| !token(c))
        .map(bare)
        .any(|t| t == site || t.strip_suffix(site).is_some_and(|q| q.ends_with("::")))
}

/// A token less trailing `.`, `,` or `:` and any `:line[:col]` suffix.
fn bare(mut t: &str) -> &str {
    loop {
        let trimmed = t.trim_end_matches(['.', ',', ':']);
        let unlined = match trimmed.rsplit_once(':') {
            Some((head, n)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => head,
            _ => trimmed,
        };
        if unlined == t {
            return t;
        }
        t = unlined;
    }
}

/// The suspects to hunt: sorted by unavailability, then coherence (both
/// highest first), then mechanism and site; duplicates (same site and
/// mechanism) kept once; at most [`TOP_K`].
pub fn rank(suspects: Vec<Suspect>, notes: &[String], list: &[Mechanism]) -> Vec<Suspect> {
    let mut keyed: Vec<_> = suspects
        .into_iter()
        .map(|s| {
            (
                std::cmp::Reverse(unavailability(&s, notes, list)),
                std::cmp::Reverse(s.coherence),
                s,
            )
        })
        .collect();
    keyed.sort_by(|a, b| {
        (a.0, a.1, a.2.mechanism, &a.2.site).cmp(&(b.0, b.1, b.2.mechanism, &b.2.site))
    });
    let mut seen = BTreeSet::new();
    keyed
        .into_iter()
        .map(|(_, _, s)| s)
        .filter(|s| seen.insert((s.site.clone(), s.mechanism)))
        .take(TOP_K)
        .collect()
}

/// How one probe run ended (design §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Run {
    /// The probe did not build (`cargo test --no-run` failed).
    BuildError,
    Passed,
    /// It built and at least one test failed.
    Failed,
}

/// What a probe showed, from its run on the candidate and on `base`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Outcome {
    /// It did not build on the candidate: not a finding.
    Error,
    /// The candidate passes it.
    Pass,
    /// It fails on the candidate and on base: the defect predates the diff.
    PreExisting,
    /// It passes on base and fails on the candidate: the only finding.
    Regression,
    /// It fails on the candidate and does not build on base: new code.
    NewCodeFail,
}

impl Outcome {
    /// The record word.
    pub fn name(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Pass => "pass",
            Self::PreExisting => "pre-existing",
            Self::Regression => "regression",
            Self::NewCodeFail => "new-code-fail",
        }
    }

    /// Parse a record word.
    pub fn parse(word: &str) -> Option<Self> {
        [
            Self::Error,
            Self::Pass,
            Self::PreExisting,
            Self::Regression,
            Self::NewCodeFail,
        ]
        .into_iter()
        .find(|o| o.name() == word)
    }
}

/// Classify a probe. `base` may be `None` only when the candidate run made
/// base's irrelevant (it errored or passed); a candidate failure without a
/// base run is refused, since it could be any of three outcomes.
pub fn outcome(candidate: Run, base: Option<Run>) -> Result<Outcome, String> {
    match (candidate, base) {
        (Run::BuildError, _) => Ok(Outcome::Error),
        (Run::Passed, _) => Ok(Outcome::Pass),
        (Run::Failed, Some(Run::Passed)) => Ok(Outcome::Regression),
        (Run::Failed, Some(Run::Failed)) => Ok(Outcome::PreExisting),
        (Run::Failed, Some(Run::BuildError)) => Ok(Outcome::NewCodeFail),
        (Run::Failed, None) => Err("the candidate failed but base was not run".to_owned()),
    }
}

/// One completed hunt (design §3), saved whether or not it produced a
/// probe: an empty suspect list and an all-null hunt still count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HuntRow {
    pub task: String,
    pub claim: u64,
    /// The candidate artifact hunted.
    pub candidate: u64,
    /// Suspects the enumerate reply named, and probes the hunts wrote.
    pub suspects: u32,
    pub probes: u32,
    /// Model tokens over the hunt's calls.
    pub tokens: u64,
    /// What the hunt cost the lens's meter, in micro-dollars.
    pub cost: u64,
    /// When the hunt began, Unix milliseconds: orders the §8 cohort.
    pub started: u64,
}

/// One probe's record (design §6): what was hunted, what it showed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeRow {
    pub task: String,
    pub claim: u64,
    /// The candidate artifact the probe ran against.
    pub candidate: u64,
    pub site: String,
    pub mechanism: String,
    /// [`unavailability`] was 2: no ordinary evidence named the site or
    /// the mechanism.
    pub novel: bool,
    pub outcome: Outcome,
    /// Sandbox runs (candidate, then base when needed).
    pub runs: u32,
    /// SHA-256 of the probe file, hex.
    pub probe_sha256: String,
}

/// Hunts before the success bar is judged (design §8).
pub const BAR_HUNTS: usize = 20;
/// The alien lens's share of its claims' total spend must stay under this
/// percentage (design §8).
pub const BAR_SPEND_PERCENT: u64 = 15;

/// The shadow numbers over a cohort of hunts and their probes (design §8).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    /// Distinct (task, claim, candidate) over hunt and probe rows.
    pub hunts: usize,
    pub probes: usize,
    pub regressions: usize,
    /// Regressions no ordinary lens had named.
    pub novel: usize,
    pub pre_existing: usize,
    pub errors: usize,
    pub runs: u64,
    pub tokens: u64,
    /// The alien lens's own cost over the cohort, micro-dollars.
    pub cost: u64,
}

/// Total spend on the cohort's claims, every lens included, micro-dollars.
/// The runner does not track it yet, so the bar's spend half stays
/// unassessed until a caller supplies it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimSpend(pub u64);

/// Whether the shadow run has earned a v2 design.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bar {
    /// Fewer than [`BAR_HUNTS`] hunts so far.
    TooFew,
    /// Under one novel regression per [`BAR_HUNTS`] hunts.
    RateNotMet,
    /// The finding rate is met, but no claim spend is known (or it is
    /// zero), so the bar is not met: fail closed.
    SpendUnassessed,
    /// The finding rate is met, but the lens cost at least
    /// [`BAR_SPEND_PERCENT`]% of its claims' spend.
    SpendOver,
    /// Both halves met.
    Met,
}

/// Sum a cohort. A probe whose hunt row is missing still counts its hunt.
pub fn summarize(hunts: &[HuntRow], probes: &[ProbeRow]) -> Summary {
    let keys: BTreeSet<_> = hunts
        .iter()
        .map(|h| (&h.task, h.claim, h.candidate))
        .chain(probes.iter().map(|r| (&r.task, r.claim, r.candidate)))
        .collect();
    let count = |o: Outcome| probes.iter().filter(|r| r.outcome == o).count();
    Summary {
        hunts: keys.len(),
        probes: probes.len(),
        regressions: count(Outcome::Regression),
        novel: probes
            .iter()
            .filter(|r| r.novel && r.outcome == Outcome::Regression)
            .count(),
        pre_existing: count(Outcome::PreExisting),
        errors: count(Outcome::Error),
        runs: probes.iter().map(|r| u64::from(r.runs)).sum(),
        tokens: hunts.iter().map(|h| h.tokens).sum(),
        cost: hunts.iter().map(|h| h.cost).sum(),
    }
}

/// The §8 cohort: the first [`BAR_HUNTS`] hunt rows by start time (ties by
/// task, claim, candidate) and their probes. Later hunts never change it.
/// A probe without a hunt row has no start time, so it is not in it.
pub fn cohort(hunts: &[HuntRow], probes: &[ProbeRow]) -> Summary {
    let mut first: Vec<&HuntRow> = hunts.iter().collect();
    first.sort_by(|a, b| {
        (a.started, &a.task, a.claim, a.candidate).cmp(&(b.started, &b.task, b.claim, b.candidate))
    });
    first.truncate(BAR_HUNTS);
    let keys: BTreeSet<_> = first
        .iter()
        .map(|h| (&h.task, h.claim, h.candidate))
        .collect();
    let hunts: Vec<HuntRow> = first.into_iter().cloned().collect();
    let probes: Vec<ProbeRow> = probes
        .iter()
        .filter(|r| keys.contains(&(&r.task, r.claim, r.candidate)))
        .cloned()
        .collect();
    summarize(&hunts, &probes)
}

/// Judge the §8 bar over the [`cohort`] of `hunts` and `probes`.
pub fn judge(hunts: &[HuntRow], probes: &[ProbeRow], spend: Option<ClaimSpend>) -> Bar {
    cohort(hunts, probes).bar(spend)
}

impl Summary {
    /// The §8 bar on this summary taken as the cohort: at least
    /// [`BAR_HUNTS`] hunts, at least one novel regression per
    /// [`BAR_HUNTS`] of them, and the lens's cost under
    /// [`BAR_SPEND_PERCENT`]% of `spend`. Callers use [`judge`].
    fn bar(&self, spend: Option<ClaimSpend>) -> Bar {
        if self.hunts < BAR_HUNTS {
            return Bar::TooFew;
        }
        if self.novel.saturating_mul(BAR_HUNTS) < self.hunts {
            return Bar::RateNotMet;
        }
        let Some(ClaimSpend(total)) = spend.filter(|s| s.0 > 0) else {
            return Bar::SpendUnassessed;
        };
        if self.cost.saturating_mul(100) < total.saturating_mul(BAR_SPEND_PERCENT) {
            Bar::Met
        } else {
            Bar::SpendOver
        }
    }
}

fn only<'a>(v: &'a Value, keys: &[&str], what: &str) -> Result<&'a Map, String> {
    let o = v.as_object().ok_or(format!("{what} is not an object"))?;
    if let Some(k) = o.keys().find(|k| !keys.contains(&k.as_str())) {
        return Err(format!("{what}: unknown field {k:?}"));
    }
    Ok(o)
}

fn field(o: &Map, key: &str) -> Result<String, String> {
    let s = o
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or(format!("{key}: a non-empty string required"))?;
    if s.chars().count() > MAX_FIELD {
        return Err(format!("{key}: over {MAX_FIELD} characters"));
    }
    Ok(s.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_diff_touches_rs_by_its_header_paths_only() {
        let cases = [
            ("--- a/src/x.rs\n+++ b/src/x.rs\n", true),
            ("--- /dev/null\n+++ b/new.rs\n", true),
            ("--- a/gone.rs\n+++ /dev/null\n", true),
            (
                "diff --git a/old.rs b/new.rs\nrename from old.rs\nrename to new.rs\n",
                true,
            ),
            ("rename from a.txt\nrename to b.rs\n", true),
            ("--- a/README.md\n+++ b/README.md\n+see src/x.rs\n", false),
            ("--- a/x.rsx\n+++ b/x.rsx\n", false),
            ("--- /dev/null\n+++ /dev/null\n", false),
            ("", false),
        ];
        for (diff, want) in cases {
            assert_eq!(touches_rs(diff), want, "{diff:?}");
        }
    }

    fn claim() -> Claim<'static> {
        Claim {
            task: "t",
            id: MsgId(7),
            rust_candidate: true,
            reviews_settled: true,
            value: Some(50),
            reversible: true,
            hunted: false,
        }
    }

    const FROZEN: Trigger = Trigger {
        min_value: Some(40),
        control_one_in: 10,
    };
    const UNFROZEN: Trigger = Trigger {
        min_value: None,
        control_one_in: 10,
    };

    #[test]
    fn high_value_or_irreversible_claims_are_hunted_once_reviews_settle() {
        assert!(eligible(&claim(), FROZEN));
        assert!(eligible(
            &Claim {
                reversible: false,
                value: None,
                ..claim()
            },
            UNFROZEN
        ));
        for c in [
            Claim {
                rust_candidate: false,
                ..claim()
            },
            Claim {
                reviews_settled: false,
                ..claim()
            },
            Claim {
                hunted: true,
                ..claim()
            },
            Claim {
                reversible: false,
                hunted: true,
                ..claim()
            },
        ] {
            assert!(!eligible(&c, FROZEN), "{c:?}");
        }
    }

    #[test]
    fn before_phase_a_only_irreversible_claims_qualify() {
        assert!(!eligible(&claim(), UNFROZEN));
        assert!(!eligible(
            &Claim {
                value: None,
                ..claim()
            },
            FROZEN
        ));
    }

    #[test]
    fn below_the_threshold_only_the_control_sample_qualifies() {
        let low = |id| Claim {
            id: MsgId(id),
            value: Some(39),
            ..claim()
        };
        let hunted = (0..1000).filter(|&i| eligible(&low(i), FROZEN)).count();
        assert!((60..=140).contains(&hunted), "{hunted} of 1000");
        assert_eq!(
            (0..1000)
                .map(|i| eligible(&low(i), FROZEN))
                .collect::<Vec<_>>(),
            (0..1000)
                .map(|i| control("t", MsgId(i), 10))
                .collect::<Vec<_>>()
        );
        // Golden vectors, computed apart from this code: the whole
        // sha256("t\n<claim>") as an integer, mod 10.
        assert!(control("t", MsgId(0), 10)); // digest 5b1d…2c46, mod 10 == 0
        assert!(!control("t", MsgId(2), 10)); // its first 8 bytes alone give 0
                                              // Golden residues of sha256("t\n0"), computed in Python apart from
                                              // this code, including moduli near u64::MAX.
        for (n, r) in [
            (1, 0),
            (10, 0),
            (1_000_003, 531_540),
            (9_223_372_036_854_775_815, 7_131_217_979_231_520_140),
            (u64::MAX, 16_742_479_330_715_611_685),
        ] {
            assert_eq!(residue("t", MsgId(0), n), r, "mod {n}");
        }
        assert!(!control("t", MsgId(1), 0));
        assert!(control("t", MsgId(1), 1));
    }

    const REPLY: &str = r#"{"suspects": [
        {"site": "store::write_all", "mechanism": "rs-atomic", "sketch": "rename without fsync", "coherence": 4},
        {"site": "src/path.rs", "mechanism": "rs-made-up", "sketch": "x", "coherence": 5},
        {"site": "path::join", "mechanism": "rs-path", "sketch": "dot-dot escapes root", "coherence": 3}
    ]}"#;

    #[test]
    fn suspects_parse_and_unknown_mechanisms_drop() {
        let s = parse_suspects(REPLY, RUST).expect("valid");
        assert_eq!(
            s.iter()
                .map(|s| (s.site.as_str(), s.mechanism))
                .collect::<Vec<_>>(),
            [("store::write_all", "rs-atomic"), ("path::join", "rs-path")]
        );
    }

    #[test]
    fn malformed_suspects_refuse_the_reply() {
        let one = |body: &str| format!(r#"{{"suspects": [{body}]}}"#);
        for (reply, why) in [
            ("[]".to_owned(), "not an object"),
            (r#"{"suspects": [], "x": 1}"#.to_owned(), "unknown field"),
            (
                one(r#"{"site": "new", "mechanism": "rs-cast", "sketch": "s", "coherence": 3}"#),
                "not a module::fn",
            ),
            (
                one(r#"{"site": "a::b", "mechanism": "rs-cast", "sketch": "s", "coherence": 6}"#),
                "coherence",
            ),
            (
                one(r#"{"site": "a::b", "mechanism": "rs-cast", "sketch": " ", "coherence": 2}"#),
                "sketch",
            ),
            (
                one(
                    r#"{"site": "a::b", "mechanism": "rs-cast", "sketch": "s", "coherence": 2, "extra": 1}"#,
                ),
                "unknown field",
            ),
        ] {
            let e = parse_suspects(&reply, RUST).expect_err(why);
            assert!(e.contains(why), "{e} lacks {why}");
        }
        let many = format!(
            r#"{{"suspects": [{}]}}"#,
            vec![r#"{"site": "a::b", "mechanism": "rs-cast", "sketch": "s", "coherence": 2}"#; 13]
                .join(",")
        );
        assert!(parse_suspects(&many, RUST)
            .expect_err("cap")
            .contains("exceeds"));
    }

    #[test]
    fn probes_are_a_test_or_nothing() {
        assert_eq!(parse_probe(r#"{"probe": null}"#), Ok(None));
        assert_eq!(
            parse_probe(r##"{"probe": "#[test] fn f() {}"}"##),
            Ok(Some("#[test] fn f() {}".to_owned()))
        );
        assert!(parse_probe(r#"{"probe": "  "}"#).is_err());
        assert!(parse_probe(r#"{"probe": 3}"#).is_err());
        assert!(parse_probe(r#"{}"#).is_err());
        let big = format!(r#"{{"probe": "{}"}}"#, "x".repeat(MAX_PROBE + 1));
        assert!(parse_probe(&big).expect_err("big").contains("over"));
    }

    fn sus(site: &str, mechanism: &'static str, coherence: u8) -> Suspect {
        Suspect {
            site: site.to_owned(),
            mechanism,
            sketch: "s".to_owned(),
            coherence,
        }
    }

    #[test]
    fn unavailability_counts_site_and_mechanism_matches() {
        let notes = vec!["Store::Write_All loses data on crash".to_owned()];
        assert_eq!(
            unavailability(&sus("store::write_all", "rs-atomic", 3), &notes, RUST),
            0
        );
        assert_eq!(
            unavailability(&sus("store::write_all", "rs-cast", 3), &notes, RUST),
            1
        );
        assert_eq!(
            unavailability(&sus("path::join", "rs-atomic", 3), &notes, RUST),
            1
        );
        assert_eq!(
            unavailability(&sus("path::join", "rs-path", 3), &notes, RUST),
            2
        );
        assert_eq!(
            unavailability(&sus("path::join", "rs-path", 3), &[], RUST),
            2
        );
        for longer in ["store::write_all", "my_store::write", "store::write2"] {
            let note = vec![format!("{longer} is fine")];
            assert_eq!(
                unavailability(&sus("store::write", "rs-path", 3), &note, RUST),
                2,
                "{longer}"
            );
        }
        for (note, site, named) in [
            ("see Store::Write, line 3", "store::write", true),
            ("(crate::store::write)", "store::write", true),
            ("src/a.rs: fine.", "src/a.rs", true),
            ("src/a.rs:42 is fine", "src/a.rs", true),
            ("x/src/a.rs is fine", "src/a.rs", false),
            ("src/a.rs.old.rs is fine", "src/a.rs", false),
            ("src/a.rs/other.rs is fine", "src/a.rs", false),
            ("store::write::inner is fine", "store::write", false),
            ("integration-tests/foo.rs is fine", "tests/foo.rs", false),
            ("my-store::write is fine", "store::write", false),
            ("`src/a.rs:42:7` is fine", "src/a.rs", true),
            ("src/a.rs. Fine", "src/a.rs", true),
            ("src/a.rsx is fine", "src/a.rs", false),
        ] {
            let note = vec![note.to_owned()];
            assert_eq!(
                unavailability(&sus(site, "rs-cast", 3), &note, RUST),
                2 - u8::from(named),
                "{note:?} {site}"
            );
        }
    }

    #[test]
    fn rank_prefers_unavailable_then_coherent_and_breaks_ties_by_id() {
        let notes = vec!["path::join may overflow".to_owned()];
        let picked = rank(
            vec![
                sus("path::join", "rs-cast", 5), // 0: site and mechanism seen
                sus("a::x", "rs-time", 2),       // 2
                sus("a::y", "rs-err", 4),        // 2, more coherent
                sus("a::y", "rs-err", 4),        // duplicate
                sus("a::z", "rs-conc", 2),       // 2, ties a::x on coherence
                sus("path::join", "rs-path", 5), // 1
            ],
            &notes,
            RUST,
        );
        assert_eq!(
            picked
                .iter()
                .map(|s| (s.site.as_str(), s.mechanism))
                .collect::<Vec<_>>(),
            [("a::y", "rs-err"), ("a::z", "rs-conc"), ("a::x", "rs-time")]
        );
        assert!(rank(Vec::new(), &notes, RUST).is_empty());
    }

    #[test]
    fn outcomes_follow_the_candidate_then_base() {
        use Run::*;
        assert_eq!(outcome(BuildError, None), Ok(Outcome::Error));
        assert_eq!(outcome(BuildError, Some(Failed)), Ok(Outcome::Error));
        assert_eq!(outcome(Passed, None), Ok(Outcome::Pass));
        assert_eq!(outcome(Failed, Some(Passed)), Ok(Outcome::Regression));
        assert_eq!(outcome(Failed, Some(Failed)), Ok(Outcome::PreExisting));
        assert_eq!(outcome(Failed, Some(BuildError)), Ok(Outcome::NewCodeFail));
        assert!(outcome(Failed, None).is_err(), "base never ran");
    }

    #[test]
    fn outcome_words_round_trip() {
        for o in [
            Outcome::Error,
            Outcome::Pass,
            Outcome::PreExisting,
            Outcome::Regression,
            Outcome::NewCodeFail,
        ] {
            assert_eq!(Outcome::parse(o.name()), Some(o));
        }
        assert_eq!(Outcome::parse("found"), None);
    }

    fn row(claim: u64, novel: bool, outcome: Outcome) -> ProbeRow {
        ProbeRow {
            task: "t".to_owned(),
            claim,
            candidate: 1,
            site: format!("a::f{claim}"),
            mechanism: "rs-cast".to_owned(),
            novel,
            outcome,
            runs: 2,
            probe_sha256: "00".to_owned(),
        }
    }

    fn hunt(claim: u64, probes: u32) -> HuntRow {
        HuntRow {
            task: "t".to_owned(),
            claim,
            candidate: 1,
            suspects: 3,
            probes,
            tokens: 100,
            cost: 10,
            started: 1_000 + claim,
        }
    }

    #[test]
    fn summary_counts_each_hunt_once_with_or_without_probes() {
        let hunts = vec![hunt(1, 3), hunt(2, 0), hunt(3, 0)];
        let probes = vec![
            row(1, true, Outcome::Regression),
            row(1, false, Outcome::Regression),
            row(1, true, Outcome::PreExisting),
            row(4, true, Outcome::Error),
        ];
        assert_eq!(
            summarize(&hunts, &probes),
            Summary {
                hunts: 4,
                probes: 4,
                regressions: 2,
                novel: 1,
                pre_existing: 1,
                errors: 1,
                runs: 8,
                tokens: 300,
                cost: 30,
            }
        );
        assert_eq!(summarize(&[], &[]), Summary::default());
    }

    #[test]
    fn twenty_hunts_without_probes_are_judged() {
        let hunts: Vec<_> = (0..20).map(|c| hunt(c, 0)).collect();
        let s = summarize(&hunts, &[]);
        assert_eq!((s.hunts, s.probes), (20, 0));
        assert_eq!(s.bar(Some(ClaimSpend(1_000_000))), Bar::RateNotMet);
        assert_eq!(summarize(&hunts[..19], &[]).bar(None), Bar::TooFew);
    }

    #[test]
    fn the_rate_needs_one_novel_regression_per_twenty_hunts() {
        let mut hunts: Vec<_> = (0..20).map(|c| hunt(c, 0)).collect();
        let mut probes = vec![row(0, true, Outcome::Regression)];
        let spend = Some(ClaimSpend(1_000_000));
        assert_eq!(judge(&hunts, &probes, spend), Bar::Met);
        probes[0].novel = false;
        assert_eq!(judge(&hunts, &probes, spend), Bar::RateNotMet, "not novel");
        probes[0].novel = true;
        hunts.truncate(19);
        assert_eq!(judge(&hunts, &probes, spend), Bar::TooFew);
    }

    #[test]
    fn the_bar_is_frozen_to_the_first_twenty_hunts() {
        let spend = Some(ClaimSpend(1_000_000));
        let mut hunts: Vec<_> = (0..20).map(|c| hunt(c, 0)).collect();
        let found = vec![row(0, true, Outcome::Regression)];
        hunts.push(hunt(20, 0));
        assert_eq!(judge(&hunts, &found, spend), Bar::Met, "a 21st hunt");
        let c = cohort(&hunts, &found);
        assert_eq!((c.hunts, c.novel), (20, 1));
        assert_eq!(summarize(&hunts, &found).hunts, 21, "totals keep it");
        let late = vec![row(20, true, Outcome::Regression)];
        assert_eq!(judge(&hunts, &late, spend), Bar::RateNotMet, "too late");
        // Start time orders the cohort, not the claim id.
        hunts[0].started = 5_000;
        assert_eq!(judge(&hunts, &found, spend), Bar::RateNotMet, "hunt 0 last");
    }

    #[test]
    fn the_spend_half_is_strictly_under_fifteen_percent_and_fails_closed() {
        // 20 hunts at cost 10: the lens spent 200.
        let hunts: Vec<_> = (0..20).map(|c| hunt(c, 0)).collect();
        let s = summarize(&hunts, &[row(0, true, Outcome::Regression)]);
        assert_eq!(s.cost, 200);
        assert_eq!(s.bar(Some(ClaimSpend(1_334))), Bar::Met, "14.99%");
        assert_eq!(s.bar(Some(ClaimSpend(1_333))), Bar::SpendOver, "15.004%");
        let exactly = Summary {
            cost: 15,
            ..s.clone()
        };
        assert_eq!(
            exactly.bar(Some(ClaimSpend(100))),
            Bar::SpendOver,
            "exactly 15%"
        );
        assert_eq!(s.bar(None), Bar::SpendUnassessed);
        assert_eq!(s.bar(Some(ClaimSpend(0))), Bar::SpendUnassessed);
    }

    #[test]
    fn the_rust_list_has_unique_ids_and_lowercase_keywords() {
        let ids: BTreeSet<_> = RUST.iter().map(|m| m.id).collect();
        assert_eq!(ids.len(), RUST.len());
        assert!(RUST
            .iter()
            .flat_map(|m| m.keywords)
            .all(|k| *k == k.to_lowercase()));
    }
}
