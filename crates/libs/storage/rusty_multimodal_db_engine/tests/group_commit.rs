//! `GroupCommit`: a batch of writes under one sync of the insert log, the
//! engine API `rusty_remind_me`'s hub applies a push with (its ADR-0021).
//!
//! What a batch must keep from sync-per-write:
//!
//! - every layer sees each write at once, inside the batch;
//! - after `commit`, a reopen folds exactly what the batch wrote, inserts,
//!   replaces and deletes in order;
//! - a compaction inside a batch leaves nothing for `commit` to sync.

use rusty_multimodal_db_engine::generic::query::{
    Compact, Delete, GetById, Insert, PageBy, Replace,
};
use rusty_multimodal_db_engine::generic::store::{GroupCommit, Ordered};
use rusty_multimodal_db_engine::generic::traits::{
    IndexedField, OrderedField, Record, ScannableField, SchemaTag,
};
use rusty_multimodal_db_engine::generic::GenericMmapStore;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Row {
    id: Uuid,
    seq: i64,
    body: String,
}

impl Record for Row {
    type Id = Uuid;
    fn id(&self) -> Uuid {
        self.id
    }
}

impl SchemaTag for Row {
    const SCHEMA_TAG: &'static str = "tests::group_commit::Row";
}

struct Body;
impl IndexedField<Body> for Row {
    type IndexValue = String;
    fn indexed_value(&self) -> &String {
        &self.body
    }
}

struct Seq;
impl ScannableField<Seq> for Row {
    type ScanValue = i64;
    fn scannable_value(&self) -> i64 {
        self.seq
    }
    fn set_scannable_value(&mut self, value: i64) {
        self.seq = value;
    }
}
impl OrderedField<Seq> for Row {
    type Key = i64;
    fn order_key(&self) -> i64 {
        self.seq
    }
}

type Core = GenericMmapStore<Row, Body, Seq>;
type Table = Ordered<Core, Row, Seq>;

fn row(n: u128, seq: i64, body: &str) -> Row {
    Row {
        id: Uuid::from_u128(n),
        seq,
        body: body.to_string(),
    }
}

fn temp_path(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rusty_multimodal_db_engine_group_commit_{label}_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("table.mmap")
}

fn log_of(path: &Path) -> PathBuf {
    let mut log = path.as_os_str().to_owned();
    log.push(".inserts");
    PathBuf::from(log)
}

fn reopen(path: &Path) -> Table {
    Ordered::new(Core::open_portable(path).unwrap())
}

/// Every row, in `seq` order.
fn rows(table: &Table) -> Vec<Row> {
    table
        .page_by(None, usize::MAX)
        .into_iter()
        .map(|id| table.get(id).unwrap())
        .collect()
}

#[test]
fn a_committed_batch_is_seen_at_once_and_folds_on_reopen() {
    let path = temp_path("batch");
    let mut table: Table = Ordered::new(Core::create(Vec::new(), &path).unwrap());
    table.insert(row(1, 1, "before")).unwrap();

    table.defer_sync();
    table.insert(row(2, 2, "two")).unwrap();
    table.insert(row(3, 3, "three")).unwrap();
    table.replace(row(1, 4, "one, replaced")).unwrap();
    table.delete(Uuid::from_u128(2)).unwrap();
    table.insert(row(2, 5, "two again")).unwrap();
    let inside = rows(&table);
    assert_eq!(
        inside,
        vec![
            row(3, 3, "three"),
            row(1, 4, "one, replaced"),
            row(2, 5, "two again"),
        ],
        "reads inside the batch see every write, in order"
    );
    assert!(table.inner().is_sync_deferred());
    table.commit().unwrap();
    assert!(!table.inner().is_sync_deferred());

    // A write after the commit is synced on its own again, and lands too.
    table.insert(row(9, 9, "after")).unwrap();
    let mut expected = inside;
    expected.push(row(9, 9, "after"));
    drop(table);
    assert_eq!(rows(&reopen(&path)), expected);
}

#[test]
fn a_commit_with_nothing_written_creates_no_log() {
    let path = temp_path("empty");
    let mut table: Table = Ordered::new(Core::create(Vec::new(), &path).unwrap());
    table.defer_sync();
    table.commit().unwrap();
    assert!(!log_of(&path).exists());
}

#[test]
fn a_compaction_inside_a_batch_leaves_nothing_to_sync() {
    let path = temp_path("compact");
    let mut table: Table = Ordered::new(Core::create(Vec::new(), &path).unwrap());
    table.defer_sync();
    table.insert(row(1, 1, "one")).unwrap();
    table.insert(row(2, 2, "two")).unwrap();
    table.compact().unwrap();
    assert!(!log_of(&path).exists(), "the compaction folded the log");
    table.commit().unwrap();
    assert!(!log_of(&path).exists(), "commit had nothing to sync");
    drop(table);
    assert_eq!(
        rows(&reopen(&path)),
        vec![row(1, 1, "one"), row(2, 2, "two")]
    );
}
