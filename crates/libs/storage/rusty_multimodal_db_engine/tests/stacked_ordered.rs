//! Three sort orders on one durable store: the spike `rusty_remind_me`'s
//! ADR-0021 (phase 2) called for before building the hub's store on this
//! engine.
//!
//! The hub pages memories three ways: by its own `hub_seq`, by
//! `(updated_at, id)` in byte order (the legacy keyset cursor), and by
//! `(created_at, id)` for links and relations. `Ordered` answers `PageBy`
//! only for its own marker, so three orders mean three stacked layers,
//! with the inner two reached read-only through `inner()`. This pins what
//! that design needs:
//!
//! - writes through the outermost layer keep *every* layer's index exact
//!   (insert, replace moving all three keys, delete);
//! - each order pages exactly like a brute-force sort of the same records;
//! - a fixed-size id key (bytes zero-padded to 64, ADR-0021's id-length
//!   cap) sorts exactly like the string ids do in byte order;
//! - the indexes rebuild from the durable core after a reopen.

use rusty_multimodal_db_engine::generic::query::{Delete, GetById, Insert, PageBy, Replace};
use rusty_multimodal_db_engine::generic::store::Ordered;
use rusty_multimodal_db_engine::generic::traits::{
    IndexedField, OrderedField, Record, ScannableField, SchemaTag,
};
use rusty_multimodal_db_engine::generic::GenericMmapStore;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// ADR-0021's cap: ids longer than this are refused by the hub.
const ID_CAP: usize = 64;

/// A hub-shaped record: a string id kept verbatim, a `Uuid` derived from
/// it for the engine, and the three keys the hub pages by.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Row {
    uuid: Uuid,
    id: String,
    hub_seq: i64,
    updated_at_us: i64,
    created_at_us: i64,
    category: String,
}

impl Row {
    fn new(id: &str, hub_seq: i64, updated_at_us: i64, created_at_us: i64) -> Self {
        Row {
            uuid: test_uuid(id),
            id: id.to_string(),
            hub_seq,
            updated_at_us,
            created_at_us,
            category: "general".into(),
        }
    }

    /// The id as a fixed-size, byte-order-preserving key: zero padding
    /// sorts a shorter id before any longer id it prefixes, as byte-order
    /// string comparison does.
    fn id_key(&self) -> [u8; ID_CAP] {
        let mut key = [0u8; ID_CAP];
        let bytes = self.id.as_bytes();
        assert!(bytes.len() <= ID_CAP, "the hub refuses longer ids");
        key[..bytes.len()].copy_from_slice(bytes);
        key
    }
}

/// A deterministic `Uuid` per string id (FNV-1a over the bytes, twice).
/// The hub's real mapping is v5 (ADR-0021); this test only needs distinct,
/// stable ids and keeps the engine's `uuid` features as they are.
fn test_uuid(id: &str) -> Uuid {
    let fnv = |seed: u64| {
        id.bytes().fold(seed, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
        })
    };
    let (hi, lo) = (fnv(0xcbf2_9ce4_8422_2325), fnv(0x8422_2325_cbf2_9ce4));
    Uuid::from_u128((u128::from(hi) << 64) | u128::from(lo))
}

impl Record for Row {
    type Id = Uuid;
    fn id(&self) -> Uuid {
        self.uuid
    }
}

impl SchemaTag for Row {
    const SCHEMA_TAG: &'static str = "spike::stacked_ordered::Row";
}

// The mmap core needs one equality-indexed and one scannable field; the
// hub's real record will pick meaningful ones.
struct Category;
impl IndexedField<Category> for Row {
    type IndexValue = String;
    fn indexed_value(&self) -> &String {
        &self.category
    }
}

struct Seq;
impl ScannableField<Seq> for Row {
    type ScanValue = i64;
    fn scannable_value(&self) -> i64 {
        self.hub_seq
    }
    fn set_scannable_value(&mut self, value: i64) {
        self.hub_seq = value;
    }
}
impl OrderedField<Seq> for Row {
    type Key = i64;
    fn order_key(&self) -> i64 {
        self.hub_seq
    }
}

struct UpdatedKeyset;
impl OrderedField<UpdatedKeyset> for Row {
    type Key = (i64, [u8; ID_CAP]);
    fn order_key(&self) -> Self::Key {
        (self.updated_at_us, self.id_key())
    }
}

struct CreatedKeyset;
impl OrderedField<CreatedKeyset> for Row {
    type Key = (i64, [u8; ID_CAP]);
    fn order_key(&self) -> Self::Key {
        (self.created_at_us, self.id_key())
    }
}

