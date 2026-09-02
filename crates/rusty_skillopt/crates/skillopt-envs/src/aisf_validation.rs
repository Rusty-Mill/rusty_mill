use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;
use skillopt_core::{Environment, Example};

/// Which split a labeled row belongs to. Same convention as
/// `aisf_triage.rs`: kept in the row itself (a `split` field) rather than
/// three separate file paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Split {
    Train,
    Val,
    Test,
}

/// One line of the labels file: a scenario (AISF's `ValidationScenario`
/// shape -- `{"pr_number":..., "diff":..., "tests_passed":...,
/// "test_summary":...}`, passed through verbatim as `eval-stage
/// validation`'s stdin) plus the verdict a human decided was correct for
/// it.
#[derive(Debug, Clone, Deserialize)]
struct LabeledRow {
    id: String,
    split: Split,
    scenario: Value,
    expected_verdict: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AisfValidationParams {
    /// Path to a JSONL file, one `LabeledRow` per non-empty line.
    pub labels_path: PathBuf,
}

/// A benchmark backed by hand-labeled AISF validation scenarios --
/// `aisf_triage`'s sibling for the `validation` stage. A PR review isn't
/// a GitHub issue list, so this is its own `Environment` rather than
/// `aisf_triage` with renamed fields, the same way AISF's own
/// `ValidationScenario` is a genuinely separate type from
/// `TriageScenario`.
pub struct AisfValidationEnv {
    train: Vec<Example>,
    val: Vec<Example>,
    test: Vec<Example>,
}

/// Parses JSONL text into its three splits, with no file I/O of its own
/// -- same rationale as `aisf_triage::parse_labels`: a malformed-line
/// error message and the split-partitioning logic are both plain unit
/// tests against an inline string, not a fixture file on disk.
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
            expected: row.expected_verdict,
        };
        match row.split {
            Split::Train => train.push(example),
            Split::Val => val.push(example),
            Split::Test => test.push(example),
        }
    }

    Ok((train, val, test))
}

impl AisfValidationEnv {
    pub fn new(params: AisfValidationParams) -> anyhow::Result<Self> {
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

impl Environment for AisfValidationEnv {
    fn name(&self) -> &str {
        "aisf_validation"
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
    /// JSON output (`{"stage":..., "verdict":..., "audit":[...]}`), no
    /// LLM judge involved -- the same shape `aisf_triage`'s scorer uses,
    /// with `verdict` in place of `priority`. Same known limitation, too:
    /// AISF's own validation stage silently falls back to `NeedsHuman`
    /// when `report_validation` is never called at all (see AISF's
    /// `pipeline.rs`), so that failure mode is indistinguishable here
    /// from a genuine `NeedsHuman` unless the expected verdict also
    /// happens to be `NeedsHuman`.
    fn score(&self, example: &Example, output: &str) -> f64 {
        let parsed: Value = match serde_json::from_str(output) {
            Ok(v) => v,
            Err(_) => return 0.0,
        };

        let verdict_correct =
            parsed.get("verdict").and_then(Value::as_str) == Some(example.expected.as_str());

        let no_denials = parsed
            .get("audit")
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .all(|e| e.get("decision").and_then(Value::as_str) != Some("deny"))
            })
            .unwrap_or(false);

        match (verdict_correct, no_denials) {
            (true, true) => 1.0,
            (true, false) => 0.5,
            (false, _) => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, split: &str, verdict: &str) -> String {
        format!(
            r#"{{"id": "{id}", "split": "{split}", "scenario": {{"pr_number": 1, "diff": "x", "tests_passed": true, "test_summary": "ok"}}, "expected_verdict": "{verdict}"}}"#
        )
    }

    #[test]
    fn parses_and_partitions_by_split() {
        let text = [
            row("t1", "train", "Pass"),
            row("v1", "val", "Fail"),
            row("te1", "test", "NeedsHuman"),
        ]
        .join("\n");

        let (train, val, test) = parse_labels(&text).unwrap();
        assert_eq!(train.len(), 1);
        assert_eq!(val.len(), 1);
        assert_eq!(test.len(), 1);
        assert_eq!(train[0].id, "t1");
        assert_eq!(train[0].expected, "Pass");
    }

    #[test]
    fn blank_lines_are_skipped() {
        let text = format!(
            "\n{}\n\n{}\n",
            row("t1", "train", "Pass"),
            row("t2", "train", "Fail")
        );
        let (train, _, _) = parse_labels(&text).unwrap();
        assert_eq!(train.len(), 2);
    }

    #[test]
    fn scenario_round_trips_as_example_input() {
        let text = r#"{"id": "t1", "split": "train", "scenario": {"pr_number": 7, "diff": "d", "tests_passed": false, "test_summary": "1 failed"}, "expected_verdict": "Fail"}"#;
        let (train, _, _) = parse_labels(text).unwrap();
        let parsed_back: Value = serde_json::from_str(&train[0].input).unwrap();
        assert_eq!(parsed_back["pr_number"], 7);
        assert_eq!(parsed_back["tests_passed"], false);
    }

    #[test]
    fn malformed_line_reports_its_line_number() {
        let text = format!("{}\nnot json", row("t1", "train", "Pass"));
        let err = parse_labels(&text).unwrap_err();
        assert!(err.to_string().contains("line 2"));
    }

    fn env_with(rows: &[String]) -> AisfValidationEnv {
        let (train, val, test) = parse_labels(&rows.join("\n")).unwrap();
        AisfValidationEnv { train, val, test }
    }

    #[test]
    fn scores_correct_verdict_with_clean_audit_as_perfect() {
        let env = env_with(&[row("t1", "train", "Pass")]);
        let example = &env.train[0];
        let output = r#"{"stage":"validation","verdict":"Pass","audit":[{"decision":"allow"}]}"#;
        assert_eq!(env.score(example, output), 1.0);
    }

    #[test]
    fn scores_correct_verdict_with_a_denial_as_partial() {
        let env = env_with(&[row("t1", "train", "Pass")]);
        let example = &env.train[0];
        let output = r#"{"stage":"validation","verdict":"Pass","audit":[{"decision":"deny"}]}"#;
        assert_eq!(env.score(example, output), 0.5);
    }

    #[test]
    fn scores_wrong_verdict_as_zero_regardless_of_audit() {
        let env = env_with(&[row("t1", "train", "Pass")]);
        let example = &env.train[0];
        let output = r#"{"stage":"validation","verdict":"Fail","audit":[]}"#;
        assert_eq!(env.score(example, output), 0.0);
    }

    #[test]
    fn scores_unparseable_output_as_zero() {
        let env = env_with(&[row("t1", "train", "Pass")]);
        let example = &env.train[0];
        assert_eq!(env.score(example, "not json"), 0.0);
    }

    #[test]
    fn missing_audit_field_does_not_count_as_clean() {
        let env = env_with(&[row("t1", "train", "Pass")]);
        let example = &env.train[0];
        let output = r#"{"stage":"validation","verdict":"Pass"}"#;
        assert_eq!(env.score(example, output), 0.5);
    }
}
