//! Lineage entry types and the append-only hash chain (ADR-0005 §6,
//! invariant 4).
//!
//! Every graded candidate becomes one [`LineageEntry`]. The store adapter
//! serialises each entry to bytes and links it to its predecessor with
//! [`chain_hash`]; [`verify_chain`] then detects any edit, reordering or
//! deletion in the middle of the log. Large artefacts (diffs, solutions,
//! broker transcripts) are referenced by content address ([`BlobId`]).
//!
//! An entry stores task results, never an aggregated grade: the grade is
//! recomputed from results ([`EvaluationRecord::evaluation`]), so the two
//! cannot disagree. A [`LineageEntry`] can only be built by
//! [`LineageEntry::new`], which replays the accept gate on those results and
//! refuses an entry whose recorded decision does not follow from them.

use core::fmt;

use rusty_err::Error;
use rusty_rsa::{sha256, Digest, Sha256};

use crate::accept::{confirm, screen, Decision, Evaluation, Rejection, Screen};
use crate::budget::{Budget, CostUsage};
use crate::error::CoreError;
use crate::noise::Margin;
use crate::rng::Seed;
use crate::score::{Grade, Score};

/// The `prev` hash of the first entry in a chain.
pub const GENESIS: Digest = [0; 32];

/// `SHA-256(prev || payload)`: the hash that links one entry to the last.
#[must_use]
pub fn chain_hash(prev: &Digest, payload: &[u8]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(prev);
    hasher.update(payload);
    hasher.finish()
}

/// One stored entry as the chain sees it: its claimed links and its bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainRecord<'a> {
    /// The hash this entry claims for its predecessor.
    pub prev: Digest,
    /// The hash this entry claims for itself.
    pub hash: Digest,
    /// The serialised entry.
    pub payload: &'a [u8],
}

/// Where and how a hash chain is broken. Indices are zero-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ChainError {
    /// The entry's `prev` is not the previous entry's hash (or [`GENESIS`]).
    #[error("lineage entry {index}: prev hash does not match the preceding entry")]
    BrokenLink {
        /// The offending entry.
        index: usize,
    },
    /// The entry's `hash` does not match `prev` and its payload.
    #[error("lineage entry {index}: hash does not match its contents")]
    HashMismatch {
        /// The offending entry.
        index: usize,
    },
}

/// Verifies a whole chain from [`GENESIS`] and returns its head hash
/// ([`GENESIS`] for an empty chain).
///
/// # Errors
/// The first [`ChainError`] found, in order.
pub fn verify_chain<'a, I>(records: I) -> Result<Digest, ChainError>
where
    I: IntoIterator<Item = ChainRecord<'a>>,
{
    let mut head = GENESIS;
    for (index, record) in records.into_iter().enumerate() {
        if record.prev != head {
            return Err(ChainError::BrokenLink { index });
        }
        if chain_hash(&record.prev, record.payload) != record.hash {
            return Err(ChainError::HashMismatch { index });
        }
        head = record.hash;
    }
    Ok(head)
}

/// Lowercase hex encoding of a digest.
#[must_use]
pub fn to_hex(digest: &Digest) -> String {
    use fmt::Write as _;
    digest
        .iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            // Writing to a String cannot fail.
            let _ = write!(out, "{byte:02x}");
            out
        })
}

/// Parses 64 lowercase hex characters into a digest.
///
/// # Errors
/// [`CoreError::InvalidId`] for any other input.
pub fn from_hex(text: &str) -> Result<Digest, CoreError> {
    let invalid = || CoreError::InvalidId {
        kind: "sha256 digest",
        value: text.to_owned(),
    };
    if text.len() != 64 || !is_lower_hex(text) {
        return Err(invalid());
    }
    let bytes = text.as_bytes();
    let pairs = bytes.iter().step_by(2).zip(bytes.iter().skip(1).step_by(2));
    let mut digest = [0u8; 32];
    for (byte, (&high, &low)) in digest.iter_mut().zip(pairs) {
        *byte = (nibble(high) << 4) | nibble(low);
    }
    Ok(digest)
}

