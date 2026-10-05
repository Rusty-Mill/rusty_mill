//! The run's append-only record (ADR-0005 §6, invariant 4).
//!
//! ```text
//! <run>/lineage.jsonl      one line per candidate, hash-chained
//! <run>/blobs/<sha256>     diffs, submitted solutions, broker transcripts
//! ```
//!
//! Each line is `{"hash":"<hex>","prev":"<hex>","entry":<entry>}`, where
//! `hash = SHA-256(prev || entry)` over the entry's exact bytes and `prev`
//! is the previous line's hash (zeros for the first). The fixed-width
//! prefix lets a reader recover those exact bytes without re-serialising.
//! The file is only ever opened for appending, and the store has no update
//! or delete; any edit, deletion or reordering breaks the chain, which
//! [`JsonlLineage::open`] and [`LineageStore::entries`] verify.
//!
//! Numbers that must round-trip exactly (scores, grades, margins and
//! 64-bit seeds) are written as decimal strings; durations as integer
//! nanoseconds.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rsi_core::lineage::{chain_hash, from_hex, to_hex, verify_chain, ChainRecord, Digest, GENESIS};
use rsi_core::{
    BlobId, Budget, CandidateId, CommitSha, CostUsage, Decision, EntryFields, EvaluationRecord,
    Grade, LineageEntry, LineageStore, Margin, ModelId, Rejection, Score, Seed, TaskId, TaskResult,
};
use rusty_json::Value;

use crate::error::RuntimeError;

/// The lineage file inside a run directory.
pub const LINEAGE_FILE: &str = "lineage.jsonl";

/// The blob directory inside a run directory.
pub const BLOB_DIR: &str = "blobs";

const PREFIX_HASH: &str = "{\"hash\":\"";
const PREFIX_PREV: &str = "\",\"prev\":\"";
const PREFIX_ENTRY: &str = "\",\"entry\":";
const PREFIX_LEN: usize = PREFIX_HASH.len() + 64 + PREFIX_PREV.len() + 64 + PREFIX_ENTRY.len();

fn bad(what: impl Into<String>) -> RuntimeError {
    RuntimeError::Lineage(what.into())
}

/// The lineage of one run: `lineage.jsonl` in the run directory.
#[derive(Debug)]
pub struct JsonlLineage {
    path: PathBuf,
    head: Digest,
}

