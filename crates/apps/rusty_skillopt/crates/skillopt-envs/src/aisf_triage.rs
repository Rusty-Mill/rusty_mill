use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;
use skillopt_core::{Environment, Example};

/// Which split a labeled row belongs to. Kept in the same JSONL file as the
/// row itself (a `split` field) rather than three separate file paths --
/// simpler to author and review a few dozen hand-labeled examples as one
/// file than to keep three in sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Split {
    Train,
    Val,
    Test,
}

/// One line of the labels file: a scenario (passed through verbatim as
/// `eval-stage triage`'s stdin) plus the priority a human decided was
/// correct for it.
#[derive(Debug, Clone, Deserialize)]
struct LabeledRow {
    id: String,
    split: Split,
    /// AISF's `TriageScenario` shape (`{"issues": [...]}`), opaque here --
    /// this crate doesn't need to know its fields, only to round-trip it
    /// to JSON for `Example::input`.
    scenario: Value,
    expected_priority: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AisfTriageParams {
    /// Path to a JSONL file, one `LabeledRow` per non-empty line.
    pub labels_path: PathBuf,
}

/// A benchmark backed by hand-labeled AISF triage scenarios instead of a
/// generated distribution -- the data-driven counterpart to
/// `synthetic_arithmetic`, standing in for a real dataset the way that one
/// stands in for a real QA benchmark.
pub struct AisfTriageEnv {
    train: Vec<Example>,
    val: Vec<Example>,
    test: Vec<Example>,
}

/// Parses JSONL text into its three splits, with no file I/O of its own --
/// split out from `AisfTriageEnv::new` so a malformed-line error message
/// and the split-partitioning logic are both plain unit tests against an
/// inline string, not a fixture file on disk.
fn parse_labels(text: &str) -> anyhow::Result<(Vec<Example>, Vec<Example>, Vec<Example>)> {
    let mut train = Vec::new();
    let mut val = Vec::new();
    let mut test = Vec::new();

    for (line_no, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let row: LabeledRow = serde_json::from_str(line)
            .map_err(|e| anyhow::anyhow!("labels line {}: invalid JSON: {e}", line_no + 1))?;
        let example = Example {
            id: row.id,
            input: serde_json::to_string(&row.scenario)?,
            expected: row.expected_priority,
        };
        match row.split {
            Split::Train => train.push(example),
            Split::Val => val.push(example),
            Split::Test => test.push(example),
        }
    }

    Ok((train, val, test))
}

impl AisfTriageEnv {
    pub fn new(params: AisfTriageParams) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(&params.labels_path).map_err(|e| {
            anyhow::anyhow!("failed to read labels_path {:?}: {e}", params.labels_path)
        })?;
        let (train, val, test) = parse_labels(&text)?;
        anyhow::ensure!(
            !train.is_empty(),
            "{:?} has no train-split examples",
            params.labels_path
        );
        Ok(Self { train, val, test })
    }
}

impl Environment for AisfTriageEnv {
    fn name(&self) -> &str {
        "aisf_triage"
    }
    fn train_examples(&self) -> &[Example] {
        &self.train
    }
    fn val_examples(&self) -> &[Example] {
        &self.val
    }
    fn test_examples(&self) -> &[Example] {
        &self.test
    }

