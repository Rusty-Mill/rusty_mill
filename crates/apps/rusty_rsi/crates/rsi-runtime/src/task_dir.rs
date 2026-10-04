//! Task directories: the on-disk task format (ADR-0005 §4).
//!
//! ```text
//! <task>/task.json        manifest (see TaskManifest)
//! <task>/public/          readable by the inner loop
//!     <shared files>      e.g. train.csv, staged for every run
//!     <inputs>            public evaluation inputs
//!     labels.txt          public evaluation labels (public score)
//!     baseline.py         the starting solution x0
//! <task>/private/         never staged, never reachable from a sandbox
//!     <inputs>            held-out inputs, staged only for private grading
//!     labels.txt          held-out labels, read only by the grader process
//! ```
//!
//! A solution runs as `python3 solution.py data/<inputs> output.txt` in a
//! fresh working directory holding `solution.py` and `data/` (the shared
//! files plus one split's inputs), with `RSI_SEED` in its environment.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rsi_core::{Limits, Score, Solution, TaskId};
use rusty_json::Value;

use crate::error::RuntimeError;
use crate::metric::Metric;

/// The file a solution must write its answers to.
pub const OUTPUT_FILE: &str = "output.txt";

/// Open descriptors a solution may hold.
pub const OPEN_FILE_LIMIT: u64 = 64;

/// The process limit given to every solution.
///
/// Linux counts `RLIMIT_NPROC` against *every* process and thread of the
/// user, not just the sandboxed tree, so a tight value fails ordinary
/// spawns on a busy machine: CI's runner user broke `subprocess.Popen` at
/// 64. It is a fork-bomb brake, not a quota; the wall-clock kill of the
/// process group is what bounds a run.
pub const PROCESS_LIMIT: u64 = 4096;

/// Which data a run sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Split {
    /// The inner loop's evaluation data.
    Public,
    /// Held-out data for the grade.
    Private,
}

impl Split {
    /// The directory name of the split.
    #[must_use]
    pub const fn dir_name(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Private => "private",
        }
    }

    /// Parses `public` or `private`.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "public" => Some(Self::Public),
            "private" => Some(Self::Private),
            _ => None,
        }
    }
}

/// A validated `task.json`.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskManifest {
    /// Task name; equals its directory name.
    pub id: TaskId,
    /// Task family (`ml`, `heuristic`, `harness`), for reporting.
    pub family: String,
    /// How output is scored.
    pub metric: Metric,
    /// Files in `public/` staged into every run's `data/`.
    pub shared: Vec<String>,
    /// File name of each split's evaluation inputs.
    pub inputs: String,
    /// Score for missing or invalid output.
    pub floor: Score,
    /// Limits for one solution run.
    pub limits: Limits,
}

impl TaskManifest {
    /// Parses and validates a manifest.
    ///
    /// # Errors
    /// [`RuntimeError::Task`] for malformed JSON, missing or mistyped
    /// fields, an unknown metric, or a file name that is not a plain name.
    pub fn parse(text: &str) -> Result<Self, RuntimeError> {
        let bad = |what: &str| RuntimeError::Task(format!("task.json: {what}"));
        let json = Value::parse(text).map_err(|e| bad(&e.to_string()))?;
        let string = |key: &str| {
            json.get(key)
                .and_then(Value::as_str)
                .ok_or_else(|| bad(&format!("`{key}` must be a string")))
        };
        let limits = json
            .get("limits")
            .ok_or_else(|| bad("`limits` is missing"))?;
        let limit = |key: &str| {
            limits
                .get(key)
                .and_then(Value::as_u64)
                .filter(|v| *v > 0)
                .ok_or_else(|| bad(&format!("`limits.{key}` must be a positive integer")))
        };
        let shared = match json.get("shared") {
            None => Vec::new(),
            Some(value) => value
                .as_array()
                .ok_or_else(|| bad("`shared` must be an array"))?
                .iter()
                .map(|v| v.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| bad("`shared` must hold strings"))?,
        };
        let inputs = string("inputs")?.to_owned();
        for name in shared.iter().chain([&inputs]) {
            if !is_plain_file_name(name) {
                return Err(bad(&format!("`{name}` is not a plain file name")));
            }
        }
        let floor = match json.get("floor") {
            None => Score::ZERO,
            Some(value) => Score::new(
                value
                    .as_f64()
                    .ok_or_else(|| bad("`floor` must be a number"))?,
            )?,
        };
        let metric_name = string("metric")?;
        Ok(Self {
            id: TaskId::parse(string("id")?)?,
            family: string("family")?.to_owned(),
            metric: Metric::parse(metric_name)
                .ok_or_else(|| bad(&format!("unknown metric `{metric_name}`")))?,
            shared,
            inputs,
            floor,
            limits: Limits::new(
                Duration::from_secs(limit("cpu_secs")?),
                Duration::from_secs(limit("wall_secs")?),
                limit("memory_mb")?.saturating_mul(1 << 20),
                limit("output_mb").unwrap_or(16).saturating_mul(1 << 20),
                OPEN_FILE_LIMIT,
                PROCESS_LIMIT,
            )?,
        })
    }
}

fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', '\0'])
        && name != OUTPUT_FILE
}

/// A loaded task directory.
#[derive(Debug, Clone)]
pub struct TaskDir {
    root: PathBuf,
    manifest: TaskManifest,
    baseline: Solution,
}

impl TaskDir {
    /// Loads and validates the task at `root`.
    ///
    /// # Errors
    /// [`RuntimeError::Task`] when the manifest is invalid, its `id` differs
    /// from the directory name, or a required file is missing.
    pub fn load(root: &Path) -> Result<Self, RuntimeError> {
        let root = root
            .canonicalize()
            .map_err(|e| RuntimeError::io(format!("resolving task {}", root.display()), e))?;
        let manifest = TaskManifest::parse(&read(&root.join("task.json"))?)?;
        if root.file_name().and_then(|n| n.to_str()) != Some(manifest.id.as_str()) {
            return Err(RuntimeError::Task(format!(
                "{}: directory name differs from id `{}`",
                root.display(),
                manifest.id.as_str()
            )));
        }
        let task = Self {
            baseline: Solution::new(read(&root.join("public").join("baseline.py"))?)?,
            root,
            manifest,
        };
        for split in [Split::Public, Split::Private] {
            for file in [task.inputs_path(split), task.labels_path(split)] {
                if !file.is_file() {
                    return Err(RuntimeError::Task(format!("missing {}", file.display())));
                }
            }
        }
        for name in &task.manifest.shared {
            let file = task.root.join("public").join(name);
            if !file.is_file() {
                return Err(RuntimeError::Task(format!("missing {}", file.display())));
            }
        }
        Ok(task)
    }

    /// Loads every task directory under `suite`, sorted by name.
    ///
    /// # Errors
    /// As [`TaskDir::load`], or when `suite` cannot be listed or is empty.
    pub fn load_suite(suite: &Path) -> Result<Vec<Self>, RuntimeError> {
        let entries = std::fs::read_dir(suite)
            .map_err(|e| RuntimeError::io(format!("listing {}", suite.display()), e))?;
        let mut tasks = Vec::new();
        for entry in entries {
            let path = entry
                .map_err(|e| RuntimeError::io("reading the suite", e))?
                .path();
            if path.join("task.json").is_file() {
                tasks.push(Self::load(&path)?);
            }
        }
        if tasks.is_empty() {
            return Err(RuntimeError::Task(format!(
                "no tasks under {}",
                suite.display()
            )));
        }
        tasks.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
        Ok(tasks)
    }

    /// The canonical task directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The validated manifest.
    #[must_use]
    pub const fn manifest(&self) -> &TaskManifest {
        &self.manifest
    }

    /// The starting solution.
    #[must_use]
    pub const fn baseline(&self) -> &Solution {
        &self.baseline
    }

    fn inputs_path(&self, split: Split) -> PathBuf {
        self.root.join(split.dir_name()).join(&self.manifest.inputs)
    }

    fn labels_path(&self, split: Split) -> PathBuf {
        self.root.join(split.dir_name()).join("labels.txt")
    }

