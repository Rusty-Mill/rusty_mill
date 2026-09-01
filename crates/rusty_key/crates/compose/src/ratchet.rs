//! The verification ratchet (P3 feedback, `docs/assessment/RECOMMENDATIONS.md`).
//!
//! The harness already *attributes* every failed turn to a fixed [`FailureType`]
//! plus a `(category, layer)` pair (the `compose` verifier). The ratchet closes
//! the feedback loop the way skill consolidation closes the memory loop: it records
//! each failed-turn attribution to an append-only log, aggregates recurring
//! `(failure_type, category)` pairs, and *proposes* — never auto-applies —
//! `checks.toml` stanzas that would catch them next time. A human reviews and
//! commits the stanza, so verification tightens (ratchets) over time.
//!
//! **Zero aspirational rules (enforced in code):** a check can only be proposed
//! from a *logged* attribution. [`propose_checks`] derives proposals solely from
//! [`RatchetLog::aggregate`] output, and there is no other path — an empty log
//! yields zero proposals. Rules describe failures that actually happened, never
//! ones someone imagined.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::failure::Attribution;
use crate::ComposeError;

/// Minimum times a `(failure_type, category)` must recur before the ratchet
/// proposes a check for it — one-off failures stay noise, recurrence earns a
/// rule.
pub const RATCHET_MIN_OCCURRENCES: usize = 2;

/// Append-only `ratchet.jsonl`: one record per failed-turn attribution.
pub struct RatchetLog {
    path: PathBuf,
    lock: Mutex<()>,
}

/// One logged attribution (the schema written to `ratchet.jsonl`).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RatchetRecord {
    v: u32,
    kind: String,
    ts: f64,
    turn_id: String,
    failure_type: String,
    category: String,
    layer: String,
    check: String,
    evidence: String,
}

/// Recurring failures grouped by `(failure_type, category)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RatchetAggregate {
    /// Fixed taxonomy bucket, snake_case (e.g. `f_tool`).
    pub failure_type: String,
    /// Frozen category vocabulary.
    pub category: String,
    /// How many logged attributions fall in this group.
    pub count: usize,
    /// Distinct layers seen, sorted.
    pub layers: Vec<String>,
    /// Distinct producing checks seen, sorted.
    pub checks: Vec<String>,
    /// One representative evidence string (the first non-empty seen).
    pub example_evidence: String,
}

impl RatchetLog {
    /// Build the log under `dir` (typically `<workspace>/.rustykeys`).
    pub fn new(dir: &Path) -> Self {
        Self {
            path: dir.join("ratchet.jsonl"),
            lock: Mutex::new(()),
        }
    }

    /// Append one attribution from a failed turn. Best-effort, append-only.
    pub fn record(&self, turn_id: &str, a: &Attribution) -> Result<(), ComposeError> {
        let failure_type = serde_json::to_value(a.failure_type)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "f_unknown".to_string());
        let record = RatchetRecord {
            v: 1,
            kind: "ratchet".to_string(),
            ts: now(),
            turn_id: turn_id.to_string(),
            failure_type,
            category: a.category.clone(),
            layer: a.layer.clone(),
            check: a.check.clone(),
            evidence: a.evidence.clone(),
        };
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let line = serde_json::to_string(&record)?;
        let _guard = self.lock.lock();
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(f, "{line}")?;
        Ok(())
    }

    /// Aggregate every logged attribution by `(failure_type, category)`, sorted
    /// by descending count (ties broken by name). Torn-line tolerant: a partial
    /// or unparseable final line is skipped, never fatal.
    pub fn aggregate(&self) -> Result<Vec<RatchetAggregate>, ComposeError> {
        let body = match std::fs::read_to_string(&self.path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };

        // Group, preserving distinct layers/checks and a representative evidence.
        struct Acc {
            count: usize,
            layers: std::collections::BTreeSet<String>,
            checks: std::collections::BTreeSet<String>,
            evidence: String,
        }
        let mut groups: BTreeMap<(String, String), Acc> = BTreeMap::new();
        for rec in body
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str::<RatchetRecord>(l).ok())
        {
            let acc = groups
                .entry((rec.failure_type, rec.category))
                .or_insert_with(|| Acc {
                    count: 0,
                    layers: Default::default(),
                    checks: Default::default(),
                    evidence: String::new(),
                });
            acc.count += 1;
            if !rec.layer.is_empty() {
                acc.layers.insert(rec.layer);
            }
            if !rec.check.is_empty() {
                acc.checks.insert(rec.check);
            }
            if acc.evidence.is_empty() && !rec.evidence.is_empty() {
                acc.evidence = rec.evidence;
            }
        }

        let mut out: Vec<RatchetAggregate> = groups
            .into_iter()
            .map(|((failure_type, category), acc)| RatchetAggregate {
                failure_type,
                category,
                count: acc.count,
                layers: acc.layers.into_iter().collect(),
                checks: acc.checks.into_iter().collect(),
                example_evidence: acc.evidence,
            })
            .collect();
        out.sort_by(|a, b| {
            b.count
                .cmp(&a.count)
                .then_with(|| a.failure_type.cmp(&b.failure_type))
                .then_with(|| a.category.cmp(&b.category))
        });
        Ok(out)
    }
}

