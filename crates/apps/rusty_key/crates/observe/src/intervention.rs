//! The `InterventionLogger` and the M-HIR metric (PRD 04, ADR-0019).
//!
//! M-HIR — *Missing-Harness* Human Intervention Rate — counts only `avoidable`
//! interventions (runtime support a maturing harness could have closed) in the
//! numerator; `benign` and `unavoidable` records are kept for transparency but
//! excluded. The denominator is *turns* (ADR-0018 divergence), passed in by the
//! caller from `EvidenceJournal::count_turns()`.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The seven UI-observable intervention kinds (PRD 04).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterventionKind {
    /// User set `/task` while one was already active.
    TaskOverride,
    /// User ran `/reflect` or `/sleep`.
    ManualReflect,
    /// User ran `/groom`.
    ManualGroom,
    /// User inspected `/verify`.
    ManualVerify,
    /// User sent a message after an unverified turn.
    UnverifiedFollowup,
    /// User blocked a tool approval request.
    ToolBlock,
    /// User edited a file directly (desktop only).
    DirectEdit,
}

/// Whether an intervention reflects a missing harness capability (ADR-0019).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Avoidability {
    /// Runtime support a maturing harness could close — counts toward M-HIR.
    Avoidable,
    /// The policy/boundary working as intended — excluded.
    Unavoidable,
    /// Healthy human action, not a gap — excluded.
    Benign,
}

impl InterventionKind {
    /// The v1-intent `(avoidability, harness_gap, burden)` classification (PRD 04).
    fn classify(self) -> (Avoidability, &'static str, u8) {
        use Avoidability::*;
        use InterventionKind::*;
        match self {
            TaskOverride => (Avoidable, "task_interface", 1),
            ManualReflect => (Avoidable, "memory", 1),
            ManualGroom => (Avoidable, "memory", 1),
            ManualVerify => (Benign, "verification", 0),
            UnverifiedFollowup => (Avoidable, "verification", 2),
            ToolBlock => (Unavoidable, "permissions", 1),
            DirectEdit => (Avoidable, "tools", 3),
        }
    }
}

/// One recorded intervention (PRD 04; data-model §4.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterventionRecord {
    /// The kind of intervention.
    pub kind: InterventionKind,
    /// Free-text note (e.g. the overriding instruction).
    pub note: String,
    /// Whether it counts toward M-HIR.
    pub avoidability: Avoidability,
    /// Which harness responsibility it points at.
    pub harness_gap: String,
    /// Imposed burden, 0–3.
    pub burden: u8,
    /// Dedup key: one user action → one record.
    pub source_message_id: String,
}

/// The computed M-HIR snapshot (PRD 04).
#[derive(Debug, Clone, PartialEq)]
pub struct MhirReport {
    /// Avoidable count — the M-HIR numerator (D2/F23).
    pub n_interventions: usize,
    /// Unavoidable records (excluded; surfaced for transparency).
    pub n_unavoidable: usize,
    /// Benign records (excluded; surfaced for transparency).
    pub n_benign: usize,
    /// Denominator = turns (ADR-0018 divergence).
    pub n_turns: usize,
    /// All-time cumulative rate = avoidable / turns.
    pub rate: f64,
    /// Count by kind (all recorded kinds).
    pub breakdown: HashMap<InterventionKind, usize>,
}

/// Append-only intervention log at `<dir>/interventions.jsonl`.
pub struct InterventionLogger {
    path: PathBuf,
    session_id: String,
}

impl InterventionLogger {
    /// Logger under `dir`, stamping records with `session_id`.
    pub fn new(dir: &Path, session_id: impl Into<String>) -> Self {
        Self {
            path: dir.join("interventions.jsonl"),
            session_id: session_id.into(),
        }
    }

    /// Record an intervention. De-dupes by `source_message_id` (one action → one
    /// record): if a record with the same id already exists, this is a no-op.
    pub fn record(
        &self,
        kind: InterventionKind,
        note: &str,
        source_message_id: &str,
    ) -> Result<(), crate::ObserveError> {
        if self
            .existing_ids()?
            .iter()
            .any(|id| id == source_message_id)
        {
            return Ok(());
        }
        let (avoidability, harness_gap, burden) = kind.classify();
        let record = serde_json::json!({
            "v": 1,
            "ts": now_secs(),
            "session_id": self.session_id,
            "kind": kind,
            "note": note,
            "avoidability": avoidability,
            "harness_gap": harness_gap,
            "burden": burden,
            "source_message_id": source_message_id,
        });
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut line = serde_json::to_string(&record)?;
        line.push('\n');
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.write_all(line.as_bytes())?;
        Ok(())
    }

