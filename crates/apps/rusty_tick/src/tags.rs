//! Tags as entities (name, colour, parent), so a tag exists and keeps its
//! colour independent of which tasks carry it. Tasks refer to tags by `name`.

use crate::store::TickError;
use crate::table::Table;
use rusty_multimodal_db_engine::generic::traits::{
    IndexedField, Record, ScannableField, SchemaTag,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

/// Tags are matched case-insensitively: `name` is the lowercased `label`.
pub fn tag_name(label: &str) -> String {
    label.trim().to_lowercase()
}

/// A tag's engine id, derived from its name so lookup needs no index scan.
pub fn tag_id(name: &str) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("rusty_tick/tag/{name}").as_bytes(),
    )
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tag {
    pub name: String,
    pub label: String,
    pub color: Option<String>,
    /// The `name` of the parent tag, for nested tags.
    pub parent: Option<String>,
    pub sort_order: i64,
    pub version: u32,
}

impl Tag {
    pub fn new(label: &str, sort_order: i64) -> Self {
        let label = label.trim().to_string();
        Self {
            name: tag_name(&label),
            label,
            color: None,
            parent: None,
            sort_order,
            version: 1,
        }
    }
}

impl Record for Tag {
    type Id = Uuid;
    fn id(&self) -> Uuid {
        tag_id(&self.name)
    }
}

impl SchemaTag for Tag {
    const SCHEMA_TAG: &'static str = "rusty_tick::Tag@1";
}

/// Index marker: tags by name. The engine wants one indexed field per
/// table; lookups go by id ([`tag_id`]) and the set is small, so the name
/// is the honest choice.
pub struct ByName;
impl IndexedField<ByName> for Tag {
    type IndexValue = String;
    fn indexed_value(&self) -> &String {
        &self.name
    }
}

pub struct TagOrder;
impl ScannableField<TagOrder> for Tag {
    type ScanValue = i64;
    fn scannable_value(&self) -> i64 {
        self.sort_order
    }
    fn set_scannable_value(&mut self, value: i64) {
        self.sort_order = value;
    }
}

pub struct TagStore {
    table: Table<Tag, ByName, TagOrder>,
}

impl TagStore {
    pub fn open(dir: &Path) -> Result<Self, TickError> {
        Ok(Self {
            table: Table::open(dir, "tags.mmap")?,
        })
    }

    pub fn insert(&mut self, tag: Tag) -> Result<(), TickError> {
        self.table.insert(tag)
    }

    pub fn replace(&mut self, tag: Tag) -> Result<(), TickError> {
        self.table.replace(tag)
    }

    pub fn delete(&mut self, name: &str) -> Result<(), TickError> {
        self.table.delete(tag_id(name))
    }

    pub fn get(&self, name: &str) -> Option<Tag> {
        self.table.get(tag_id(name))
    }

    /// Every tag in manual order (ties by name).
    pub fn all(&self) -> Vec<Tag> {
        let mut tags = self.table.all();
        tags.sort_by(|a, b| (a.sort_order, &a.name).cmp(&(b.sort_order, &b.name)));
        tags
    }
}