/// A `checks.toml` stanza the ratchet proposes for a recurring failure. Advisory
/// only — the human edits the placeholder command and commits it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposedCheck {
    /// The proposed (sanitized, unique) check name.
    pub name: String,
    /// The rendered `[[check]]` TOML stanza, with explanatory comments.
    pub stanza: String,
}

/// Propose `checks.toml` stanzas for the aggregates that recur at least
/// `min_occurrences` times. Derives entirely from `aggregates` (the log), so an
/// empty log yields no proposals — the "zero aspirational rules" guarantee.
pub fn propose_checks(aggregates: &[RatchetAggregate], min_occurrences: usize) -> Vec<ProposedCheck> {
    aggregates
        .iter()
        .filter(|a| a.count >= min_occurrences)
        .map(|a| {
            let name = format!(
                "ratchet_{}_{}",
                sanitize(&a.failure_type),
                sanitize(&a.category)
            );
            let evidence = if a.example_evidence.is_empty() {
                String::new()
            } else {
                format!("\n# e.g. {}", one_line(&a.example_evidence))
            };
            let covers = if a.checks.is_empty() {
                String::new()
            } else {
                a.checks.join("\", \"")
            };
            let stanza = format!(
                "# Proposed by /ratchet: {ft} / {cat} recurred {count}× \
                 (layers: {layers}).{evidence}\n\
                 # Replace `command`/`expected_substring` with a deterministic check \
                 that would catch this,\n\
                 # then move the stanza into .rustykeys/checks.toml.\n\
                 [[check]]\n\
                 name = \"{name}\"\n\
                 command = \"REPLACE_ME\"\n\
                 expected_substring = \"\"\n\
                 covers = [\"{covers}\"]\n\
                 method = \"registered_test\"",
                ft = a.failure_type,
                cat = a.category,
                count = a.count,
                layers = if a.layers.is_empty() {
                    "—".to_string()
                } else {
                    a.layers.join(", ")
                },
            );
            ProposedCheck { name, stanza }
        })
        .collect()
}

/// Render the `/ratchet` report: the recurring-failure summary followed by the
/// proposed stanzas (or a note that none recur often enough yet).
pub fn render_ratchet(aggregates: &[RatchetAggregate], proposals: &[ProposedCheck]) -> String {
    if aggregates.is_empty() {
        return "(no failed-turn attributions logged yet)".to_string();
    }
    let mut s = String::from("recurring failures (failure_type / category × count):\n");
    for a in aggregates {
        s.push_str(&format!(
            "  {} / {} × {}\n",
            a.failure_type, a.category, a.count
        ));
    }
    if proposals.is_empty() {
        s.push_str(&format!(
            "\nnothing recurs ≥{RATCHET_MIN_OCCURRENCES}× yet — no checks proposed."
        ));
    } else {
        s.push_str("\nproposed checks.toml stanzas (review, edit, then commit):\n\n");
        s.push_str(
            &proposals
                .iter()
                .map(|p| p.stanza.clone())
                .collect::<Vec<_>>()
                .join("\n\n"),
        );
    }
    s
}

/// Lower-case, keep `[a-z0-9_]`, collapse the rest to `_` — for a TOML-safe name.
fn sanitize(s: &str) -> String {
    let mut out: String = s
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    while out.contains("__") {
        out = out.replace("__", "_");
    }
    out.trim_matches('_').to_string()
}

