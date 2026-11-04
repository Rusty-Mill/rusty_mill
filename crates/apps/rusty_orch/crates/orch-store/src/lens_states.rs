//! The on-disk shape of one BBP board's lens scheduler memory (#594).
//!
//! A board is a task in one BBP store directory: the caller names it, so
//! two tasks of the same name in different directories never share a
//! record.
//!
//! Positional bincode under the schema tag below: the v1 layout is the
//! format, and any layout change needs a new tag (see `record.rs`). The
//! provider ids are re-checked when `orch_lens::schedule::State::restore`
//! rebuilds the memory.

use orch_lens::saved::{Saved, SavedPass, SavedProgress, SavedQuestion};
use rusty_multimodal_db_engine::durability::record_blob::Fnv1a64;
use rusty_multimodal_db_engine::generic::traits::{
    IndexedField, Record, ScannableField, SchemaTag,
};
use serde::{Deserialize, Serialize};

use crate::record::{ById, Revision};
use crate::StoreError;

/// Engine key for a board: a hash of its identity. The identity is stored
/// too, so a collision is refused instead of read as another board's memory.
pub fn key(board: &str) -> i64 {
    let mut hash = Fnv1a64::new();
    hash.update(board.as_bytes());
    hash.finish() as i64
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PassRow {
    provider: String,
    posted: u64,
    found_counter: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProgressRow {
    lens: String,
    claim: u64,
    blind: Option<PassRow>,
    look_harder: Option<PassRow>,
    foreign_seen: u64,
    epoch: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QuestionRow {
    claim: u64,
    used: Vec<String>,
    settled: Vec<u8>,
    tried: Vec<(Option<u8>, Vec<String>)>,
    rebel_landed: bool,
    rebel_failures: u32,
    attacks: Vec<(u64, u32)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LensStateRecord {
    id: i64,
    pub revision: i64,
    pub board: String,
    progress: Vec<ProgressRow>,
    questions: Vec<QuestionRow>,
}

impl LensStateRecord {
    pub fn new(board: &str, saved: &Saved, revision: u64) -> Self {
        Self {
            id: key(board),
            revision: revision as i64,
            board: board.to_owned(),
            progress: saved.progress.iter().map(progress_row).collect(),
            questions: saved.questions.iter().map(question_row).collect(),
        }
    }

    /// The saved memory, once the record's board checks out.
    pub fn into_saved(self, board: &str) -> Result<Saved, StoreError> {
        if self.board != board {
            return Err(StoreError::Corrupt(format!(
                "lens state key for {board:?} holds {:?}",
                self.board
            )));
        }
        Ok(Saved {
            progress: self.progress.into_iter().map(progress).collect(),
            questions: self.questions.into_iter().map(question).collect(),
        })
    }
}

fn pass_row(p: &SavedPass) -> PassRow {
    PassRow {
        provider: p.provider.clone(),
        posted: p.posted,
        found_counter: p.found_counter,
    }
}

fn pass(p: PassRow) -> SavedPass {
    SavedPass {
        provider: p.provider,
        posted: p.posted,
        found_counter: p.found_counter,
    }
}

fn progress_row(p: &SavedProgress) -> ProgressRow {
    ProgressRow {
        lens: p.lens.clone(),
        claim: p.claim,
        blind: p.blind.as_ref().map(pass_row),
        look_harder: p.look_harder.as_ref().map(pass_row),
        foreign_seen: p.foreign_seen,
        epoch: p.epoch,
    }
}

fn progress(p: ProgressRow) -> SavedProgress {
    SavedProgress {
        lens: p.lens,
        claim: p.claim,
        blind: p.blind.map(pass),
        look_harder: p.look_harder.map(pass),
        foreign_seen: p.foreign_seen,
        epoch: p.epoch,
    }
}

fn question_row(q: &SavedQuestion) -> QuestionRow {
    QuestionRow {
        claim: q.claim,
        used: q.used.clone(),
        settled: q.settled.clone(),
        tried: q.tried.clone(),
        rebel_landed: q.rebel_landed,
        rebel_failures: q.rebel_failures,
        attacks: q.attacks.clone(),
    }
}

fn question(q: QuestionRow) -> SavedQuestion {
    SavedQuestion {
        claim: q.claim,
        used: q.used,
        settled: q.settled,
        tried: q.tried,
        rebel_landed: q.rebel_landed,
        rebel_failures: q.rebel_failures,
        attacks: q.attacks,
    }
}

impl Record for LensStateRecord {
    type Id = i64;
    fn id(&self) -> i64 {
        self.id
    }
}

impl SchemaTag for LensStateRecord {
    const SCHEMA_TAG: &'static str = "rusty_orch::LensStateRecord::v1";
}

impl IndexedField<ById> for LensStateRecord {
    type IndexValue = i64;
    fn indexed_value(&self) -> &i64 {
        &self.id
    }
}

impl ScannableField<Revision> for LensStateRecord {
    type ScanValue = i64;
    fn scannable_value(&self) -> i64 {
        self.revision
    }
    fn set_scannable_value(&mut self, value: i64) {
        self.revision = value;
    }
}