fn is_lower_hex(text: &str) -> bool {
    text.bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The value of one lowercase hex digit; callers check [`is_lower_hex`] first.
const fn nibble(digit: u8) -> u8 {
    match digit {
        b'0'..=b'9' => digit - b'0',
        _ => digit - b'a' + 10,
    }
}

/// The content address of a stored artefact: the SHA-256 of its bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlobId(Digest);

impl BlobId {
    /// The address `bytes` are stored under.
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        Self(sha256(bytes))
    }

    /// Parses a hex address.
    ///
    /// # Errors
    /// [`CoreError::InvalidId`] unless `text` is 64 lowercase hex characters.
    pub fn parse(text: &str) -> Result<Self, CoreError> {
        from_hex(text).map(Self)
    }

    /// The raw digest.
    #[must_use]
    pub const fn digest(&self) -> &Digest {
        &self.0
    }
}

impl fmt::Display for BlobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&to_hex(&self.0))
    }
}

/// A candidate's position in the run: `0` is the baseline `a0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CandidateId(u64);

impl CandidateId {
    /// The baseline agent.
    pub const BASELINE: Self = Self(0);

    /// Wraps a raw candidate number.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// The raw candidate number.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// A full git object id: 40 (SHA-1) or 64 (SHA-256) lowercase hex characters.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CommitSha(String);

impl CommitSha {
    /// Validates a commit id.
    ///
    /// # Errors
    /// [`CoreError::InvalidId`] for abbreviated, upper-case or non-hex ids.
    pub fn parse(text: &str) -> Result<Self, CoreError> {
        if matches!(text.len(), 40 | 64) && is_lower_hex(text) {
            return Ok(Self(text.to_owned()));
        }
        Err(CoreError::InvalidId {
            kind: "commit sha",
            value: text.to_owned(),
        })
    }

    /// The id as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A model identifier as the endpoint names it (e.g. `qwen2.5-coder:7b`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModelId(String);

impl ModelId {
    /// Validates a model id: non-empty, printable, no whitespace.
    ///
    /// # Errors
    /// [`CoreError::InvalidId`] otherwise.
    pub fn parse(text: &str) -> Result<Self, CoreError> {
        let valid = !text.is_empty() && text.chars().all(|c| !c.is_whitespace() && !c.is_control());
        if valid {
            return Ok(Self(text.to_owned()));
        }
        Err(CoreError::InvalidId {
            kind: "model id",
            value: text.to_owned(),
        })
    }

    /// The id as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A task name: lowercase ASCII letters, digits, `-` and `_`.
///
/// Task names become directory names, so path separators and `..` are
/// unrepresentable.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaskId(String);

impl TaskId {
    /// Validates a task name.
    ///
    /// # Errors
    /// [`CoreError::InvalidId`] for an empty name or any other character.
    pub fn parse(text: &str) -> Result<Self, CoreError> {
        let allowed =
            |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_';
        if !text.is_empty() && text.bytes().all(allowed) {
            return Ok(Self(text.to_owned()));
        }
        Err(CoreError::InvalidId {
            kind: "task id",
            value: text.to_owned(),
        })
    }

    /// The name as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One task run on one seed: what the harness submitted and how it scored.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskResult {
    /// The task.
    pub task: TaskId,
    /// The seed the inner run used.
    pub seed: Seed,
    /// The public score of the submitted solution, if it ran.
    pub public: Option<Score>,
    /// The private score (the task's floor when nothing was submitted).
    pub private: Score,
    /// The submitted solution, if any.
    pub solution: Option<BlobId>,
    /// The recorded broker transcript, for trajectory replay.
    pub transcript: Option<BlobId>,
    /// What the inner run consumed.
    pub cost: CostUsage,
}

/// All task results from one grading round of one candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct EvaluationRecord {
    /// `0` for the first grading, `1` for the fresh-seed re-evaluation.
    pub round: u32,
    /// One result per task and seed; a `(task, seed)` pair appears at most once.
    pub results: Vec<TaskResult>,
}

impl EvaluationRecord {
    /// Recomputes the round's grade and collects its distinct seeds.
    ///
    /// Tasks are grouped in order of first appearance. The same seed may be
    /// used by several tasks, but each `(task, seed)` run is counted once:
    /// a repeated row would otherwise weigh that run twice in the grade.
    ///
    /// # Errors
    /// [`CoreError::DuplicateResult`] if a `(task, seed)` pair repeats, even
    /// with identical contents; [`CoreError::EmptyGrade`] for a round with
    /// no results.
    pub fn evaluation(&self) -> Result<Evaluation, CoreError> {
        let mut tasks: Vec<(&str, Vec<(Seed, Score)>)> = Vec::new();
        let mut seeds: Vec<Seed> = Vec::new();
        for result in &self.results {
            let name = result.task.as_str();
            let run = (result.seed, result.private);
            match tasks.iter_mut().find(|(task, _)| *task == name) {
                Some((_, runs)) if runs.iter().any(|(seed, _)| *seed == result.seed) => {
                    return Err(CoreError::DuplicateResult {
                        task: name.to_owned(),
                        seed: result.seed.get(),
                    });
                }
                Some((_, runs)) => runs.push(run),
                None => tasks.push((name, vec![run])),
            }
            if !seeds.contains(&result.seed) {
                seeds.push(result.seed);
            }
        }
        let tasks: Vec<(&str, Vec<Score>)> = tasks
            .into_iter()
            .map(|(task, runs)| (task, runs.into_iter().map(|(_, score)| score).collect()))
            .collect();
        let grade = Grade::from_tasks(
            tasks
                .iter()
                .map(|(task, scores)| (*task, scores.as_slice())),
        )?;
        Evaluation::new(grade, seeds)
    }

