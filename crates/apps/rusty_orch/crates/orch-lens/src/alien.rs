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
    if n == 0 {
        return false;
    }
    let digest = Sha256::of(format!("{task}\n{}", claim.0).as_bytes());
    let mut head = [0u8; 8];
    head.copy_from_slice(&digest.0[..8]);
    u64::from_be_bytes(head) % n == 0
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
    let named = lower.iter().any(|n| n.contains(&site));
    let keywords = list
        .iter()
        .find(|m| m.id == s.mechanism)
        .map_or(&[][..], |m| m.keywords);
    let tagged = lower.iter().any(|n| keywords.iter().any(|k| n.contains(k)));
    2 - u8::from(named) - u8::from(tagged)
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

/// Classify a probe. `base` is `None` when the candidate run made base's
/// irrelevant (it errored or passed).
pub fn outcome(candidate: Run, base: Option<Run>) -> Outcome {
    match (candidate, base) {
        (Run::BuildError, _) => Outcome::Error,
        (Run::Passed, _) => Outcome::Pass,
        (Run::Failed, Some(Run::Passed)) => Outcome::Regression,
        (Run::Failed, Some(Run::Failed)) => Outcome::PreExisting,
        (Run::Failed, Some(Run::BuildError) | None) => Outcome::NewCodeFail,
    }
}

/// One probe's record (design §6): what was hunted, what it showed, what
/// it cost.
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
    /// Sandbox runs debited to the lens, and model tokens spent.
    pub runs: u32,
    pub tokens: u64,
    /// SHA-256 of the probe file, hex.
    pub probe_sha256: String,
}

/// Hunts before the success bar is judged (design §8).
pub const BAR_HUNTS: usize = 20;

/// The shadow numbers over a set of probe rows (design §8).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    /// Distinct (task, claim, candidate).
    pub hunts: usize,
    pub probes: usize,
    pub regressions: usize,
    /// Regressions no ordinary lens had named.
    pub novel: usize,
    pub pre_existing: usize,
    pub errors: usize,
    pub runs: u64,
    pub tokens: u64,
}

/// Whether the shadow run has earned a v2 design.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bar {
    /// Fewer than [`BAR_HUNTS`] hunts so far.
    TooFew,
    /// At least one novel regression per [`BAR_HUNTS`] hunts.
    Met,
    NotMet,
}

/// Sum `rows`.
pub fn summarize(rows: &[ProbeRow]) -> Summary {
    let hunts: BTreeSet<_> = rows
        .iter()
        .map(|r| (&r.task, r.claim, r.candidate))
        .collect();
    let count = |o: Outcome| rows.iter().filter(|r| r.outcome == o).count();
    Summary {
        hunts: hunts.len(),
        probes: rows.len(),
        regressions: count(Outcome::Regression),
        novel: rows
            .iter()
            .filter(|r| r.novel && r.outcome == Outcome::Regression)
            .count(),
        pre_existing: count(Outcome::PreExisting),
        errors: count(Outcome::Error),
        runs: rows.iter().map(|r| u64::from(r.runs)).sum(),
        tokens: rows.iter().map(|r| r.tokens).sum(),
    }
}

impl Summary {
    /// The §8 bar: judged only after [`BAR_HUNTS`] hunts.
    pub fn bar(&self) -> Bar {
        if self.hunts < BAR_HUNTS {
            return Bar::TooFew;
        }
        if self.novel.saturating_mul(BAR_HUNTS) >= self.hunts {
            Bar::Met
        } else {
            Bar::NotMet
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
        assert_eq!(outcome(BuildError, None), Outcome::Error);
        assert_eq!(outcome(BuildError, Some(Failed)), Outcome::Error);
        assert_eq!(outcome(Passed, None), Outcome::Pass);
        assert_eq!(outcome(Failed, Some(Passed)), Outcome::Regression);
        assert_eq!(outcome(Failed, Some(Failed)), Outcome::PreExisting);
        assert_eq!(outcome(Failed, Some(BuildError)), Outcome::NewCodeFail);
        assert_eq!(outcome(Failed, None), Outcome::NewCodeFail);
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
            tokens: 100,
            probe_sha256: "00".to_owned(),
        }
    }

    #[test]
    fn summary_counts_hunts_once_and_only_novel_regressions() {
        let rows = vec![
            row(1, true, Outcome::Regression),
            row(1, false, Outcome::Regression),
            row(1, true, Outcome::PreExisting),
            row(2, true, Outcome::Error),
        ];
        let s = summarize(&rows);
        assert_eq!(
            s,
            Summary {
                hunts: 2,
                probes: 4,
                regressions: 2,
                novel: 1,
                pre_existing: 1,
                errors: 1,
                runs: 8,
                tokens: 400,
            }
        );
        assert_eq!(s.bar(), Bar::TooFew);
        assert_eq!(summarize(&[]), Summary::default());
    }

    #[test]
    fn the_bar_needs_one_novel_regression_per_twenty_hunts() {
        let mut rows: Vec<_> = (0..20).map(|c| row(c, true, Outcome::Pass)).collect();
        assert_eq!(summarize(&rows).bar(), Bar::NotMet);
        rows[0].outcome = Outcome::Regression;
        assert_eq!(summarize(&rows).bar(), Bar::Met);
        rows.push(row(20, true, Outcome::Pass));
        assert_eq!(summarize(&rows).bar(), Bar::NotMet, "1 in 21");
        rows[0].novel = false;
        assert_eq!(summarize(&rows[..20]).bar(), Bar::NotMet, "not novel");
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
