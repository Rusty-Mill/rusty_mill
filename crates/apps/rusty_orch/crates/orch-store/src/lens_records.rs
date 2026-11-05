//! The on-disk shape of one blind review job for a lens's track record
//! (#599, D58's `lens_record`, D89): what the job posted. Points are not
//! stored; they are computed from these rows whenever the record is read.
//!
//! Positional bincode under the schema tag below: the v1 layout is the
//! format, and any layout change needs a new tag (see `record.rs`).

use orch_lens::record::{MsgId, Review};
use rusty_multimodal_db_engine::generic::traits::{
    IndexedField, Record, ScannableField, SchemaTag,
};
use serde::{Deserialize, Serialize};

use crate::probes::hash_parts;
use crate::record::{ById, Revision};
use crate::StoreError;

/// Engine key for a job: its board, lens, claim, epoch and kind. Asking the
/// same job again (after a refused reply) replaces its row.
pub fn key(board: &str, row: &Review) -> i64 {
    hash_parts(&[
        board.as_bytes(),
        row.lens.as_bytes(),
        &row.claim.0.to_le_bytes(),
        &row.epoch.to_le_bytes(),
        &[u8::from(row.look_harder)],
    ])
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LensRecordRecord {
    id: i64,
    pub revision: i64,
    board: String,
    lens: String,
    claim: u64,
    epoch: u32,
    look_harder: bool,
    items: Vec<u64>,
}

impl LensRecordRecord {
    pub fn new(board: &str, row: &Review, revision: u64) -> Self {
        Self {
            id: key(board, row),
            revision: revision as i64,
            board: board.to_owned(),
            lens: row.lens.clone(),
            claim: row.claim.0,
            epoch: row.epoch,
            look_harder: row.look_harder,
            items: row.items.iter().map(|m| m.0).collect(),
        }
    }

    /// Does this record hold the job `row` names on `board`?
    pub fn is(&self, board: &str, row: &Review) -> bool {
        (
            self.board.as_str(),
            self.lens.as_str(),
            self.claim,
            self.epoch,
            self.look_harder,
        ) == (
            board,
            row.lens.as_str(),
            row.claim.0,
            row.epoch,
            row.look_harder,
        )
    }

    pub fn board(&self) -> &str {
        &self.board
    }

    /// The row, once its key checks out.
    pub fn into_row(self) -> Result<Review, StoreError> {
        let row = Review {
            lens: self.lens,
            claim: MsgId(self.claim),
            epoch: self.epoch,
            look_harder: self.look_harder,
            items: self.items.into_iter().map(MsgId).collect(),
        };
        if key(&self.board, &row) != self.id {
            return Err(StoreError::Corrupt(format!(
                "lens record key {} does not match its fields",
                self.id
            )));
        }
        Ok(row)
    }
}

impl Record for LensRecordRecord {
    type Id = i64;
    fn id(&self) -> i64 {
        self.id
    }
}

impl SchemaTag for LensRecordRecord {
    const SCHEMA_TAG: &'static str = "rusty_orch::LensRecordRecord::v1";
}

impl IndexedField<ById> for LensRecordRecord {
    type IndexValue = i64;
    fn indexed_value(&self) -> &i64 {
        &self.id
    }
}

impl ScannableField<Revision> for LensRecordRecord {
    type ScanValue = i64;
    fn scannable_value(&self) -> i64 {
        self.revision
    }
    fn set_scannable_value(&mut self, value: i64) {
        self.revision = value;
    }
}