    /// Total cost of the round.
    #[must_use]
    pub fn cost(&self) -> CostUsage {
        self.results
            .iter()
            .fold(CostUsage::default(), |total, r| total.plus(r.cost))
    }
}

/// The contents of a lineage entry, not yet checked against each other.
///
/// Turn it into a [`LineageEntry`] with [`LineageEntry::new`].
#[derive(Debug, Clone, PartialEq)]
pub struct EntryFields {
    /// This candidate.
    pub candidate: CandidateId,
    /// The candidate it was proposed from; `None` for the baseline.
    pub parent: Option<CandidateId>,
    /// The harness commit that was built.
    pub harness_commit: CommitSha,
    /// The proposed diff against the parent; `None` for the baseline.
    pub diff: Option<BlobId>,
    /// The inner agent's model.
    pub inner_model: ModelId,
    /// The outer agent's model; `None` for the baseline.
    pub outer_model: Option<ModelId>,
    /// The per-task budget every inner run was held to.
    pub budget: Budget,
    /// Grading rounds in order; empty when the candidate never built.
    pub evaluations: Vec<EvaluationRecord>,
    /// The accept gate's verdict.
    pub decision: Decision,
    /// The machine that produced the entry (wall-clock budgets depend on it).
    pub host: String,
}

/// Everything needed to reproduce one candidate's grade and verdict, with
/// the verdict proven to follow from the recorded evidence.
#[derive(Debug, Clone, PartialEq)]
pub struct LineageEntry(EntryFields);

impl LineageEntry {
    /// Validates `fields` and wraps them.
    ///
    /// - A gate verdict ([`Decision::Accepted`], [`Rejection::NotBetter`],
    ///   [`Rejection::WithinNoise`]) is re-derived by running [`screen`] and
    ///   [`confirm`] on the recorded evaluations with the recorded incumbent
    ///   grade and margin; it must equal the recorded decision exactly.
    /// - [`Decision::Baseline`] needs exactly one valid evaluation.
    /// - [`Rejection::Buggy`] and [`Rejection::PathViolation`] need no
    ///   evaluations (and must have none), plus a reason or at least one path.
    /// - Evaluation rounds must be numbered `0, 1, …` in order.
    ///
    /// # Errors
    /// [`CoreError::InconsistentEvidence`] when the evaluations have the
    /// wrong shape for the decision, [`CoreError::DecisionMismatch`] when the
    /// replayed decision differs, and any error from recomputing a grade
    /// ([`EvaluationRecord::evaluation`]) or from [`confirm`].
    pub fn new(fields: EntryFields) -> Result<Self, CoreError> {
        verify_decision(&fields.decision, &fields.evaluations)?;
        Ok(Self(fields))
    }

    /// The validated contents.
    #[must_use]
    pub const fn fields(&self) -> &EntryFields {
        &self.0
    }

