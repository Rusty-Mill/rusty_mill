//! A durable table of one record type on the engine: the open/insert/replace/
//! delete/get boilerplate that lists, tags and docs would otherwise each repeat.

use crate::store::TickError;
use rusty_multimodal_db_engine::generic::mmap_field::MmapFieldValue;
use rusty_multimodal_db_engine::generic::query::{AllIds, FilterEq, GetById};
use rusty_multimodal_db_engine::generic::traits::{
    IndexedField, Record, ScannableField, SchemaTag,
};
use rusty_multimodal_db_engine::generic::{
    DeleteError, GenericMmapStore, InsertError, ReplaceError,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::path::Path;
use uuid::Uuid;

pub struct Table<R, Index, Slot>
where
    R: Record<Id = Uuid>
        + IndexedField<Index>
        + ScannableField<Slot>
        + Clone
        + Serialize
        + DeserializeOwned,
    R::ScanValue: MmapFieldValue,
{
    core: GenericMmapStore<R, Index, Slot>,
}

impl<R, Index, Slot> Table<R, Index, Slot>
where
    R: Record<Id = Uuid>
        + IndexedField<Index>
        + ScannableField<Slot>
        + Clone
        + Serialize
        + DeserializeOwned
        + SchemaTag,
    R::ScanValue: MmapFieldValue,
{
    /// Open `dir/file`, creating an empty table when absent.
    pub fn open(dir: &Path, file: &str) -> Result<Self, TickError> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(file);
        let core = if path.exists() {
            GenericMmapStore::open_portable(&path)?
        } else {
            GenericMmapStore::create(Vec::new(), &path)?
        };
        Ok(Self { core })
    }

    pub fn insert(&mut self, record: R) -> Result<(), TickError> {
        let id = record.id();
        match self.core.insert(record) {
            Ok(()) => Ok(()),
            Err(InsertError::Duplicate(_)) => Err(TickError::Duplicate(id)),
            Err(InsertError::Durability(e)) => Err(TickError::Storage(e)),
        }
    }

    pub fn replace(&mut self, record: R) -> Result<(), TickError> {
        let id = record.id();
        match self.core.replace(record) {
            Ok(()) => Ok(()),
            Err(ReplaceError::NotFound(_)) => Err(TickError::NotFound(id)),
            Err(ReplaceError::Durability(e)) => Err(TickError::Storage(e)),
        }
    }

    pub fn delete(&mut self, id: Uuid) -> Result<(), TickError> {
        match self.core.delete(id) {
            Ok(()) => Ok(()),
            Err(DeleteError::NotFound(_)) => Err(TickError::NotFound(id)),
            Err(DeleteError::Durability(e)) => Err(TickError::Storage(e)),
        }
    }

    pub fn get(&self, id: Uuid) -> Option<R> {
        self.core.get(id)
    }

    /// Every record, unspecified order.
    pub fn all(&self) -> Vec<R> {
        self.core
            .all_ids()
            .into_iter()
            .filter_map(|id| self.core.get(id))
            .collect()
    }

    /// Records whose indexed field equals `value`.
    pub fn with_index(&self, value: &R::IndexValue) -> Vec<R> {
        FilterEq::<R, Index>::filter_eq(&self.core, value)
            .into_iter()
            .filter_map(|id| self.core.get(id))
            .collect()
    }
}