type Core = GenericMmapStore<Row, Category, Seq>;
type Stack = Ordered<Ordered<Ordered<Core, Row, CreatedKeyset>, Row, UpdatedKeyset>, Row, Seq>;

fn stack(core: Core) -> Stack {
    Ordered::new(Ordered::new(Ordered::new(core)))
}

fn temp_path(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rusty_multimodal_db_engine_{label}_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("hub.mmap")
}

/// Walk every page of one order, `page` ids at a time, and return the
/// string ids in the order the store produced them.
fn walk<S, M>(layer: &S, store: &Stack, page: usize) -> Vec<String>
where
    S: PageBy<Row, M>,
    Row: OrderedField<M>,
{
    let mut out = Vec::new();
    let mut cursor = None;
    loop {
        let ids = layer.page_by(cursor, page);
        if ids.is_empty() {
            return out;
        }
        for id in &ids {
            let row = store.get(*id).expect("a paged id resolves");
            cursor = Some((<Row as OrderedField<M>>::order_key(&row), *id));
            out.push(row.id);
        }
    }
}

/// A brute-force sort key: the ordered field, then the id's bytes, then
/// the `Uuid` the index breaks exact ties on.
type SortKey = (i64, Vec<u8>, Uuid);

/// The expected order for each index, by brute force over `rows`.
fn expected(rows: &[Row]) -> [Vec<String>; 3] {
    let by = |mut v: Vec<&Row>, key: &dyn Fn(&Row) -> SortKey| {
        v.sort_by_key(|r| key(r));
        v.into_iter().map(|r| r.id.clone()).collect::<Vec<_>>()
    };
    let all: Vec<&Row> = rows.iter().collect();
    [
        by(all.clone(), &|r| (r.hub_seq, Vec::new(), r.uuid)),
        by(all.clone(), &|r| {
            (r.updated_at_us, r.id.as_bytes().to_vec(), r.uuid)
        }),
        by(all, &|r| {
            (r.created_at_us, r.id.as_bytes().to_vec(), r.uuid)
        }),
    ]
}

fn assert_orders(store: &Stack, rows: &[Row]) {
    let [seq, updated, created] = expected(rows);
    for page in [1, 2, 7] {
        assert_eq!(
            walk::<_, Seq>(store, store, page),
            seq,
            "hub_seq order, page {page}"
        );
        assert_eq!(
            walk::<_, UpdatedKeyset>(store.inner(), store, page),
            updated,
            "(updated_at, id) order, page {page}"
        );
        assert_eq!(
            walk::<_, CreatedKeyset>(store.inner().inner(), store, page),
            created,
            "(created_at, id) order, page {page}"
        );
    }
}

fn sample() -> Vec<Row> {
    vec![
        // Same updated_at: ties must break on the id's bytes, with a
        // shorter id ("m") before the id it prefixes ("m1").
        Row::new("m1", 3, 1_000, 10),
        Row::new("m", 1, 1_000, 30),
        Row::new("mem_00ff", 2, 900, 20),
        Row::new("a", 5, 2_000, 20),
        Row::new("Z", 4, 1_500, 5), // uppercase sorts before lowercase in byte order
    ]
}

#[test]
fn three_orders_page_correctly_through_writes_and_a_reopen() {
    let path = temp_path("stacked_ordered");
    let mut rows = sample();
    let mut store = stack(Core::create(rows.clone(), &path).unwrap());
    assert_orders(&store, &rows);

    // Insert through the outermost layer reaches all three indexes.
    let inserted = Row::new("b", 6, 1_000, 1);
    store.insert(inserted.clone()).unwrap();
    rows.push(inserted);
    assert_orders(&store, &rows);

    // A replace that moves all three keys re-keys every index.
    let mut moved = rows[0].clone();
    moved.hub_seq = 7;
    moved.updated_at_us = 50;
    moved.created_at_us = 99;
    store.replace(moved.clone()).unwrap();
    rows[0] = moved;
    assert_orders(&store, &rows);

    // A delete leaves every index.
    let gone = rows.remove(2);
    store.delete(gone.uuid).unwrap();
    assert_orders(&store, &rows);

    // The durable core reopens with its writes, and the three memory-only
    // indexes rebuild from it.
    drop(store);
    let reopened = stack(Core::open_portable(&path).unwrap());
    assert_orders(&reopened, &rows);

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn the_padded_id_key_sorts_like_byte_order_strings() {
    let mut ids = vec!["m1", "m", "mem_00ff", "a", "Z", "m\u{e9}", "mem_", "b"];
    let mut rows: Vec<Row> = ids.iter().map(|id| Row::new(id, 0, 0, 0)).collect();
    rows.sort_by_key(|r| r.id_key());
    ids.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    assert_eq!(rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ids);
}