    /// The most recent `n` well-formed records (torn-line tolerant).
    pub fn recent(&self, n: usize) -> Result<Vec<InterventionRecord>, crate::ObserveError> {
        let all = self.records()?;
        let start = all.len().saturating_sub(n);
        Ok(all[start..].to_vec())
    }

    /// Compute M-HIR over `total_turns` (the denominator; ADR-0018). The
    /// numerator counts only `avoidable` records (not raw HIR; D2/F23).
    pub fn mhir(&self, total_turns: usize) -> Result<MhirReport, crate::ObserveError> {
        let records = self.records()?;
        let mut n_avoidable = 0;
        let mut n_unavoidable = 0;
        let mut n_benign = 0;
        let mut breakdown: HashMap<InterventionKind, usize> = HashMap::new();
        for r in &records {
            *breakdown.entry(r.kind).or_insert(0) += 1;
            match r.avoidability {
                Avoidability::Avoidable => n_avoidable += 1,
                Avoidability::Unavoidable => n_unavoidable += 1,
                Avoidability::Benign => n_benign += 1,
            }
        }
        let rate = if total_turns == 0 {
            0.0
        } else {
            n_avoidable as f64 / total_turns as f64
        };
        Ok(MhirReport {
            n_interventions: n_avoidable,
            n_unavoidable,
            n_benign,
            n_turns: total_turns,
            rate,
            breakdown,
        })
    }

    /// Avoidable-intervention count per session, in first-seen order — the
    /// numerator source for the cross-session M-HIR trend (the denominator,
    /// turns-per-session, lives in the evidence journal).
    pub fn avoidable_by_session(&self) -> Result<Vec<(String, usize)>, crate::ObserveError> {
        let mut order: Vec<String> = Vec::new();
        let mut counts: HashMap<String, usize> = HashMap::new();
        for v in self.raw_lines()? {
            let Some(sid) = v.get("session_id").and_then(Value::as_str) else {
                continue;
            };
            let sid = sid.to_string();
            if !counts.contains_key(&sid) {
                order.push(sid.clone());
                counts.insert(sid.clone(), 0);
            }
            if v.get("avoidability").and_then(Value::as_str) == Some("avoidable") {
                if let Some(c) = counts.get_mut(&sid) {
                    *c += 1;
                }
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

    fn existing_ids(&self) -> Result<Vec<String>, crate::ObserveError> {
        Ok(self
            .raw_lines()?
            .iter()
            .filter_map(|v| {
                v.get("source_message_id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect())
    }

    fn records(&self) -> Result<Vec<InterventionRecord>, crate::ObserveError> {
        Ok(self
            .raw_lines()?
            .into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect())
    }

    fn raw_lines(&self) -> Result<Vec<Value>, crate::ObserveError> {
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

    fn tmp(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("rk-intv-{}-{}", tag, std::process::id()))
    }

    #[test]
    fn only_avoidable_counts_toward_mhir() {
        let dir = tmp("mhir");
        let _ = std::fs::remove_dir_all(&dir);
        let log = InterventionLogger::new(&dir, "s_test");

        log.record(InterventionKind::TaskOverride, "fix parser", "m1")
            .unwrap(); // avoidable
        log.record(InterventionKind::ToolBlock, "blocked rm", "m2")
            .unwrap(); // unavoidable
        log.record(InterventionKind::ManualVerify, "looked", "m3")
            .unwrap(); // benign

        let report = log.mhir(4).unwrap();
        assert_eq!(report.n_interventions, 1);
        assert_eq!(report.n_unavoidable, 1);
        assert_eq!(report.n_benign, 1);
        assert_eq!(report.n_turns, 4);
        assert!((report.rate - 0.25).abs() < 1e-9);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dedupes_by_source_message_id() {
        let dir = tmp("dedup");
        let _ = std::fs::remove_dir_all(&dir);
        let log = InterventionLogger::new(&dir, "s_test");

        log.record(InterventionKind::TaskOverride, "first", "m1")
            .unwrap();
        // Same source message → dropped (one action → one record).
        log.record(InterventionKind::UnverifiedFollowup, "second", "m1")
            .unwrap();

        let recent = log.recent(10).unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].kind, InterventionKind::TaskOverride);
        assert_eq!(recent[0].note, "first");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn classification_matches_table() {
        assert_eq!(
            InterventionKind::DirectEdit.classify(),
            (Avoidability::Avoidable, "tools", 3)
        );
        assert_eq!(
            InterventionKind::ToolBlock.classify().0,
            Avoidability::Unavoidable
        );
        assert_eq!(
            InterventionKind::ManualVerify.classify().0,
            Avoidability::Benign
        );
    }
}