impl JsonlLineage {
    /// Starts a new, empty lineage in `run_dir`.
    ///
    /// # Errors
    /// [`RuntimeError::Lineage`] if `run_dir` already has a lineage;
    /// [`RuntimeError::Io`] on file errors.
    pub fn create(run_dir: &Path) -> Result<Self, RuntimeError> {
        std::fs::create_dir_all(run_dir)
            .map_err(|e| RuntimeError::io(format!("creating {}", run_dir.display()), e))?;
        let path = run_dir.join(LINEAGE_FILE);
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::AlreadyExists => {
                    bad(format!("{} already exists", path.display()))
                }
                _ => RuntimeError::io("creating the lineage", e),
            })?;
        Ok(Self {
            path,
            head: GENESIS,
        })
    }

    /// Opens the existing lineage in `run_dir` and verifies it. A missing
    /// lineage is an error, never an empty run.
    ///
    /// # Errors
    /// [`RuntimeError::Lineage`] if there is no lineage, the chain is
    /// broken or an entry is invalid; [`RuntimeError::Io`] on file errors.
    pub fn open(run_dir: &Path) -> Result<Self, RuntimeError> {
        let mut store = Self {
            path: run_dir.join(LINEAGE_FILE),
            head: GENESIS,
        };
        let (_, head) = store.read()?;
        store.head = head;
        Ok(store)
    }

    fn read(&self) -> Result<(Vec<LineageEntry>, Digest), RuntimeError> {
        let text = std::fs::read_to_string(&self.path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => bad(format!("{} does not exist", self.path.display())),
            _ => RuntimeError::io("reading the lineage", e),
        })?;
        if !text.is_empty() && !text.ends_with('\n') {
            return Err(bad("the last lineage line is incomplete"));
        }
        let mut records = Vec::new();
        for (index, line) in text.lines().enumerate() {
            records.push(split_line(line).map_err(|e| bad(format!("line {}: {e}", index + 1)))?);
        }
        let head = verify_chain(records.iter().map(|(prev, hash, payload)| ChainRecord {
            prev: *prev,
            hash: *hash,
            payload: payload.as_bytes(),
        }))
        .map_err(|e| bad(format!("hash chain: {e}")))?;
        let entries = records
            .iter()
            .enumerate()
            .map(|(index, (_, _, payload))| {
                let json = Value::parse(payload).map_err(|e| bad(format!("entry {index}: {e}")))?;
                decode_entry(&json).map_err(|e| bad(format!("entry {index}: {e}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok((entries, head))
    }
}

/// Splits a line into its claimed `prev`, claimed `hash` and exact entry
/// bytes.
fn split_line(line: &str) -> Result<(Digest, Digest, &str), String> {
    let malformed = || "not a lineage line".to_owned();
    if line.len() <= PREFIX_LEN || !line.ends_with('}') || !line.starts_with(PREFIX_HASH) {
        return Err(malformed());
    }
    let hash_at = PREFIX_HASH.len();
    let prev_at = hash_at + 64 + PREFIX_PREV.len();
    if line.get(hash_at + 64..prev_at) != Some(PREFIX_PREV)
        || line.get(prev_at + 64..PREFIX_LEN) != Some(PREFIX_ENTRY)
    {
        return Err(malformed());
    }
    let digest = |at: usize| {
        line.get(at..at + 64)
            .ok_or_else(malformed)
            .and_then(|hex| from_hex(hex).map_err(|e| e.to_string()))
    };
    Ok((
        digest(prev_at)?,
        digest(hash_at)?,
        &line[PREFIX_LEN..line.len() - 1],
    ))
}

impl LineageStore for JsonlLineage {
    type Error = RuntimeError;

    fn append(&mut self, entry: &LineageEntry) -> Result<Digest, RuntimeError> {
        let payload = encode_entry(entry).to_json_string();
        let hash = chain_hash(&self.head, payload.as_bytes());
        let line = format!(
            "{PREFIX_HASH}{}{PREFIX_PREV}{}{PREFIX_ENTRY}{payload}}}\n",
            to_hex(&hash),
            to_hex(&self.head)
        );
        // O_APPEND: every write lands at the end, whatever else holds the
        // file open.
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| RuntimeError::io("opening the lineage", e))?;
        file.write_all(line.as_bytes())
            .and_then(|()| file.sync_data())
            .map_err(|e| RuntimeError::io("appending to the lineage", e))?;
        self.head = hash;
        Ok(hash)
    }

    fn entries(&self) -> Result<Vec<LineageEntry>, RuntimeError> {
        self.read().map(|(entries, _)| entries)
    }
}

/// Content-addressed blobs: each file is named by the SHA-256 of its bytes.
#[derive(Debug, Clone)]
pub struct Blobs {
    dir: PathBuf,
}

impl Blobs {
    /// The blob directory of `run_dir`.
    ///
    /// # Errors
    /// [`RuntimeError::Io`] if it cannot be created.
    pub fn open(run_dir: &Path) -> Result<Self, RuntimeError> {
        let dir = run_dir.join(BLOB_DIR);
        std::fs::create_dir_all(&dir).map_err(|e| RuntimeError::io("creating blobs/", e))?;
        Ok(Self { dir })
    }

    /// Stores `bytes` (once) and returns their address.
    ///
    /// # Errors
    /// [`RuntimeError::Io`] on write errors.
    pub fn put(&self, bytes: &[u8]) -> Result<BlobId, RuntimeError> {
        let id = BlobId::of(bytes);
        let path = self.dir.join(id.to_string());
        if path.exists() {
            return Ok(id);
        }
        // Written aside and renamed, so a blob is never seen half-written.
        let partial = self
            .dir
            .join(format!("{id}.partial-{}", std::process::id()));
        std::fs::write(&partial, bytes)
            .and_then(|()| std::fs::rename(&partial, &path))
            .map_err(|e| RuntimeError::io("writing a blob", e))?;
        Ok(id)
    }

    /// The bytes stored at `id`, checked against their address.
    ///
    /// # Errors
    /// [`RuntimeError::Lineage`] if the blob is missing or was altered.
    pub fn get(&self, id: &BlobId) -> Result<Vec<u8>, RuntimeError> {
        let bytes = std::fs::read(self.dir.join(id.to_string()))
            .map_err(|e| bad(format!("blob {id}: {e}")))?;
        if BlobId::of(&bytes) != *id {
            return Err(bad(format!("blob {id} does not match its address")));
        }
        Ok(bytes)
    }
}

// --- Encoding -------------------------------------------------------------

fn nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

fn exact(value: f64) -> Value {
    Value::from(value.to_string())
}

fn opt<T>(value: Option<T>, f: impl FnOnce(T) -> Value) -> Value {
    value.map_or(Value::Null, f)
}

fn object(fields: Vec<(&str, Value)>) -> Value {
    let mut out = Value::object();
    for (key, value) in fields {
        out.insert(key, value);
    }
    out
}

fn encode_cost(cost: &CostUsage) -> Value {
    object(vec![
        ("prompt_tokens", Value::from(cost.prompt_tokens)),
        ("completion_tokens", Value::from(cost.completion_tokens)),
        ("wall_ns", Value::from(nanos(cost.wall))),
        ("gpu_ns", opt(cost.gpu, |g| Value::from(nanos(g)))),
    ])
}

fn encode_result(result: &TaskResult) -> Value {
    object(vec![
        ("task", Value::from(result.task.as_str())),
        ("seed", Value::from(result.seed.get().to_string())),
        ("public", opt(result.public, |s| exact(s.get()))),
        ("private", exact(result.private.get())),
        (
            "solution",
            opt(result.solution.as_ref(), |b| Value::from(b.to_string())),
        ),
        (
            "transcript",
            opt(result.transcript.as_ref(), |b| Value::from(b.to_string())),
        ),
        ("cost", encode_cost(&result.cost)),
    ])
}

fn encode_decision(decision: &Decision) -> Value {
    let grade = |g: &Grade| exact(g.get());
    match decision {
        Decision::Baseline => object(vec![("kind", "baseline".into())]),
        Decision::Accepted {
            incumbent,
            fresh,
            margin,
        } => object(vec![
            ("kind", "accepted".into()),
            ("incumbent", grade(incumbent)),
            ("fresh", grade(fresh)),
            ("margin", exact(margin.get())),
        ]),
        Decision::Rejected(Rejection::NotBetter { incumbent, first }) => object(vec![
            ("kind", "not_better".into()),
            ("incumbent", grade(incumbent)),
            ("first", grade(first)),
        ]),
        Decision::Rejected(Rejection::WithinNoise {
            incumbent,
            fresh,
            margin,
        }) => object(vec![
            ("kind", "within_noise".into()),
            ("incumbent", grade(incumbent)),
            ("fresh", grade(fresh)),
            ("margin", exact(margin.get())),
        ]),
        Decision::Rejected(Rejection::Buggy { reason }) => object(vec![
            ("kind", "buggy".into()),
            ("reason", reason.as_str().into()),
        ]),
        Decision::Rejected(Rejection::PathViolation { paths }) => object(vec![
            ("kind", "path_violation".into()),
            (
                "paths",
                Value::from(
                    paths
                        .iter()
                        .map(|p| Value::from(p.as_str()))
                        .collect::<Vec<_>>(),
                ),
            ),
        ]),
    }
}

/// The JSON form of an entry.
#[must_use]
pub fn encode_entry(entry: &LineageEntry) -> Value {
    let f = entry.fields();
    let budget = object(vec![
        ("tokens", Value::from(f.budget.tokens())),
        ("wall_ns", Value::from(nanos(f.budget.wall()))),
        ("gpu_ns", opt(f.budget.gpu(), |g| Value::from(nanos(g)))),
    ]);
    let evaluations = f
        .evaluations
        .iter()
        .map(|round| {
            object(vec![
                ("round", Value::from(u64::from(round.round))),
                (
                    "results",
                    Value::from(round.results.iter().map(encode_result).collect::<Vec<_>>()),
                ),
            ])
        })
        .collect::<Vec<_>>();
    object(vec![
        ("candidate", Value::from(f.candidate.get())),
        ("parent", opt(f.parent, |p| Value::from(p.get()))),
        ("harness_commit", Value::from(f.harness_commit.as_str())),
        ("diff", opt(f.diff.as_ref(), |b| Value::from(b.to_string()))),
        ("inner_model", Value::from(f.inner_model.as_str())),
        (
            "outer_model",
            opt(f.outer_model.as_ref(), |m| Value::from(m.as_str())),
        ),
        ("outer_cost", opt(f.outer_cost.as_ref(), encode_cost)),
        ("budget", budget),
        ("evaluations", Value::from(evaluations)),
        ("decision", encode_decision(&f.decision)),
        ("host", Value::from(f.host.as_str())),
    ])
}

// --- Decoding -------------------------------------------------------------

fn field<'a>(json: &'a Value, key: &str) -> Result<&'a Value, String> {
    json.get(key).ok_or_else(|| format!("`{key}` is missing"))
}

