//! "Everything changed since version N", built app-side from what the
//! engine has (issue #382, gap 4; `rusty_multimodal_db`'s ADR-0125 records
//! why it stays there): the recipe `rusty_remind_me`'s hub already runs
//! for `hub_seq`.
//!
//! - Every record carries a `seq` the app stamps on each write, issued by
//!   [`Journal::allocate`], which is durable with the next commit and never
//!   goes backwards, even across a checkpoint.
//! - An [`Ordered`] layer keyed by `seq` is the feed: `page_by` after the
//!   client's cursor is `changes_since`, and costs the page.
//! - A delete is a `replace` that sets `deleted` and takes a new `seq`, so
//!   the feed carries it; the record is only removed by a later purge.

use rusty_multimodal_db_engine::generic::query::{GetById, Insert, PageBy, Replace};
use rusty_multimodal_db_engine::generic::store::Ordered;
use rusty_multimodal_db_engine::generic::traits::{
    IndexedField, OrderedField, Record, ScannableField, SchemaTag,
};
use rusty_multimodal_db_engine::generic::GenericMmapStore;
use rusty_multimodal_db_engine::journal::{Batch, Journal};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const SEQ: &str = "change_seq";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Item {
    id: Uuid,
    title: String,
    deleted: bool,
    seq: i64,
}

impl Record for Item {
    type Id = Uuid;
    fn id(&self) -> Uuid {
        self.id
    }
}

impl SchemaTag for Item {
    const SCHEMA_TAG: &'static str = "test::change_feed_recipe::Item";
}

struct ByDeleted;
impl IndexedField<ByDeleted> for Item {
    type IndexValue = bool;
    fn indexed_value(&self) -> &bool {
        &self.deleted
    }
}

struct Seq;
impl ScannableField<Seq> for Item {
    type ScanValue = i64;
    fn scannable_value(&self) -> i64 {
        self.seq
    }
    fn set_scannable_value(&mut self, value: i64) {
        self.seq = value;
    }
}
impl OrderedField<Seq> for Item {
    type Key = i64;
    fn order_key(&self) -> i64 {
        self.seq
    }
}

type Feed = Ordered<GenericMmapStore<Item, ByDeleted, Seq>, Item, Seq>;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

/// Stamp `item` with the next sequence and make the counter durable.
fn stamp(journal: &mut Journal, mut item: Item) -> Item {
    item.seq = i64::try_from(journal.allocate(SEQ)).expect("a sequence fits i64");
    journal.commit(&Batch::default()).unwrap();
    item
}

/// Ids changed after `cursor`, oldest change first: the feed's read.
fn changes_since(feed: &Feed, cursor: i64, limit: usize) -> Vec<Uuid> {
    let after = (cursor, Uuid::from_u128(u128::MAX));
    PageBy::<Item, Seq>::page_by(feed, Some(after), limit)
}

#[test]
fn the_feed_carries_edits_and_deletes_and_survives_a_reopen() {
    let dir = std::env::temp_dir().join(format!(
        "rusty_multimodal_db_engine_feed_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (data, log) = (dir.join("items.mmap"), dir.join("items.journal"));

    let mut journal = Journal::open(&log).unwrap().journal;
    let first = stamp(
        &mut journal,
        Item {
            id: id(1),
            title: "one".into(),
            deleted: false,
            seq: 0,
        },
    );
    let mut feed: Feed = Ordered::new(GenericMmapStore::create(vec![first], &data).unwrap());
    let second = stamp(
        &mut journal,
        Item {
            id: id(2),
            title: "two".into(),
            deleted: false,
            seq: 0,
        },
    );
    feed.insert(second).unwrap();
    assert_eq!(changes_since(&feed, 0, 10), [id(1), id(2)]);

    // A client that has seen up to 2 asks again after an edit and a delete.
    let mut edited = feed.get(id(1)).unwrap();
    edited.title = "uno".into();
    feed.replace(stamp(&mut journal, edited)).unwrap();
    let mut gone = feed.get(id(2)).unwrap();
    gone.deleted = true;
    feed.replace(stamp(&mut journal, gone)).unwrap();
    assert_eq!(changes_since(&feed, 2, 10), [id(1), id(2)]);
    assert!(feed.get(id(2)).unwrap().deleted, "the tombstone is served");
    // Paging is by cursor: one at a time, then nothing.
    assert_eq!(changes_since(&feed, 2, 1), [id(1)]);
    assert_eq!(changes_since(&feed, 3, 10), [id(2)]);
    assert!(changes_since(&feed, 4, 10).is_empty());

    // A checkpoint empties the journal but keeps the counter, so a reopen
    // never issues a sequence a client has already seen.
    journal.checkpoint().unwrap();
    drop((feed, journal));
    let mut journal = Journal::open(&log).unwrap().journal;
    assert_eq!(journal.last(SEQ), 4);
    assert_eq!(journal.allocate(SEQ), 5);
    let feed: Feed = Ordered::new(GenericMmapStore::open_portable(&data).unwrap());
    assert_eq!(changes_since(&feed, 2, 10), [id(1), id(2)]);
    let _ = std::fs::remove_dir_all(dir);
}