    /// Gives the contents back, for example to serialise them.
    #[must_use]
    pub fn into_fields(self) -> EntryFields {
        self.0
    }
}

fn verify_decision(decision: &Decision, evaluations: &[EvaluationRecord]) -> Result<(), CoreError> {
    let in_order = evaluations
        .iter()
        .enumerate()
        .all(|(index, record)| u32::try_from(index).is_ok_and(|i| i == record.round));
    if !in_order {
        return Err(CoreError::InconsistentEvidence(
            "evaluation rounds must be numbered 0, 1, ... in order",
        ));
    }
    let (incumbent, margin) = match decision {
        Decision::Baseline => {
            let [only] = evaluations else {
                return Err(CoreError::InconsistentEvidence(
                    "a baseline needs exactly one evaluation",
                ));
            };
            return only.evaluation().map(|_| ());
        }
        Decision::Rejected(Rejection::Buggy { reason }) => {
            require_no_evaluations(evaluations)?;
            if reason.trim().is_empty() {
                return Err(CoreError::InconsistentEvidence(
                    "a buggy verdict needs a reason",
                ));
            }
            return Ok(());
        }
        Decision::Rejected(Rejection::PathViolation { paths }) => {
            require_no_evaluations(evaluations)?;
            if paths.is_empty() {
                return Err(CoreError::InconsistentEvidence(
                    "a path violation needs at least one path",
                ));
            }
            return Ok(());
        }
        Decision::Rejected(Rejection::NotBetter { incumbent, .. }) => (*incumbent, None),
        Decision::Accepted {
            incumbent, margin, ..
        }
        | Decision::Rejected(Rejection::WithinNoise {
            incumbent, margin, ..
        }) => (*incumbent, Some(*margin)),
    };
    if replay_gate(incumbent, margin, evaluations)? != *decision {
        return Err(CoreError::DecisionMismatch);
    }
    Ok(())
}

fn require_no_evaluations(evaluations: &[EvaluationRecord]) -> Result<(), CoreError> {
    if !evaluations.is_empty() {
        return Err(CoreError::InconsistentEvidence(
            "a candidate that was never graded has no evaluations",
        ));
    }
    Ok(())
}

/// Runs the accept gate on recorded evaluations, as the outer loop did.
fn replay_gate(
    incumbent: Grade,
    margin: Option<Margin>,
    evaluations: &[EvaluationRecord],
) -> Result<Decision, CoreError> {
    let [first, rest @ ..] = evaluations else {
        return Err(CoreError::InconsistentEvidence(
            "a gate verdict needs its first evaluation",
        ));
    };
    let challenger = match screen(incumbent, first.evaluation()?) {
        Screen::Reject(rejection) if rest.is_empty() => return Ok(Decision::Rejected(rejection)),
        Screen::Reject(_) => {
            return Err(CoreError::InconsistentEvidence(
                "a candidate rejected in stage 1 has no re-evaluation",
            ));
        }
        Screen::Reevaluate(challenger) => challenger,
    };
    // A recorded stage-1 rejection whose evidence actually passed stage 1.
    let Some(margin) = margin else {
        return Err(CoreError::DecisionMismatch);
    };
    let [fresh] = rest else {
        return Err(CoreError::InconsistentEvidence(
            "a candidate that passed stage 1 needs exactly one fresh re-evaluation",
        ));
    };
    confirm(incumbent, challenger, fresh.evaluation()?, margin)
}

#[cfg(test)]
mod tests {
    use core::time::Duration;

    use super::*;

    const SHA1: &str = "0123456789abcdef0123456789abcdef01234567";