fn string<'a>(json: &'a Value, key: &str) -> Result<&'a str, String> {
    field(json, key)?
        .as_str()
        .ok_or_else(|| format!("`{key}` must be a string"))
}

fn number(json: &Value, key: &str) -> Result<u64, String> {
    field(json, key)?
        .as_u64()
        .ok_or_else(|| format!("`{key}` must be a whole number"))
}

fn nullable<'a, T>(
    json: &'a Value,
    key: &str,
    f: impl FnOnce(&'a Value) -> Result<T, String>,
) -> Result<Option<T>, String> {
    match field(json, key)? {
        Value::Null => Ok(None),
        value => f(value).map(Some),
    }
}

/// Like [`nullable`], but a missing key is `None` too: for fields added
/// after lineages were first written.
fn optional<'a, T>(
    json: &'a Value,
    key: &str,
    f: impl FnOnce(&'a Value) -> Result<T, String>,
) -> Result<Option<T>, String> {
    match json.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => f(value).map(Some),
    }
}

fn float(text: &str) -> Result<f64, String> {
    text.parse::<f64>()
        .map_err(|_| format!("`{text}` is not a number"))
}

fn float_at(json: &Value, key: &str) -> Result<f64, String> {
    float(string(json, key)?)
}

fn as_str(value: &Value) -> Result<&str, String> {
    value.as_str().ok_or_else(|| "expected a string".to_owned())
}