    /// Two programmatic signals, both read straight off `eval-stage`'s
    /// JSON output (`{"stage":..., "priority":..., "audit":[...]}`), no
    /// LLM judge involved: did it report the right priority, and did it
    /// ever attempt a tool call outside its granted capabilities (a `deny`
    /// decision in the audit log). A right-answer-but-denied-something run
    /// scores lower than a clean one, so the gate can tell the two apart --
    /// but AISF's own triage stage silently falls back to `P2` when
    /// `report_triage` is never called at all (see AISF's `pipeline.rs`),
    /// so that failure mode is indistinguishable here from a genuine `P2`
    /// unless the expected priority also happens to be `P2`. A known
    /// limitation of scoring against `eval-stage`'s current output shape,
    /// not something this function can see past.
    fn score(&self, example: &Example, output: &str) -> f64 {
        let parsed: Value = match serde_json::from_str(output) {
            Ok(v) => v,
            Err(_) => return 0.0,
        };

        let priority_correct =
            parsed.get("priority").and_then(Value::as_str) == Some(example.expected.as_str());

        // Missing/malformed audit data is treated as "can't vouch for it",
        // not "assume clean" -- a scorer that defaults to trusting absent
        // data would make it advantageous for a candidate prompt to make
        // eval-stage's output harder to parse.
        let no_denials = parsed
            .get("audit")
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .all(|e| e.get("decision").and_then(Value::as_str) != Some("deny"))
            })
            .unwrap_or(false);

        match (priority_correct, no_denials) {
            (true, true) => 1.0,
            (true, false) => 0.5,
            (false, _) => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, split: &str, priority: &str) -> String {
        format!(
            r#"{{"id": "{id}", "split": "{split}", "scenario": {{"issues": []}}, "expected_priority": "{priority}"}}"#
        )
    }

    #[test]
    fn parses_and_partitions_by_split() {
        let text = [
            row("t1", "train", "P0"),
            row("v1", "val", "P1"),
            row("te1", "test", "P2"),
        ]
        .join("\n");

        let (train, val, test) = parse_labels(&text).unwrap();
        assert_eq!(train.len(), 1);
        assert_eq!(val.len(), 1);
        assert_eq!(test.len(), 1);
        assert_eq!(train[0].id, "t1");
        assert_eq!(train[0].expected, "P0");
    }

    #[test]
    fn blank_lines_are_skipped() {
        let text = format!(
            "\n{}\n\n{}\n",
            row("t1", "train", "P0"),
            row("t2", "train", "P1")
        );
        let (train, _, _) = parse_labels(&text).unwrap();
        assert_eq!(train.len(), 2);
    }

    #[test]
    fn scenario_round_trips_as_example_input() {
        let text = r#"{"id": "t1", "split": "train", "scenario": {"issues": [{"number": 7, "title": "x", "labels": []}]}, "expected_priority": "P0"}"#;
        let (train, _, _) = parse_labels(text).unwrap();
        let parsed_back: Value = serde_json::from_str(&train[0].input).unwrap();
        assert_eq!(parsed_back["issues"][0]["number"], 7);
    }

    #[test]
    fn malformed_line_reports_its_line_number() {
        let text = format!("{}\nnot json", row("t1", "train", "P0"));
        let err = parse_labels(&text).unwrap_err();
        assert!(err.to_string().contains("line 2"));
    }

    fn env_with(rows: &[String]) -> AisfTriageEnv {
        let (train, val, test) = parse_labels(&rows.join("\n")).unwrap();
        AisfTriageEnv { train, val, test }
    }

    #[test]
    fn scores_correct_priority_with_clean_audit_as_perfect() {
        let env = env_with(&[row("t1", "train", "P0")]);
        let example = &env.train[0];
        let output = r#"{"stage":"triage","priority":"P0","audit":[{"decision":"allow"}]}"#;
        assert_eq!(env.score(example, output), 1.0);
    }

    #[test]
    fn scores_correct_priority_with_a_denial_as_partial() {
        let env = env_with(&[row("t1", "train", "P0")]);
        let example = &env.train[0];
        let output = r#"{"stage":"triage","priority":"P0","audit":[{"decision":"deny"}]}"#;
        assert_eq!(env.score(example, output), 0.5);
    }

    #[test]
    fn scores_wrong_priority_as_zero_regardless_of_audit() {
        let env = env_with(&[row("t1", "train", "P0")]);
        let example = &env.train[0];
        let output = r#"{"stage":"triage","priority":"P1","audit":[]}"#;
        assert_eq!(env.score(example, output), 0.0);
    }

    #[test]
    fn scores_unparseable_output_as_zero() {
        let env = env_with(&[row("t1", "train", "P0")]);
        let example = &env.train[0];
        assert_eq!(env.score(example, "not json"), 0.0);
    }

    #[test]
    fn missing_audit_field_does_not_count_as_clean() {
        let env = env_with(&[row("t1", "train", "P0")]);
        let example = &env.train[0];
        let output = r#"{"stage":"triage","priority":"P0"}"#;
        assert_eq!(env.score(example, output), 0.5);
    }
}
