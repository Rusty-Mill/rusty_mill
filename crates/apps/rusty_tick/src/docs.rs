//! Client-owned JSON documents (habits, check-ins, focus records,
//! preferences). The server validates size and that the body is JSON, and
//! stores it; it does not interpret it. Durable on the engine like everything
//! else, so nothing the UI keeps lives only in the browser.

use crate::store::TickError;
use crate::table::Table;
use rusty_multimodal_db_engine::generic::traits::{
    IndexedField, Record, ScannableField, SchemaTag,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

/// The document kinds the API accepts.
pub const KINDS: [&str; 5] = [
    "habit",
    "habit_checkin",
    "focus",
    "prefs",
    "summary_template",
];
/// Largest accepted body, in bytes.
pub const MAX_DOC_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Doc {
    pub id: Uuid,
    pub kind: String,
    /// A JSON document, as text.
    pub body: String,
    pub updated_ms: i64,
}

impl Record for Doc {
    type Id = Uuid;
    fn id(&self) -> Uuid {
        self.id
    }
}

impl SchemaTag for Doc {
    const SCHEMA_TAG: &'static str = "rusty_tick::Doc@1";
}

/// Index marker: documents by kind.
pub struct ByKind;
impl IndexedField<ByKind> for Doc {
    type IndexValue = String;
    fn indexed_value(&self) -> &String {
        &self.kind
    }
}

pub struct UpdatedAt;
impl ScannableField<UpdatedAt> for Doc {
    type ScanValue = i64;
    fn scannable_value(&self) -> i64 {
        self.updated_ms
    }
    fn set_scannable_value(&mut self, value: i64) {
        self.updated_ms = value;
    }
}

pub struct DocStore {
    table: Table<Doc, ByKind, UpdatedAt>,
}

impl DocStore {
    pub fn open(dir: &Path) -> Result<Self, TickError> {
        Ok(Self {
            table: Table::open(dir, "docs.mmap")?,
        })
    }

    pub fn get(&self, id: Uuid) -> Option<Doc> {
        self.table.get(id)
    }

    /// Insert or replace.
    pub fn put(&mut self, doc: Doc) -> Result<(), TickError> {
        if self.table.get(doc.id).is_some() {
            self.table.replace(doc)
        } else {
            self.table.insert(doc)
        }
    }

    pub fn delete(&mut self, id: Uuid) -> Result<(), TickError> {
        self.table.delete(id)
    }

    /// Documents of `kind`, oldest first.
    pub fn of_kind(&self, kind: &str) -> Vec<Doc> {
        let mut docs = self.table.with_index(&kind.to_string());
        docs.sort_by_key(|d| (d.updated_ms, d.id));
        docs
    }
}