fn blob(value: &Value) -> Result<BlobId, String> {
    BlobId::parse(as_str(value)?).map_err(|e| e.to_string())
}

fn duration(json: &Value, key: &str) -> Result<Duration, String> {
    number(json, key).map(Duration::from_nanos)
}

fn decode_cost(json: &Value) -> Result<CostUsage, String> {
    Ok(CostUsage {
        prompt_tokens: number(json, "prompt_tokens")?,
        completion_tokens: number(json, "completion_tokens")?,
        wall: duration(json, "wall_ns")?,
        gpu: nullable(json, "gpu_ns", |v| {
            v.as_u64()
                .map(Duration::from_nanos)
                .ok_or_else(|| "`gpu_ns` must be a whole number".to_owned())
        })?,
    })
}

fn decode_result(json: &Value) -> Result<TaskResult, String> {
    let score = |value: &Value| Score::new(float(as_str(value)?)?).map_err(|e| e.to_string());
    Ok(TaskResult {
        task: TaskId::parse(string(json, "task")?).map_err(|e| e.to_string())?,
        seed: Seed::new(
            string(json, "seed")?
                .parse()
                .map_err(|_| "`seed` must be a 64-bit number".to_owned())?,
        ),
        public: nullable(json, "public", score)?,
        private: score(field(json, "private")?)?,
        solution: nullable(json, "solution", blob)?,
        transcript: nullable(json, "transcript", blob)?,
        cost: decode_cost(field(json, "cost")?)?,
    })
}