    fn chain(payloads: &[&'static [u8]]) -> Vec<ChainRecord<'static>> {
        let mut prev = GENESIS;
        payloads
            .iter()
            .map(|payload| {
                let hash = chain_hash(&prev, payload);
                let record = ChainRecord {
                    prev,
                    hash,
                    payload,
                };
                prev = hash;
                record
            })
            .collect()
    }

    fn result(task: &str, seed: u64, private: f64) -> TaskResult {
        TaskResult {
            task: TaskId::parse(task).expect("valid task"),
            seed: Seed::new(seed),
            public: None,
            private: Score::new(private).expect("valid score"),
            solution: None,
            transcript: None,
            cost: CostUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
                ..CostUsage::default()
            },
        }
    }

    #[test]
    fn chain_hash_matches_sha256_of_concatenation() {
        let mut joined = GENESIS.to_vec();
        joined.extend_from_slice(b"entry");
        assert_eq!(chain_hash(&GENESIS, b"entry"), sha256(&joined));
    }

    #[test]
    fn valid_chain_verifies_to_its_head() {
        let records = chain(&[b"a", b"b", b"c"]);
        assert_eq!(verify_chain(records.iter().copied()), Ok(records[2].hash));
        assert_eq!(verify_chain([]), Ok(GENESIS));
    }

    #[test]
    fn edited_payload_is_detected() {
        let mut records = chain(&[b"a", b"b", b"c"]);
        records[1].payload = b"B";
        assert_eq!(
            verify_chain(records),
            Err(ChainError::HashMismatch { index: 1 })
        );
    }

    #[test]
    fn deleted_entry_is_detected() {
        let mut records = chain(&[b"a", b"b", b"c"]);
        records.remove(1);
        assert_eq!(
            verify_chain(records),
            Err(ChainError::BrokenLink { index: 1 })
        );
    }

    #[test]
    fn reordered_entries_are_detected() {
        let mut records = chain(&[b"a", b"b"]);
        records.swap(0, 1);
        assert_eq!(
            verify_chain(records),
            Err(ChainError::BrokenLink { index: 0 })
        );
    }

    #[test]
    fn rewritten_entry_with_recomputed_hash_breaks_the_next_link() {
        let mut records = chain(&[b"a", b"b", b"c"]);
        records[1].payload = b"B";
        records[1].hash = chain_hash(&records[1].prev, b"B");
        assert_eq!(
            verify_chain(records),
            Err(ChainError::BrokenLink { index: 2 })
        );
    }

    #[test]
    fn hex_round_trips() {
        let digest = sha256(b"lineage");
        let hex = to_hex(&digest);
        assert_eq!(hex.len(), 64);
        assert_eq!(from_hex(&hex), Ok(digest));
        assert_eq!(BlobId::parse(&hex).map(|id| id.to_string()), Ok(hex));
        assert_eq!(BlobId::of(b"lineage").digest(), &digest);
    }

    #[test]
    fn hex_rejects_bad_input() {
        let upper = to_hex(&sha256(b"x")).to_uppercase();
        for bad in ["", "ab", &upper, &"g".repeat(64), &"é".repeat(32)] {
            assert!(from_hex(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn id_validation() {
        assert!(CommitSha::parse(SHA1).is_ok());
        assert!(CommitSha::parse(&"a".repeat(64)).is_ok());
        assert!(CommitSha::parse("0123456").is_err(), "abbreviated");
        assert!(CommitSha::parse(&SHA1.to_uppercase()).is_err());

        assert!(ModelId::parse("qwen2.5-coder:7b").is_ok());
        for bad in ["", "two words", "tab\t", "nl\n"] {
            assert!(ModelId::parse(bad).is_err(), "{bad:?}");
        }

        assert!(TaskId::parse("tsp_small-1").is_ok());
        for bad in ["", "../private", "a/b", "Upper", "dot.name"] {
            assert!(TaskId::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn evaluation_record_recomputes_grade_per_task() {
        let record = EvaluationRecord {
            round: 0,
            results: vec![
                result("ml", 1, 1.0),
                result("tsp", 1, 0.5),
                result("ml", 2, 0.0),
                result("tsp", 2, 0.5),
            ],
        };
        let evaluation = record.evaluation().expect("non-empty");
        assert_eq!(evaluation.grade().get(), 0.5);
        assert_eq!(evaluation.seeds(), &[Seed::new(1), Seed::new(2)]);
        assert_eq!(record.cost().tokens(), 60);
    }

    #[test]
    fn empty_evaluation_record_has_no_grade() {
        let record = EvaluationRecord {
            round: 0,
            results: Vec::new(),
        };
        assert_eq!(record.evaluation(), Err(CoreError::EmptyGrade));
    }

    #[test]
    fn duplicate_task_seed_rows_are_rejected() {
        let duplicate = Err(CoreError::DuplicateResult {
            task: "ml".to_owned(),
            seed: 1,
        });
        let identical = round(0, &[("ml", 1, 0.4), ("tsp", 1, 0.5), ("ml", 1, 0.4)]);
        assert_eq!(identical.evaluation(), duplicate);
        let differing = round(0, &[("ml", 1, 0.0), ("ml", 1, 1.0)]);
        assert_eq!(differing.evaluation(), duplicate);
    }

    #[test]
    fn same_seed_across_tasks_is_allowed() {
        let record = round(0, &[("ml", 7, 1.0), ("tsp", 7, 0.0), ("ml", 8, 0.0)]);
        let evaluation = record.evaluation().expect("distinct (task, seed) pairs");
        assert_eq!(evaluation.grade().get(), 0.25, "ml 0.5 and tsp 0.0");
        assert_eq!(evaluation.seeds(), &[Seed::new(7), Seed::new(8)]);
    }

    fn round(number: u32, rows: &[(&str, u64, f64)]) -> EvaluationRecord {
        EvaluationRecord {
            round: number,
            results: rows
                .iter()
                .map(|&(task, seed, private)| result(task, seed, private))
                .collect(),
        }
    }

    fn grade(value: f64) -> Grade {
        Grade::new(value).expect("valid grade")
    }

    fn margin(value: f64) -> Margin {
        Margin::new(value).expect("valid margin")
    }

    fn fields(evaluations: Vec<EvaluationRecord>, decision: Decision) -> EntryFields {
        EntryFields {
            candidate: CandidateId::new(3),
            parent: Some(CandidateId::BASELINE),
            harness_commit: CommitSha::parse(SHA1).expect("valid sha"),
            diff: Some(BlobId::of(b"diff")),
            inner_model: ModelId::parse("llama3.1").expect("valid model"),
            outer_model: Some(ModelId::parse("gpt-5-codex").expect("valid model")),
            budget: Budget::new(1_000, Duration::from_secs(60), None).expect("valid budget"),
            evaluations,
            decision,
            host: "test-host".into(),
        }
    }

    /// Runs the gate the way the outer loop will, to record a genuine decision.
    fn decide(incumbent: Grade, margin: Margin, evaluations: &[EvaluationRecord]) -> Decision {
        let first = evaluations[0].evaluation().expect("valid first round");
        match screen(incumbent, first) {
            Screen::Reject(rejection) => Decision::Rejected(rejection),
            Screen::Reevaluate(challenger) => {
                let fresh = evaluations[1].evaluation().expect("valid fresh round");
                confirm(incumbent, challenger, fresh, margin).expect("fresh seeds")
            }
        }
    }

    /// First round grades 0.7 on seed 1; the fresh round grades 0.6 on seed 3.
    fn winning_rounds() -> Vec<EvaluationRecord> {
        vec![
            round(0, &[("ml", 1, 0.8), ("tsp", 1, 0.6)]),
            round(1, &[("ml", 3, 0.7), ("tsp", 3, 0.5)]),
        ]
    }

    fn accepted_entry() -> LineageEntry {
        let evaluations = winning_rounds();
        let decision = decide(grade(0.5), margin(0.05), &evaluations);
        assert!(matches!(decision, Decision::Accepted { .. }));
        LineageEntry::new(fields(evaluations, decision)).expect("decision follows from evidence")
    }

    #[test]
    fn genuine_gate_verdicts_are_accepted() {
        let entry = accepted_entry();
        assert!(entry.fields().decision.is_incumbent());

        let within_noise = decide(grade(0.5), margin(0.2), &winning_rounds());
        assert!(matches!(
            within_noise,
            Decision::Rejected(Rejection::WithinNoise { .. })
        ));
        assert!(LineageEntry::new(fields(winning_rounds(), within_noise)).is_ok());

        let first_only = vec![round(0, &[("ml", 1, 0.4)])];
        let not_better = decide(grade(0.5), margin(0.05), &first_only);
        assert!(matches!(
            not_better,
            Decision::Rejected(Rejection::NotBetter { .. })
        ));
        assert!(LineageEntry::new(fields(first_only, not_better)).is_ok());
    }

    #[test]
    fn gate_verdict_with_empty_evidence_is_rejected() {
        let decision = accepted_entry().into_fields().decision;
        assert!(matches!(
            LineageEntry::new(fields(Vec::new(), decision)),
            Err(CoreError::InconsistentEvidence(_))
        ));
        let empty_round = vec![round(0, &[])];
        let not_better = Decision::Rejected(Rejection::NotBetter {
            incumbent: grade(0.5),
            first: grade(0.4),
        });
        assert_eq!(
            LineageEntry::new(fields(empty_round, not_better)),
            Err(CoreError::EmptyGrade)
        );
    }

    #[test]
    fn decision_that_does_not_match_its_evidence_is_rejected() {
        let mismatch = Err(CoreError::DecisionMismatch);
        // An independent grade: evidence says 0.6, the record claims 0.9.
        let inflated = Decision::Accepted {
            incumbent: grade(0.5),
            fresh: grade(0.9),
            margin: margin(0.05),
        };
        assert_eq!(
            LineageEntry::new(fields(winning_rounds(), inflated)),
            mismatch
        );
        // Accepted, though the fresh lead of 0.1 is inside a 0.2 margin.
        let fresh = winning_rounds()[1].evaluation().expect("valid").grade();
        let lenient = Decision::Accepted {
            incumbent: grade(0.5),
            fresh,
            margin: margin(0.2),
        };
        assert_eq!(
            LineageEntry::new(fields(winning_rounds(), lenient)),
            mismatch
        );
        // A stage-1 rejection whose evidence actually beat the incumbent.
        let first = winning_rounds()[0].evaluation().expect("valid").grade();
        let not_better = Decision::Rejected(Rejection::NotBetter {
            incumbent: grade(0.5),
            first,
        });
        assert_eq!(
            LineageEntry::new(fields(winning_rounds()[..1].to_vec(), not_better)),
            mismatch
        );
        // Evidence from an unrelated, losing candidate under an Accepted verdict.
        let unrelated = vec![round(0, &[("ml", 1, 0.1)]), round(1, &[("ml", 3, 0.1)])];
        let decision = accepted_entry().into_fields().decision;
        assert!(matches!(
            LineageEntry::new(fields(unrelated, decision)),
            Err(CoreError::InconsistentEvidence(_))
        ));
    }

    #[test]
    fn changing_valid_evidence_invalidates_the_entry() {
        let valid = accepted_entry().into_fields();

        let mut rescored = valid.clone();
        rescored.evaluations[1].results[0].private = Score::new(0.2).expect("valid");
        assert_eq!(
            LineageEntry::new(rescored),
            Err(CoreError::DecisionMismatch)
        );

        let mut reseeded = valid.clone();
        for result in &mut reseeded.evaluations[1].results {
            result.seed = Seed::new(1);
        }
        assert_eq!(LineageEntry::new(reseeded), Err(CoreError::SeedReused(1)));

        let mut truncated = valid.clone();
        truncated.evaluations.pop();
        assert!(matches!(
            LineageEntry::new(truncated),
            Err(CoreError::InconsistentEvidence(_))
        ));

        let mut duplicated = valid.clone();
        let row = duplicated.evaluations[0].results[0].clone();
        duplicated.evaluations[0].results.push(row);
        assert!(matches!(
            LineageEntry::new(duplicated),
            Err(CoreError::DuplicateResult { .. })
        ));

        let mut renumbered = valid;
        renumbered.evaluations[1].round = 5;
        assert!(matches!(
            LineageEntry::new(renumbered),
            Err(CoreError::InconsistentEvidence(_))
        ));
    }

    #[test]
    fn ungraded_verdicts_need_no_evaluations() {
        let buggy = Decision::Rejected(Rejection::Buggy {
            reason: "rustc: E0308".into(),
        });
        let entry = LineageEntry::new(fields(Vec::new(), buggy.clone())).expect("valid buggy");
        assert!(!entry.fields().decision.is_incumbent());
        let paths = Decision::Rejected(Rejection::PathViolation {
            paths: vec!["Cargo.toml".into()],
        });
        assert!(LineageEntry::new(fields(Vec::new(), paths.clone())).is_ok());

        for decision in [buggy, paths] {
            assert!(matches!(
                LineageEntry::new(fields(winning_rounds(), decision)),
                Err(CoreError::InconsistentEvidence(_))
            ));
        }
        let no_reason = Decision::Rejected(Rejection::Buggy {
            reason: "  ".into(),
        });
        let no_paths = Decision::Rejected(Rejection::PathViolation { paths: Vec::new() });
        for decision in [no_reason, no_paths] {
            assert!(matches!(
                LineageEntry::new(fields(Vec::new(), decision)),
                Err(CoreError::InconsistentEvidence(_))
            ));
        }
    }

    #[test]
    fn baseline_needs_exactly_one_valid_evaluation() {
        let one = winning_rounds()[..1].to_vec();
        assert!(LineageEntry::new(fields(one, Decision::Baseline)).is_ok());
        for evaluations in [Vec::new(), winning_rounds()] {
            assert!(matches!(
                LineageEntry::new(fields(evaluations, Decision::Baseline)),
                Err(CoreError::InconsistentEvidence(_))
            ));
        }
        let duplicated = vec![round(0, &[("ml", 1, 0.5), ("ml", 1, 0.5)])];
        assert!(matches!(
            LineageEntry::new(fields(duplicated, Decision::Baseline)),
            Err(CoreError::DuplicateResult { .. })
        ));
    }
}
