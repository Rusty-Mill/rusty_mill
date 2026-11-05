//! The on-disk shape of one BBP task's taint table (#598).
//!
//! Positional bincode under the schema tag below: the v1 layout is the
//! format, and any layout change needs a new tag (see `record.rs`).

use orch_lens::taint::Taints;
use orch_lens::DataClass;
use rusty_multimodal_db_engine::generic::traits::{
    IndexedField, Record, ScannableField, SchemaTag,
};
use serde::{Deserialize, Serialize};

use crate::record::{ById, Revision};
use crate::StoreError;

/// Engine key for a task: a hash of its id, as for meters. The id is
/// stored too, so a collision is refused.
pub fn key(task: &str) -> i64 {
    // A distinct prefix keeps a task named like an experiment apart.
    crate::meters::key(&format!("taint:{task}"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaintRecord {
    id: i64,
    pub revision: i64,
    pub task: String,
    /// `(message id, class word)`.
    entries: Vec<(u64, String)>,
}

impl TaintRecord {
    pub fn new(task: &str, taints: &Taints, revision: u64) -> Self {
        Self {
            id: key(task),
            revision: revision as i64,
            task: task.to_owned(),
            entries: taints
                .entries()
                .into_iter()
                .map(|(m, c)| (m, c.name().to_owned()))
                .collect(),
        }
    }

    /// The table, once its task and class words check out.
    pub fn into_taints(self, task: &str) -> Result<Taints, StoreError> {
        if self.task != task {
            return Err(StoreError::Corrupt(format!(
                "taint key for {task:?} holds {:?}",
                self.task
            )));
        }
        let entries = self
            .entries
            .into_iter()
            .map(|(m, word)| {
                DataClass::parse(&word)
                    .map(|c| (m, c))
                    .ok_or_else(|| StoreError::Corrupt(format!("taint class {word:?}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Taints::from_entries(entries))
    }
}

impl Record for TaintRecord {
    type Id = i64;
    fn id(&self) -> i64 {
        self.id
    }
}

impl SchemaTag for TaintRecord {
    const SCHEMA_TAG: &'static str = "rusty_orch::TaintRecord::v1";
}

impl IndexedField<ById> for TaintRecord {
    type IndexValue = i64;
    fn indexed_value(&self) -> &i64 {
        &self.id
    }
}

impl ScannableField<Revision> for TaintRecord {
    type ScanValue = i64;
    fn scannable_value(&self) -> i64 {
        self.revision
    }
    fn set_scannable_value(&mut self, value: i64) {
        self.revision = value;
    }
}
