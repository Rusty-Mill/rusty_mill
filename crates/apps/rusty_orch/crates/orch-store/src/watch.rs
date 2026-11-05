//! The on-disk shape of one watched finding (#599, D58's `watch_finding`):
//! for an app and a finding slot, the key last posted and its claim, and a
//! post written down before it was sent.
//!
//! Positional bincode under the schema tag below: the v1 layout is the
//! format, and any layout change needs a new tag (see `record.rs`).

use rusty_multimodal_db_engine::durability::record_blob::Fnv1a64;
use rusty_multimodal_db_engine::generic::traits::{
    IndexedField, Record, ScannableField, SchemaTag,
};
use serde::{Deserialize, Serialize};

use crate::record::{ById, Revision};
use crate::StoreError;

/// Engine key for an app's slot: a hash of both. They are stored too, so a
/// collision is refused instead of read as another finding.
pub fn key(app: &str, slot: &str) -> i64 {
    let mut hash = Fnv1a64::new();
    hash.update(app.as_bytes());
    hash.update(b"\n");
    hash.update(slot.as_bytes());
    hash.finish() as i64
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchRecord {
    id: i64,
    pub revision: i64,
    pub app: String,
    pub slot: String,
    posted: Option<(String, u64)>,
    pending: Option<(String, String)>,
}

impl WatchRecord {
    pub fn new(app: &str, slot: &str, row: &WatchSlot, revision: u64) -> Self {
        Self {
            id: key(app, slot),
            revision: revision as i64,
            app: app.to_owned(),
            slot: slot.to_owned(),
            posted: row.posted.as_ref().map(|p| (p.key.clone(), p.claim)),
            pending: row
                .pending
                .as_ref()
                .map(|p| (p.key.clone(), p.post.clone())),
        }
    }

    /// Whether this record is the one for `app` and `slot`.
    pub fn check(&self, app: &str, slot: &str) -> Result<(), StoreError> {
        if (self.app.as_str(), self.slot.as_str()) == (app, slot) {
            return Ok(());
        }
        Err(StoreError::Corrupt(format!(
            "watch key for {app:?}/{slot:?} holds {:?}/{:?}",
            self.app, self.slot
        )))
    }

    pub fn into_slot(self, app: &str, slot: &str) -> Result<WatchSlot, StoreError> {
        self.check(app, slot)?;
        Ok(WatchSlot {
            posted: self.posted.map(|(key, claim)| WatchFinding { key, claim }),
            pending: self.pending.map(|(key, post)| PendingPost { key, post }),
        })
    }
}

/// One app's slot: the finding last posted there, and a post sent (or about
/// to be) whose claim is not recorded yet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WatchSlot {
    pub posted: Option<WatchFinding>,
    pub pending: Option<PendingPost>,
}

/// A finding posted in a slot, and the claim it got.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchFinding {
    pub key: String,
    /// The BBP claim's message id.
    pub claim: u64,
}

/// A post written down before it is sent, so a later run can resend it
/// byte for byte (BBP replays it) and learn its claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingPost {
    pub key: String,
    /// The exact `post` arguments, as JSON.
    pub post: String,
}

impl Record for WatchRecord {
    type Id = i64;
    fn id(&self) -> i64 {
        self.id
    }
}

impl SchemaTag for WatchRecord {
    const SCHEMA_TAG: &'static str = "rusty_orch::WatchRecord::v1";
}

impl IndexedField<ById> for WatchRecord {
    type IndexValue = i64;
    fn indexed_value(&self) -> &i64 {
        &self.id
    }
}

impl ScannableField<Revision> for WatchRecord {
    type ScanValue = i64;
    fn scannable_value(&self) -> i64 {
        self.revision
    }
    fn set_scannable_value(&mut self, value: i64) {
        self.revision = value;
    }
}
