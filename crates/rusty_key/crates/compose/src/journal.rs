//! The append-only evidence journal (PRD 05; data-model §4.1). One JSONL line
//! per turn, versioned (ADR-0027), torn-line tolerant on read (§10).

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rk_observe::{redact, Episode};
use serde_json::{json, Value};

use crate::verify::VerificationReport;
use crate::ComposeError;

/// Schema version for journal records (ADR-0027).
const SCHEMA_VERSION: u32 = 1;

/// Append-only JSONL evidence journal at `<dir>/evidence.jsonl`, plus the H3
/// episode packages under `<dir>/episodes/`.
pub struct EvidenceJournal {
    path: PathBuf,
    dir: PathBuf,
}

impl EvidenceJournal {
    /// Journal under `dir` (typically `<workspace>/.rustykeys`).
    pub fn new(dir: &Path) -> Self {
        Self {
            path: dir.join("evidence.jsonl"),
            dir: dir.to_path_buf(),
        }
    }

    /// Write a full H3 episode package to `episodes/<turn_id>.json` and append a
    /// one-line `kind: "episode"` summary to `evidence.jsonl` (data-model §5).
    pub fn record_episode(&self, pkg: &crate::EpisodePackage) -> Result<(), ComposeError> {
        let dir = self.dir.join("episodes");
        std::fs::create_dir_all(&dir)?;
        let file = dir.join(format!("{}.json", pkg.turn_id));
        let json = serde_json::to_string_pretty(pkg)?;
        std::fs::write(&file, json)?;

        let summary = json!({
            "v": SCHEMA_VERSION,
            "kind": "episode",
            "ts": now_secs(),
            "session_id": Value::Null,
            "turn_id": pkg.turn_id,
            "episode_id": pkg.episode_id,
            "outcome": serde_json::to_value(pkg.outcome)?,
        });
        self.append(&summary)
    }

    /// Append a non-H3 turn record. The `evidence` summary carries each tool's
    /// name + status only (the tool-name is redacted defensively, ADR-0026); full
    /// args/results are not persisted below H3.
    pub fn record_turn(
        &self,
        session_id: &str,
        turn_id: &str,
        reply: &str,
        episode: &Episode,
        report: &VerificationReport,
    ) -> Result<(), ComposeError> {
        let evidence: Vec<Value> = episode
            .tool_events
            .iter()
            .map(|e| json!({ "name": e.name, "status": e.outcome.status }))
            .collect();

        let record = json!({
            "v": SCHEMA_VERSION,
            "kind": "turn",
            "ts": now_secs(),
            "session_id": session_id,
            "turn_id": turn_id,
            "parent_turn_id": Value::Null,
            "verified": report.verified,
            "checks": serde_json::to_value(&report.checks)?,
            "attributions": serde_json::to_value(&report.attributions)?,
            "outcome": Value::Null,
            "limits": report.limits,
            "evidence": evidence,
            "reply": redact::redact_text(reply),
        });

        self.append(&record)
    }

    /// Append a consolidation changelog record (data-model §4.1).
    pub fn record_improvement(
        &self,
        session_id: &str,
        scope: &str,
        created: usize,
        updated: usize,
        pruned: usize,
        groomed: usize,
    ) -> Result<(), ComposeError> {
        let record = json!({
            "v": SCHEMA_VERSION,
            "kind": "improvement",
            "ts": now_secs(),
            "session_id": session_id,
            "scope": scope,
            "created": created,
            "updated": updated,
            "pruned": pruned,
            "groomed": groomed,
        });
        self.append(&record)
    }

    /// Append a compaction record (PRD 06 / Phase 8). `tier` is `micro`,
    /// `session`, or `full`; `dropped`/`summarized` count the affected messages;
    /// `used`/`limit` are the token line-item snapshot that triggered it.
    pub fn record_compaction(
        &self,
        session_id: &str,
        tier: &str,
        dropped: usize,
        summarized: usize,
        used: usize,
        limit: usize,
    ) -> Result<(), ComposeError> {
        let record = json!({
            "v": SCHEMA_VERSION,
            "kind": "compaction",
            "ts": now_secs(),
            "session_id": session_id,
            "tier": tier,
            "dropped": dropped,
            "summarized": summarized,
            "used_tokens": used,
            "context_limit": limit,
        });
        self.append(&record)
    }

