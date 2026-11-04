//! The on-disk shape of an alien-lens probe row (#708).
//!
//! Positional bincode under the schema tag below: the v1 layout is the
//! format, and any layout change needs a new tag (see `record.rs`).

use orch_lens::alien::{Outcome, ProbeRow};
use rusty_multimodal_db_engine::durability::record_blob::Fnv1a64;
use rusty_multimodal_db_engine::generic::traits::{
    IndexedField, Record, ScannableField, SchemaTag,
};
use serde::{Deserialize, Serialize};

use crate::record::{ById, Revision};
use crate::StoreError;

/// Engine key for a probe: a hash of what identifies it. The fields are
/// stored too, so a collision is refused instead of read as another probe.
pub fn key(row: &ProbeRow) -> i64 {
    let mut hash = Fnv1a64::new();
    for part in [
        row.task.as_bytes(),
        &row.claim.to_le_bytes(),
        &row.candidate.to_le_bytes(),
        row.site.as_bytes(),
        row.mechanism.as_bytes(),
    ] {
        hash.update(&(part.len() as u64).to_le_bytes());
        hash.update(part);
    }
    hash.finish() as i64
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeRecord {
    id: i64,
    pub revision: i64,
    task: String,
    claim: u64,
    candidate: u64,
    site: String,
    mechanism: String,
    novel: bool,
    outcome: String,
    runs: u32,
    tokens: u64,
    probe_sha256: String,
}

impl ProbeRecord {
    pub fn new(row: &ProbeRow, revision: u64) -> Self {
        Self {
            id: key(row),
            revision: revision as i64,
            task: row.task.clone(),
            claim: row.claim,
            candidate: row.candidate,
            site: row.site.clone(),
            mechanism: row.mechanism.clone(),
            novel: row.novel,
            outcome: row.outcome.name().to_owned(),
            runs: row.runs,
            tokens: row.tokens,
            probe_sha256: row.probe_sha256.clone(),
        }
    }

    /// Does this record hold the probe `row` names?
    pub fn is(&self, row: &ProbeRow) -> bool {
        self.task == row.task
            && self.claim == row.claim
            && self.candidate == row.candidate
            && self.site == row.site
            && self.mechanism == row.mechanism
    }

    /// The row, once its outcome word and key check out.
    pub fn into_row(self) -> Result<ProbeRow, StoreError> {
        let outcome = Outcome::parse(&self.outcome).ok_or_else(|| {
            StoreError::Corrupt(format!("probe outcome {:?} is unknown", self.outcome))
        })?;
        let row = ProbeRow {
            task: self.task,
            claim: self.claim,
            candidate: self.candidate,
            site: self.site,
            mechanism: self.mechanism,
            novel: self.novel,
            outcome,
            runs: self.runs,
            tokens: self.tokens,
            probe_sha256: self.probe_sha256,
        };
        if key(&row) != self.id {
            return Err(StoreError::Corrupt(format!(
                "probe key {} does not match its fields",
                self.id
            )));
        }
        Ok(row)
    }
}

impl Record for ProbeRecord {
    type Id = i64;
    fn id(&self) -> i64 {
        self.id
    }
}

impl SchemaTag for ProbeRecord {
    const SCHEMA_TAG: &'static str = "rusty_orch::ProbeRecord::v1";
}

impl IndexedField<ById> for ProbeRecord {
    type IndexValue = i64;
    fn indexed_value(&self) -> &i64 {
        &self.id
    }
}

impl ScannableField<Revision> for ProbeRecord {
    type ScanValue = i64;
    fn scannable_value(&self) -> i64 {
        self.revision
    }
    fn set_scannable_value(&mut self, value: i64) {
        self.revision = value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> ProbeRow {
        ProbeRow {
            task: "t".to_owned(),
            claim: 1,
            candidate: 2,
            site: "a::f".to_owned(),
            mechanism: "rs-cast".to_owned(),
            novel: false,
            outcome: Outcome::Pass,
            runs: 1,
            tokens: 10,
            probe_sha256: "00".to_owned(),
        }
    }

    #[test]
    fn a_record_round_trips_and_refuses_a_bad_outcome_or_key() {
        assert_eq!(
            ProbeRecord::new(&row(), 1).into_row().expect("valid"),
            row()
        );
        let mut bad = ProbeRecord::new(&row(), 1);
        bad.outcome = "found".to_owned();
        assert!(matches!(bad.into_row(), Err(StoreError::Corrupt(_))));
        let mut moved = ProbeRecord::new(&row(), 1);
        moved.claim = 3;
        assert!(matches!(moved.into_row(), Err(StoreError::Corrupt(_))));
    }

    #[test]
    fn the_key_separates_fields() {
        let a = ProbeRow {
            site: "a::fb".to_owned(),
            mechanism: "c".to_owned(),
            ..row()
        };
        let b = ProbeRow {
            site: "a::f".to_owned(),
            mechanism: "bc".to_owned(),
            ..row()
        };
        assert_ne!(key(&a), key(&b));
    }
}