/// Flatten whitespace so evidence fits on a single comment line.
fn one_line(s: &str) -> String {
    let flat: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.len() > 120 {
        format!("{}…", &flat[..120])
    } else {
        flat
    }
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::failure::FailureType;

    fn attr(ft: FailureType, category: &str, layer: &str, check: &str, ev: &str) -> Attribution {
        Attribution {
            check: check.to_string(),
            failure_type: ft,
            category: category.to_string(),
            layer: layer.to_string(),
            evidence: ev.to_string(),
        }
    }

    fn log(tag: &str) -> (RatchetLog, PathBuf) {
        let dir = std::env::temp_dir().join(format!("rk-ratchet-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (RatchetLog::new(&dir), dir)
    }

    #[test]
    fn empty_log_yields_no_aggregates_or_proposals() {
        let (rl, dir) = log("empty");
        let aggs = rl.aggregate().unwrap();
        assert!(aggs.is_empty());
        // Zero aspirational rules: no log entry → no proposable check.
        assert!(propose_checks(&aggs, RATCHET_MIN_OCCURRENCES).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn aggregates_by_failure_type_and_category() {
        let (rl, dir) = log("agg");
        rl.record("t1", &attr(FailureType::FTool, "build", "feed/tools", "unit", "boom"))
            .unwrap();
        rl.record("t2", &attr(FailureType::FTool, "build", "feed/exec", "lint", "kaboom"))
            .unwrap();
        rl.record("t3", &attr(FailureType::FVerify, "missing", "compose", "vfy", "no test"))
            .unwrap();

        let aggs = rl.aggregate().unwrap();
        assert_eq!(aggs.len(), 2);
        // Most-frequent first.
        assert_eq!(aggs[0].failure_type, "f_tool");
        assert_eq!(aggs[0].category, "build");
        assert_eq!(aggs[0].count, 2);
        assert_eq!(aggs[0].layers, vec!["feed/exec", "feed/tools"]);
        assert_eq!(aggs[0].checks, vec!["lint", "unit"]);
        assert_eq!(aggs[0].example_evidence, "boom");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn proposes_only_recurring_failures() {
        let (rl, dir) = log("propose");
        // f_tool/build twice (recurs) ; f_verify/missing once (one-off).
        rl.record("t1", &attr(FailureType::FTool, "build", "feed", "unit", "x"))
            .unwrap();
        rl.record("t2", &attr(FailureType::FTool, "build", "feed", "unit", "y"))
            .unwrap();
        rl.record("t3", &attr(FailureType::FVerify, "missing", "compose", "v", "z"))
            .unwrap();

        let aggs = rl.aggregate().unwrap();
        let proposals = propose_checks(&aggs, RATCHET_MIN_OCCURRENCES);
        assert_eq!(proposals.len(), 1);
        let p = &proposals[0];
        assert_eq!(p.name, "ratchet_f_tool_build");
        assert!(p.stanza.contains("[[check]]"));
        assert!(p.stanza.contains("name = \"ratchet_f_tool_build\""));
        assert!(p.stanza.contains("REPLACE_ME"));
        assert!(p.stanza.contains("recurred 2×"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn render_includes_summary_and_stanzas() {
        let (rl, dir) = log("render");
        rl.record("t1", &attr(FailureType::FTool, "build", "feed", "unit", "x"))
            .unwrap();
        rl.record("t2", &attr(FailureType::FTool, "build", "feed", "unit", "y"))
            .unwrap();
        let aggs = rl.aggregate().unwrap();
        let proposals = propose_checks(&aggs, RATCHET_MIN_OCCURRENCES);
        let out = render_ratchet(&aggs, &proposals);
        assert!(out.contains("recurring failures"));
        assert!(out.contains("f_tool / build × 2"));
        assert!(out.contains("[[check]]"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn render_notes_when_nothing_recurs() {
        let (rl, dir) = log("norecur");
        rl.record("t1", &attr(FailureType::FTool, "build", "feed", "unit", "x"))
            .unwrap();
        let aggs = rl.aggregate().unwrap();
        let proposals = propose_checks(&aggs, RATCHET_MIN_OCCURRENCES);
        let out = render_ratchet(&aggs, &proposals);
        assert!(out.contains("no checks proposed"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