    /// Writes `solution` and the split's data into the empty directory `work`.
    ///
    /// # Errors
    /// [`RuntimeError::Io`] when a file cannot be copied or written.
    pub fn stage(
        &self,
        split: Split,
        solution: &Solution,
        work: &Path,
    ) -> Result<(), RuntimeError> {
        let data = work.join("data");
        std::fs::create_dir_all(&data).map_err(|e| RuntimeError::io("creating data/", e))?;
        std::fs::write(work.join("solution.py"), solution.source())
            .map_err(|e| RuntimeError::io("writing solution.py", e))?;
        let public = self.root.join("public");
        let shared = self
            .manifest
            .shared
            .iter()
            .map(|name| (public.join(name), name));
        for (from, name) in shared.chain([(self.inputs_path(split), &self.manifest.inputs)]) {
            std::fs::copy(&from, data.join(name))
                .map_err(|e| RuntimeError::io(format!("staging {}", from.display()), e))?;
        }
        Ok(())
    }

    /// The program arguments that run a staged solution.
    #[must_use]
    pub fn command(&self) -> (String, Vec<String>) {
        let args = vec![
            "solution.py".to_owned(),
            format!("data/{}", self.manifest.inputs),
            OUTPUT_FILE.to_owned(),
        ];
        ("python3".to_owned(), args)
    }

    /// Scores a solution's output on `split`; the floor when the output is
    /// missing or invalid.
    ///
    /// # Errors
    /// [`RuntimeError::Io`] when the task's own inputs or labels cannot be read.
    pub fn score_output(&self, split: Split, output: Option<&str>) -> Result<Score, RuntimeError> {
        let Some(output) = output else {
            return Ok(self.manifest.floor);
        };
        Ok(self.score(split, output)?.unwrap_or(self.manifest.floor))
    }

    /// Scores `output` on `split` against this task's own inputs and labels;
    /// `None` when the output is invalid.
    ///
    /// # Errors
    /// [`RuntimeError::Io`] when the task's inputs or labels cannot be read.
    pub fn score(&self, split: Split, output: &str) -> Result<Option<Score>, RuntimeError> {
        let inputs = read(&self.inputs_path(split))?;
        let labels = read(&self.labels_path(split))?;
        Ok(self.manifest.metric.score(&inputs, &labels, output))
    }
}

fn read(path: &Path) -> Result<String, RuntimeError> {
    std::fs::read_to_string(path)
        .map_err(|e| RuntimeError::io(format!("reading {}", path.display()), e))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"{
        "id": "toy", "family": "ml", "metric": "r2",
        "shared": ["train.csv"], "inputs": "inputs.csv", "floor": 0.0,
        "limits": {"cpu_secs": 2, "wall_secs": 5, "memory_mb": 256}
    }"#;

    #[test]
    fn parses_a_manifest() {
        let manifest = TaskManifest::parse(MANIFEST).expect("valid");
        assert_eq!(manifest.id.as_str(), "toy");
        assert_eq!(manifest.metric, Metric::R2);
        assert_eq!(manifest.shared, vec!["train.csv".to_owned()]);
        assert_eq!(manifest.limits.memory_bytes(), 256 << 20);
        assert_eq!(manifest.limits.file_bytes(), 16 << 20);
    }

    #[test]
    fn rejects_bad_manifests() {
        let edits = [
            ("unknown metric", r#""metric": "r2""#, r#""metric": "mse""#),
            (
                "path in inputs",
                r#""inputs": "inputs.csv""#,
                r#""inputs": "../private/inputs.csv""#,
            ),
            (
                "output as input",
                r#""inputs": "inputs.csv""#,
                r#""inputs": "output.txt""#,
            ),
            (
                "path in shared",
                r#""shared": ["train.csv"]"#,
                r#""shared": ["a/b"]"#,
            ),
            ("zero limit", r#""cpu_secs": 2"#, r#""cpu_secs": 0"#),
            ("missing limit", r#""cpu_secs": 2, "#, ""),
            ("floor above one", r#""floor": 0.0"#, r#""floor": 1.5"#),
            ("bad id", r#""id": "toy""#, r#""id": "../x""#),
        ];
        for (name, find, replace) in edits {
            assert!(MANIFEST.contains(find), "{name}: fixture drifted");
            assert!(
                TaskManifest::parse(&MANIFEST.replace(find, replace)).is_err(),
                "{name}"
            );
        }
        assert!(TaskManifest::parse("{").is_err(), "not JSON");
    }

    #[test]
    fn split_names() {
        assert_eq!(Split::parse("public"), Some(Split::Public));
        assert_eq!(Split::parse("private"), Some(Split::Private));
        assert_eq!(Split::parse("Private"), None);
        assert_eq!(Split::Private.dir_name(), "private");
    }
}
