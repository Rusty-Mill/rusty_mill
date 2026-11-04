//! The on-disk shape of a lens experiment's money meter (#594).
//!
//! Positional bincode under the schema tag below: the v1 layout is the
//! format, and any layout change needs a new tag (see `record.rs`).

use std::collections::BTreeSet;

use orch_lens::meter::Snapshot;
use rusty_multimodal_db_engine::durability::record_blob::Fnv1a64;
use rusty_multimodal_db_engine::generic::traits::{
    IndexedField, Record, ScannableField, SchemaTag,
};
use serde::{Deserialize, Serialize};

use crate::record::{ById, Revision};
use crate::StoreError;

/// Engine key for an experiment: a hash of its name. The name is stored
/// too, so a collision is refused instead of read as another meter.
pub fn key(experiment: &str) -> i64 {
    let mut hash = Fnv1a64::new();
    hash.update(experiment.as_bytes());
    hash.finish() as i64
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeterRecord {
    id: i64,
    pub revision: i64,
    pub experiment: String,
    experiment_spent: u64,
    tasks: Vec<(String, u64, u32)>,
    reserved: Vec<(String, u64)>,
}

impl MeterRecord {
    pub fn new(experiment: &str, snapshot: &Snapshot, revision: u64) -> Self {
        Self {
            id: key(experiment),
            revision: revision as i64,
            experiment: experiment.to_owned(),
            experiment_spent: snapshot.experiment_spent,
            tasks: snapshot.tasks.clone(),
            reserved: snapshot.reserved.clone(),
        }
    }

    /// The snapshot, once its name and task list check out.
    pub fn into_snapshot(self, experiment: &str) -> Result<Snapshot, StoreError> {
        if self.experiment != experiment {
            return Err(StoreError::Corrupt(format!(
                "meter key for {experiment:?} holds {:?}",
                self.experiment
            )));
        }
        let mut seen = BTreeSet::new();
        if let Some((t, _, _)) = self.tasks.iter().find(|(t, _, _)| !seen.insert(t)) {
            return Err(StoreError::Corrupt(format!("meter lists task {t:?} twice")));
        }
        Ok(Snapshot {
            experiment_spent: self.experiment_spent,
            tasks: self.tasks,
            reserved: self.reserved,
        })
    }
}

impl Record for MeterRecord {
    type Id = i64;
    fn id(&self) -> i64 {
        self.id
    }
}

impl SchemaTag for MeterRecord {
    const SCHEMA_TAG: &'static str = "rusty_orch::MeterRecord::v1";
}

impl IndexedField<ById> for MeterRecord {
    type IndexValue = i64;
    fn indexed_value(&self) -> &i64 {
        &self.id
    }
}

impl ScannableField<Revision> for MeterRecord {
    type ScanValue = i64;
    fn scannable_value(&self) -> i64 {
        self.revision
    }
    fn set_scannable_value(&mut self, value: i64) {
        self.revision = value;
    }
}
