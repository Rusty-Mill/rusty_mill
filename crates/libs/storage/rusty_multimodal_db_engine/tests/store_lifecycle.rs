//! Opening and closing one store per user (issue #382, gap 6): what a
//! bounded pool of open stores can rely on.
//!
//! - A store is closed by dropping it, together with its [`DirLock`]. Every
//!   write was durable when it returned, so nothing is lost by closing at
//!   any point, and a reopen sees exactly what the store held.
//! - The lock is per open file description, so it also refuses a second
//!   handle *in the same process*: an idle store can be closed safely
//!   because no other handle to that directory can be open. The store
//!   itself does not lock (`GenericMmapStore::open` on a directory already
//!   open would happily map it twice), so the lock is what makes eviction
//!   sound and must be taken first.
//! - Open cost is the derived indexes' rebuild and idle memory is the
//!   records plus those indexes, both linear in the records; the ignored
//!   probe prints both.

use rusty_multimodal_db_engine::dir_lock::{DirLock, DirLockError};
use rusty_multimodal_db_engine::generic::query::{GetById, Insert, PageBy, Replace};
use rusty_multimodal_db_engine::generic::store::Ordered;
use rusty_multimodal_db_engine::generic::traits::{
    IndexedField, OrderedField, Record, ScannableField, SchemaTag,
};
use rusty_multimodal_db_engine::generic::GenericMmapStore;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Instant;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Note {
    id: Uuid,
    owner: u8,
    order: i64,
    text: String,
}

impl Record for Note {
    type Id = Uuid;
    fn id(&self) -> Uuid {
        self.id
    }
}

impl SchemaTag for Note {
    const SCHEMA_TAG: &'static str = "test::store_lifecycle::Note";
}

struct ByOwner;
impl IndexedField<ByOwner> for Note {
    type IndexValue = u8;
    fn indexed_value(&self) -> &u8 {
        &self.owner
    }
}

struct Order;
impl ScannableField<Order> for Note {
    type ScanValue = i64;
    fn scannable_value(&self) -> i64 {
        self.order
    }
    fn set_scannable_value(&mut self, value: i64) {
        self.order = value;
    }
}
impl OrderedField<Order> for Note {
    type Key = i64;
    fn order_key(&self) -> i64 {
        self.order
    }
}

type Open = Ordered<GenericMmapStore<Note, ByOwner, Order>, Note, Order>;

/// One user's store: the directory lock, then the data. Field order is the
/// drop order, so the data closes before the lock is released.
struct UserStore {
    store: Open,
    _lock: DirLock,
}

fn open(dir: &Path) -> Result<UserStore, DirLockError> {
    let lock = DirLock::acquire(dir, "store.lock")?;
    let data = dir.join("notes.mmap");
    let core = if data.exists() {
        GenericMmapStore::open_portable(&data).expect("an existing store opens")
    } else {
        GenericMmapStore::create(Vec::new(), &data).expect("a new store is created")
    };
    Ok(UserStore {
        store: Ordered::new(core),
        _lock: lock,
    })
}

fn note(n: u128, order: i64) -> Note {
    Note {
        id: Uuid::from_u128(n),
        owner: 0,
        order,
        text: format!("note {n}"),
    }
}

fn fresh_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rusty_multimodal_db_engine_{label}_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn closing_loses_nothing_and_a_reopen_sees_every_write() {
    let root = fresh_dir("lifecycle_close");
    let dir = root.join("alice");
    let mut user = open(&dir).unwrap();
    for n in 1..=50u128 {
        user.store.insert(note(n, n as i64)).unwrap();
    }
    let mut edited = user.store.get(Uuid::from_u128(7)).unwrap();
    edited.text = "edited".into();
    user.store.replace(edited).unwrap();
    drop(user);

    let reopened = open(&dir).unwrap();
    assert_eq!(
        reopened.store.get(Uuid::from_u128(7)).unwrap().text,
        "edited"
    );
    let page = PageBy::<Note, Order>::page_by(&reopened.store, None, 100);
    assert_eq!(page.len(), 50);
    assert_eq!(page[0], Uuid::from_u128(1), "the order index rebuilds");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_second_handle_is_refused_in_process_until_the_first_is_closed() {
    let root = fresh_dir("lifecycle_lock");
    let dir = root.join("bob");
    let first = open(&dir).unwrap();
    assert!(matches!(open(&dir), Err(DirLockError::Held(_))));
    // Other users' directories are independent.
    let other = open(&root.join("carol")).unwrap();
    drop(first);
    let again = open(&dir).unwrap();
    drop((other, again));
    let _ = std::fs::remove_dir_all(root);
}

/// The resident set in bytes, from `/proc/self/statm` (Linux only).
fn resident_bytes() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages * 4096)
}

/// `cargo test -p rusty_multimodal_db_engine --release --test store_lifecycle
/// -- --ignored --nocapture`: open and close cost, and idle memory, per store.
#[test]
#[ignore = "a measurement, not a check"]
fn probe_open_close_cost_and_idle_memory() {
    const STORES: usize = 20;
    for records in [0usize, 100, 1_000, 10_000] {
        let root = fresh_dir("lifecycle_probe");
        for user in 0..STORES {
            let mut store = open(&root.join(user.to_string())).unwrap();
            for n in 0..records {
                store
                    .store
                    .insert(note((user * records + n) as u128, n as i64))
                    .unwrap();
            }
        }
        let before = resident_bytes();
        let started = Instant::now();
        let held: Vec<UserStore> = (0..STORES)
            .map(|user| open(&root.join(user.to_string())).unwrap())
            .collect();
        let opened = started.elapsed();
        let after = resident_bytes();
        let started = Instant::now();
        drop(held);
        let closed = started.elapsed();
        let per_store_kib = match (before, after) {
            (Some(b), Some(a)) => a.saturating_sub(b) / STORES as u64 / 1024,
            _ => 0,
        };
        println!(
            "{records:>6} records/store: open {:>8.2?}/store, close {:>8.2?}/store, +{per_store_kib} KiB resident/store",
            opened / STORES as u32,
            closed / STORES as u32,
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