    /// Count well-formed `kind == "compaction"` records (torn-line tolerant).
    pub fn count_compactions(&self) -> Result<usize, ComposeError> {
        Ok(self
            .parsed_lines()?
            .iter()
            .filter(|v| v.get("kind").and_then(Value::as_str) == Some("compaction"))
            .count())
    }

    fn append(&self, record: &Value) -> Result<(), ComposeError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut line = serde_json::to_string(record)?;
        line.push('\n');
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.write_all(line.as_bytes())?;
        Ok(())
    }

    /// The most recent `n` well-formed records (torn/partial lines skipped).
    pub fn recent(&self, n: usize) -> Result<Vec<Value>, ComposeError> {
        let all = self.parsed_lines()?;
        let start = all.len().saturating_sub(n);
        Ok(all[start..].to_vec())
    }

    /// Count well-formed `kind == "turn"` records (torn-line tolerant). Used by
    /// the M-HIR denominator (PRD 04) without coupling observe to this path.
    pub fn count_turns(&self) -> Result<usize, ComposeError> {
        Ok(self
            .parsed_lines()?
            .iter()
            .filter(|v| v.get("kind").and_then(Value::as_str) == Some("turn"))
            .count())
    }

    /// Turn count per session, in first-seen order — the denominator source for
    /// the cross-session M-HIR trend.
    pub fn turns_by_session(&self) -> Result<Vec<(String, usize)>, ComposeError> {
        let mut order: Vec<String> = Vec::new();
        let mut counts: HashMap<String, usize> = HashMap::new();
        for v in self.parsed_lines()? {
            if v.get("kind").and_then(Value::as_str) != Some("turn") {
                continue;
            }
            let Some(sid) = v.get("session_id").and_then(Value::as_str) else {
                continue;
            };
            let sid = sid.to_string();
            if !counts.contains_key(&sid) {
                order.push(sid.clone());
                counts.insert(sid.clone(), 0);
            }
            if let Some(c) = counts.get_mut(&sid) {
                *c += 1;
            }
        }
        Ok(order
            .into_iter()
            .map(|s| {
                let c = counts.get(&s).copied().unwrap_or(0);
                (s, c)
            })
            .collect())
    }

    fn parsed_lines(&self) -> Result<Vec<Value>, ComposeError> {
        let file = match std::fs::File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut out = Vec::new();
        for line in BufReader::new(file).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            // Torn-line tolerant: a partial final line that fails to parse is skipped.
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                out.push(v);
            }
        }
        Ok(out)
    }
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Verifier;
    use rk_observe::{ToolEvent, ToolOutcome};

    fn tmp_dir(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("rk-journal-{}-{}", tag, std::process::id()))
    }

    #[test]
    fn round_trips_turns_and_counts() {
        let dir = tmp_dir("rt");
        let _ = std::fs::remove_dir_all(&dir);
        let journal = EvidenceJournal::new(&dir);

        let ep = Episode {
            tool_events: vec![ToolEvent {
                name: "read_file".into(),
                args: serde_json::json!({"path": "a"}),
                outcome: ToolOutcome::ok("data"),
            }],
            final_reached: true,
        };
        let report = Verifier::deterministic().verify("done", &ep);

        journal
            .record_turn("s_test", "turn_1", "done", &ep, &report)
            .unwrap();
        journal
            .record_turn("s_test", "turn_2", "done", &ep, &report)
            .unwrap();

        assert_eq!(journal.count_turns().unwrap(), 2);
        let recent = journal.recent(1).unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0]["turn_id"], serde_json::json!("turn_2"));
        assert_eq!(recent[0]["verified"], serde_json::json!(true));
        assert_eq!(recent[0]["evidence"][0]["status"], serde_json::json!("ok"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn torn_final_line_is_skipped() {
        let dir = tmp_dir("torn");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("evidence.jsonl");
        // One good record + one torn/partial line with no newline.
        std::fs::write(
            &path,
            "{\"v\":1,\"kind\":\"turn\",\"turn_id\":\"a\"}\n{\"v\":1,\"kind\":\"tu",
        )
        .unwrap();

        let journal = EvidenceJournal::new(&dir);
        assert_eq!(journal.count_turns().unwrap(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
