//! The on-disk shape of one completed alien-lens hunt (#708), saved
//! whether or not it produced a probe.
//!
//! Positional bincode under the schema tag below: the v1 layout is the
//! format, and any layout change needs a new tag (see `record.rs`).

use orch_lens::alien::HuntRow;
use rusty_multimodal_db_engine::generic::traits::{
    IndexedField, Record, ScannableField, SchemaTag,
};
use serde::{Deserialize, Serialize};

use crate::probes::hash_parts;
use crate::record::{ById, Revision};
use crate::StoreError;

/// Engine key for a hunt: its task, claim and candidate.
pub fn key(row: &HuntRow) -> i64 {
    hash_parts(&[
        row.task.as_bytes(),
        &row.claim.to_le_bytes(),
        &row.candidate.to_le_bytes(),
    ])
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HuntRecord {
    id: i64,
    pub revision: i64,
    task: String,
    claim: u64,
    candidate: u64,
    suspects: u32,
    probes: u32,
    tokens: u64,
    cost: u64,
}

impl HuntRecord {
    pub fn new(row: &HuntRow, revision: u64) -> Self {
        Self {
            id: key(row),
            revision: revision as i64,
            task: row.task.clone(),
            claim: row.claim,
            candidate: row.candidate,
            suspects: row.suspects,
            probes: row.probes,
            tokens: row.tokens,
            cost: row.cost,
        }
    }

    /// Does this record hold the hunt `row` names?
    pub fn is(&self, row: &HuntRow) -> bool {
        self.task == row.task && self.claim == row.claim && self.candidate == row.candidate
    }

    /// The row, once its key checks out.
    pub fn into_row(self) -> Result<HuntRow, StoreError> {
        let row = HuntRow {
            task: self.task,
            claim: self.claim,
            candidate: self.candidate,
            suspects: self.suspects,
            probes: self.probes,
            tokens: self.tokens,
            cost: self.cost,
        };
        if key(&row) != self.id {
            return Err(StoreError::Corrupt(format!(
                "hunt key {} does not match its fields",
                self.id
            )));
        }
        Ok(row)
    }
}

impl Record for HuntRecord {
    type Id = i64;
    fn id(&self) -> i64 {
        self.id
    }
}

impl SchemaTag for HuntRecord {
    const SCHEMA_TAG: &'static str = "rusty_orch::HuntRecord::v1";
}

impl IndexedField<ById> for HuntRecord {
    type IndexValue = i64;
    fn indexed_value(&self) -> &i64 {
        &self.id
    }
}

impl ScannableField<Revision> for HuntRecord {
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

    #[test]
    fn a_record_round_trips_and_refuses_a_moved_key() {
        let row = HuntRow {
            task: "t".to_owned(),
            claim: 1,
            candidate: 2,
            suspects: 0,
            probes: 0,
            tokens: 5,
            cost: 0,
        };
        assert_eq!(HuntRecord::new(&row, 1).into_row().expect("valid"), row);
        let mut moved = HuntRecord::new(&row, 1);
        moved.candidate = 3;
        assert!(matches!(moved.into_row(), Err(StoreError::Corrupt(_))));
    }
}