fn decode_decision(json: &Value) -> Result<Decision, String> {
    let grade = |key: &str| Grade::new(float_at(json, key)?).map_err(|e| e.to_string());
    let margin = || Margin::new(float_at(json, "margin")?).map_err(|e| e.to_string());
    Ok(match string(json, "kind")? {
        "baseline" => Decision::Baseline,
        "accepted" => Decision::Accepted {
            incumbent: grade("incumbent")?,
            fresh: grade("fresh")?,
            margin: margin()?,
        },
        "not_better" => Decision::Rejected(Rejection::NotBetter {
            incumbent: grade("incumbent")?,
            first: grade("first")?,
        }),
        "within_noise" => Decision::Rejected(Rejection::WithinNoise {
            incumbent: grade("incumbent")?,
            fresh: grade("fresh")?,
            margin: margin()?,
        }),
        "buggy" => Decision::Rejected(Rejection::Buggy {
            reason: string(json, "reason")?.to_owned(),
        }),
        "path_violation" => Decision::Rejected(Rejection::PathViolation {
            paths: field(json, "paths")?
                .as_array()
                .ok_or("`paths` must be an array")?
                .iter()
                .map(|p| as_str(p).map(str::to_owned))
                .collect::<Result<_, _>>()?,
        }),
        other => return Err(format!("unknown decision `{other}`")),
    })
}

