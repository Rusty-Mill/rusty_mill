//! A journaled batch across two engine stores lands in both or neither,
//! whatever point a crash interrupts it at (`rusty_remind_me`'s ADR-0023
//! §3b): the discipline a node applies a memory and its outbox entry with.
//!
//! A "crash" here is dropping every handle part-way: the journal entry and
//! whatever the stores had synced are all that survive, as after a real
//! one, since each store write and each commit is `fsync`'d before it
//! returns.

use rusty_multimodal_db_engine::codec;
use rusty_multimodal_db_engine::generic::query::{Delete, GetById, Insert, PageBy, Replace};
use rusty_multimodal_db_engine::generic::store::{GroupCommit, Ordered};
use rusty_multimodal_db_engine::generic::traits::{
    IndexedField, OrderedField, Record, ScannableField, SchemaTag,
};
use rusty_multimodal_db_engine::generic::GenericMmapStore;
use rusty_multimodal_db_engine::journal::{Batch, Journal};
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
    const SCHEMA_TAG: &'static str = "tests::journal_batches::Row";
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

type Table = Ordered<GenericMmapStore<Row, Body, Seq>, Row, Seq>;

const MEMORIES: &str = "memories";
const OUTBOX: &str = "outbox";

/// A node's two stores and its journal, in one directory.
struct Node {
    memories: Table,
    outbox: Table,
    journal: Journal,
}

fn open_table(path: &Path) -> Table {
    let core = if path.exists() {
        GenericMmapStore::open_portable(path).unwrap()
    } else {
        GenericMmapStore::create(Vec::new(), path).unwrap()
    };
    Ordered::new(core)
}

impl Node {
    /// Open everything and replay what the journal holds, as a node's
    /// store does at start.
    fn open(dir: &Path) -> Self {
        let opened = Journal::open(&dir.join("node.journal")).unwrap();
        let mut node = Node {
            memories: open_table(&dir.join("memories.mmap")),
            outbox: open_table(&dir.join("outbox.mmap")),
            journal: opened.journal,
        };
        for batch in &opened.replay {
            node.apply(batch, usize::MAX);
        }
        node.journal.checkpoint().unwrap();
        node
    }

    fn table(&mut self, store: &str) -> &mut Table {
        match store {
            MEMORIES => &mut self.memories,
            OUTBOX => &mut self.outbox,
            other => panic!("no store {other}"),
        }
    }

    /// Apply the first `limit` changes of `batch`: a whole value is put
    /// over whatever is there, so applying a change twice is harmless.
    fn apply(&mut self, batch: &Batch, limit: usize) {
        for change in batch.changes.iter().take(limit) {
            let id = Uuid::from_slice(&change.key).unwrap();
            let table = self.table(&change.store);
            let exists = table.get(id).is_some();
            match &change.value {
                Some(bytes) => {
                    let row: Row = codec::decode(bytes).unwrap();
                    if exists {
                        table.replace(row).unwrap();
                    } else {
                        table.insert(row).unwrap();
                    }
                }
                None if exists => {
                    table.delete(id).unwrap();
                }
                None => {}
            }
        }
    }

    /// A memory and the outbox entry announcing it, as one batch.
    fn write_memory(&mut self, n: u128, body: &str) -> Batch {
        let seq = self.journal.allocate(OUTBOX) as i64;
        let memory = Row {
            id: Uuid::from_u128(n),
            seq: 0,
            body: body.to_string(),
        };
        let entry = Row {
            id: Uuid::from_u128(1_000 + n),
            seq,
            body: format!("upsert {n}"),
        };
        let mut batch = Batch::default();
        batch
            .put(
                MEMORIES,
                memory.id.as_bytes().to_vec(),
                codec::encode(&memory).unwrap(),
            )
            .put(
                OUTBOX,
                entry.id.as_bytes().to_vec(),
                codec::encode(&entry).unwrap(),
            );
        batch
    }

    fn outbox_seqs(&self) -> Vec<i64> {
        self.outbox
            .page_by(None, usize::MAX)
            .into_iter()
            .map(|id| self.outbox.get(id).unwrap().seq)
            .collect()
    }

    fn has_memory(&self, n: u128) -> bool {
        self.memories.get(Uuid::from_u128(n)).is_some()
    }
}

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rusty_multimodal_db_engine_journal_{label}_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_crash_between_the_two_stores_is_completed_on_reopen() {
    let dir = temp_dir("between");
    let mut node = Node::open(&dir);
    let batch = node.write_memory(1, "the first memory");
    node.journal.commit(&batch).unwrap();
    // The memory lands; the process dies before the outbox entry does.
    node.apply(&batch, 1);
    assert!(node.has_memory(1));
    assert!(node.outbox_seqs().is_empty());
    drop(node);

    let node = Node::open(&dir);
    assert!(node.has_memory(1));
    assert_eq!(node.outbox_seqs(), vec![1], "the outbox entry came with it");
}

#[test]
fn a_crash_before_the_journal_commit_leaves_neither() {
    let dir = temp_dir("before");
    let mut node = Node::open(&dir);
    let _never_committed = node.write_memory(1, "lost before commit");
    drop(node);

    let node = Node::open(&dir);
    assert!(!node.has_memory(1));
    assert!(node.outbox_seqs().is_empty());
}

#[test]
fn replaying_a_batch_that_already_landed_changes_nothing() {
    let dir = temp_dir("twice");
    let mut node = Node::open(&dir);
    let first = node.write_memory(1, "one");
    node.journal.commit(&first).unwrap();
    node.apply(&first, usize::MAX);
    let second = node.write_memory(2, "two");
    node.journal.commit(&second).unwrap();
    node.apply(&second, usize::MAX);
    // No checkpoint: both batches replay over stores that already hold them.
    drop(node);

    let node = Node::open(&dir);
    assert!(node.has_memory(1) && node.has_memory(2));
    assert_eq!(node.outbox_seqs(), vec![1, 2]);
}

#[test]
fn outbox_ids_are_never_issued_twice_across_crashes_and_checkpoints() {
    let dir = temp_dir("seq");
    let mut node = Node::open(&dir);
    for n in 1..=3 {
        let batch = node.write_memory(n, "x");
        node.journal.commit(&batch).unwrap();
        node.apply(&batch, usize::MAX);
    }
    node.memories.commit().unwrap();
    node.outbox.commit().unwrap();
    node.journal.checkpoint().unwrap();
    drop(node);

    let mut node = Node::open(&dir);
    let batch = node.write_memory(4, "after a checkpoint and a reopen");
    node.journal.commit(&batch).unwrap();
    node.apply(&batch, usize::MAX);
    assert_eq!(node.outbox_seqs(), vec![1, 2, 3, 4]);
}