/// Rebuilds an entry from its JSON form, re-validating it with
/// [`LineageEntry::new`] (so a recorded decision must still follow from
/// its recorded evidence).
///
/// # Errors
/// A message naming the malformed or inconsistent part.
pub fn decode_entry(json: &Value) -> Result<LineageEntry, String> {
    let budget_json = field(json, "budget")?;
    let budget = Budget::new(
        number(budget_json, "tokens")?,
        duration(budget_json, "wall_ns")?,
        nullable(budget_json, "gpu_ns", |v| {
            v.as_u64()
                .map(Duration::from_nanos)
                .ok_or_else(|| "`gpu_ns` must be a whole number".to_owned())
        })?,
    )
    .map_err(|e| e.to_string())?;
    let evaluations = field(json, "evaluations")?
        .as_array()
        .ok_or("`evaluations` must be an array")?
        .iter()
        .map(|round| {
            Ok(EvaluationRecord {
                round: u32::try_from(number(round, "round")?)
                    .map_err(|_| "`round` is too large".to_owned())?,
                results: field(round, "results")?
                    .as_array()
                    .ok_or("`results` must be an array")?
                    .iter()
                    .map(decode_result)
                    .collect::<Result<_, String>>()?,
            })
        })
        .collect::<Result<_, String>>()?;
    let model = |text: &str| ModelId::parse(text).map_err(|e| e.to_string());
    LineageEntry::new(EntryFields {
        candidate: CandidateId::new(number(json, "candidate")?),
        parent: nullable(json, "parent", |v| {
            v.as_u64()
                .map(CandidateId::new)
                .ok_or_else(|| "`parent` must be a whole number".to_owned())
        })?,
        harness_commit: CommitSha::parse(string(json, "harness_commit")?)
            .map_err(|e| e.to_string())?,
        diff: nullable(json, "diff", blob)?,
        inner_model: model(string(json, "inner_model")?)?,
        outer_model: nullable(json, "outer_model", |v| model(as_str(v)?))?,
        outer_cost: optional(json, "outer_cost", decode_cost)?,
        budget,
        evaluations,
        decision: decode_decision(field(json, "decision")?)?,
        host: string(json, "host")?.to_owned(),
    })
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("rsi-lineage-{name}-{}", std::process::id()));
            if dir.exists() {
                std::fs::remove_dir_all(&dir).expect("clear");
            }
            Self(dir)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            if !std::thread::panicking() {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    fn result(task: &str, seed: u64, private: f64, public: Option<f64>) -> TaskResult {
        TaskResult {
            task: TaskId::parse(task).expect("task"),
            seed: Seed::new(seed),
            public: public.map(|p| Score::new(p).expect("score")),
            private: Score::new(private).expect("score"),
            solution: Some(BlobId::of(b"print(1)")),
            transcript: None,
            cost: CostUsage {
                prompt_tokens: 7,
                completion_tokens: 3,
                wall: Duration::from_nanos(1_234_567_891),
                gpu: None,
            },
        }
    }

    fn entry(candidate: u64, decision: Decision, rounds: Vec<Vec<TaskResult>>) -> LineageEntry {
        LineageEntry::new(EntryFields {
            candidate: CandidateId::new(candidate),
            parent: candidate.checked_sub(1).map(CandidateId::new),
            harness_commit: CommitSha::parse(&"b".repeat(40)).expect("sha"),
            diff: Some(BlobId::of(b"diff")),
            inner_model: ModelId::parse("qwen2.5-coder:7b").expect("model"),
            outer_model: Some(ModelId::parse("outer").expect("model")),
            outer_cost: (candidate > 0).then(|| CostUsage {
                prompt_tokens: 4_000 + candidate,
                completion_tokens: 300,
                wall: Duration::from_millis(2_500),
                gpu: None,
            }),
            budget: Budget::new(1000, Duration::from_secs(60), None).expect("budget"),
            evaluations: rounds
                .into_iter()
                .enumerate()
                .map(|(round, results)| EvaluationRecord {
                    round: round as u32,
                    results,
                })
                .collect(),
            decision,
            host: "linux/x86_64".into(),
        })
        .expect("valid")
    }

    fn grade(v: f64) -> Grade {
        Grade::new(v).expect("grade")
    }

    /// One entry of every decision kind, with awkward floats and seeds.
    fn entries() -> Vec<LineageEntry> {
        let third = 1.0 / 3.0;
        let big_seed = u64::MAX - 1;
        let margin = Margin::new(0.1 + 0.2).expect("margin");
        vec![
            entry(
                0,
                Decision::Baseline,
                vec![vec![result("tsp", big_seed, third, Some(0.2))]],
            ),
            entry(
                1,
                Decision::Rejected(Rejection::NotBetter {
                    incumbent: grade(third),
                    first: grade(0.25),
                }),
                vec![vec![result("tsp", 5, 0.25, None)]],
            ),
            entry(
                2,
                Decision::Rejected(Rejection::WithinNoise {
                    incumbent: grade(third),
                    fresh: grade(0.5),
                    margin,
                }),
                vec![
                    vec![result("tsp", 6, 0.75, None)],
                    vec![result("tsp", 7, 0.5, None)],
                ],
            ),
            entry(
                3,
                Decision::Accepted {
                    incumbent: grade(third),
                    fresh: grade(0.875),
                    margin,
                },
                vec![
                    vec![result("tsp", 8, 0.75, None)],
                    vec![result("tsp", 9, 0.875, None)],
                ],
            ),
            entry(
                4,
                Decision::Rejected(Rejection::Buggy {
                    reason: "error[E0308]: \"mismatched\"\nline two".into(),
                }),
                vec![],
            ),
            entry(
                5,
                Decision::Rejected(Rejection::PathViolation {
                    paths: vec!["Cargo.toml".into(), "a/\u{e9}".into()],
                }),
                vec![],
            ),
        ]
    }

    #[test]
    fn every_entry_kind_round_trips_exactly() {
        for original in entries() {
            let json = encode_entry(&original);
            let text = json.to_json_string();
            let decoded = decode_entry(&Value::parse(&text).expect("json")).expect("decodes");
            assert_eq!(decoded, original);
            assert_eq!(
                encode_entry(&decoded).to_json_string(),
                text,
                "stable bytes"
            );
        }
    }

    #[test]
    fn an_entry_written_before_outer_cost_decodes_without_one() {
        let proposal = &entries()[1];
        let old = LineageEntry::new(EntryFields {
            outer_cost: None,
            ..proposal.fields().clone()
        })
        .expect("valid");
        let text = encode_entry(&old).to_json_string();
        let legacy = text.replace(",\"outer_cost\":null", "");
        assert_ne!(legacy, text, "the key was present to remove");
        let decoded = decode_entry(&Value::parse(&legacy).expect("json")).expect("decodes");
        assert_eq!(decoded, old);
    }

    #[test]
    fn decoding_revalidates_the_decision() {
        let accepted = &entries()[3];
        let text = encode_entry(accepted)
            .to_json_string()
            .replace("\"0.875\"", "\"0.375\"");
        let error = decode_entry(&Value::parse(&text).expect("json")).expect_err("forged");
        assert!(error.contains("does not follow"), "{error}");
    }

    #[test]
    fn appends_and_verifies_the_chain() {
        let dir = Dir::new("chain");
        assert!(
            matches!(JsonlLineage::open(&dir.0), Err(RuntimeError::Lineage(_))),
            "a missing lineage is not an empty one"
        );
        let mut store = JsonlLineage::create(&dir.0).expect("create");
        assert!(store.entries().expect("empty").is_empty());
        assert!(
            matches!(JsonlLineage::create(&dir.0), Err(RuntimeError::Lineage(_))),
            "never started twice"
        );
        let originals = entries();
        for e in &originals {
            store.append(e).expect("append");
        }
        assert_eq!(store.entries().expect("read"), originals);
        let reopened = JsonlLineage::open(&dir.0).expect("reopen");
        assert_eq!(reopened.head, store.head);
        assert_eq!(reopened.entries().expect("read"), originals);
    }

    fn tampered(dir: &Dir, edit: impl FnOnce(&mut Vec<String>)) -> RuntimeError {
        let path = dir.0.join(LINEAGE_FILE);
        let text = std::fs::read_to_string(&path).expect("read");
        let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
        edit(&mut lines);
        std::fs::write(&path, lines.join("\n") + "\n").expect("write");
        let error = JsonlLineage::open(&dir.0).expect_err("detected");
        std::fs::write(&path, text).expect("restore");
        error
    }

    #[test]
    fn edits_deletions_reordering_and_truncation_are_detected() {
        let dir = Dir::new("tamper");
        let mut store = JsonlLineage::create(&dir.0).expect("create");
        for e in &entries() {
            store.append(e).expect("append");
        }
        // Edit an entry's bytes without fixing the hashes.
        let edited = tampered(&dir, |lines| {
            lines[1] = lines[1].replace("\"host\":\"linux", "\"host\":\"LINUX")
        });
        assert!(edited.to_string().contains("hash chain"), "{edited}");
        // Delete a middle line.
        let deleted = tampered(&dir, |lines| {
            lines.remove(2);
        });
        assert!(deleted.to_string().contains("hash chain"), "{deleted}");
        // Swap two lines.
        let swapped = tampered(&dir, |lines| lines.swap(1, 2));
        assert!(swapped.to_string().contains("hash chain"), "{swapped}");
        // A torn final write.
        let path = dir.0.join(LINEAGE_FILE);
        let text = std::fs::read_to_string(&path).expect("read");
        std::fs::write(&path, &text[..text.len() - 10]).expect("truncate");
        assert!(JsonlLineage::open(&dir.0).is_err());
        std::fs::write(&path, &text).expect("restore");
        assert_eq!(
            JsonlLineage::open(&dir.0)
                .expect("intact")
                .entries()
                .expect("read")
                .len(),
            6
        );
    }

    #[test]
    fn blobs_are_content_addressed_and_checked() {
        let dir = Dir::new("blobs");
        let blobs = Blobs::open(&dir.0).expect("open");
        let id = blobs.put(b"print(42)\n").expect("put");
        assert_eq!(blobs.put(b"print(42)\n").expect("again"), id);
        assert_eq!(blobs.get(&id).expect("get"), b"print(42)\n");
        std::fs::write(dir.0.join(BLOB_DIR).join(id.to_string()), b"print(43)\n").expect("tamper");
        assert!(matches!(blobs.get(&id), Err(RuntimeError::Lineage(_))));
        assert!(blobs.get(&BlobId::of(b"absent")).is_err());
    }
}
