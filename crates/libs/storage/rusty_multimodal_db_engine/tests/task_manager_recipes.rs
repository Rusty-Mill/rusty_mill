//! What a task manager needs of the engine, built from the pieces it has
//! today (issue #382, gaps 1, 2 and 5; the spike found each supported):
//!
//! - **Several equality filters.** Index the one field that narrows most
//!   (`list_id`) and filter the rest on the records it returns, rather than
//!   stacking an index per field.
//! - **Equality plus range.** An [`OrderedField`] `Key` may be a tuple, so
//!   `(list_id, due_at)` answers "this list's tasks due between A and B" as
//!   one range walk. A key bound is a *pair* bound: pad the id with
//!   [`Uuid::nil`] and [`Uuid::max`].
//! - **Drag-and-drop ordering.** One marker that is both `ScannableField`
//!   and `OrderedField` re-keys a single record through `update`; a task
//!   moved between two neighbours takes the midpoint of their keys, so no
//!   other record is written until the gap runs out.
//!
//! Two limits shape the layout. `Ordered` answers `PageBy`/`RangeBy` for
//! its own marker only, so with two orders the inner one is reached with
//! `inner()`; forwarding the other marker's queries from the outer layer
//! is a trait-coherence conflict (E0119) with its own impl, the same one
//! `MultiNeighbors`' docs describe. And the durable core wants one
//! `IndexedField` and one `ScannableField` per record type.

use rusty_multimodal_db_engine::generic::query::{
    FilterEq, GetById, Insert, PageBy, RangeBy, Replace, UpdateField,
};
use rusty_multimodal_db_engine::generic::store::Ordered;
use rusty_multimodal_db_engine::generic::traits::{
    IndexedField, OrderedField, Record, ScannableField, SchemaTag,
};
use rusty_multimodal_db_engine::generic::GenericMmapStore;
use serde::{Deserialize, Serialize};
use std::ops::Bound::{Excluded, Included};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Task {
    id: Uuid,
    list_id: Uuid,
    status: u8,
    due_at: i64,
    sort_order: i64,
}

impl Record for Task {
    type Id = Uuid;
    fn id(&self) -> Uuid {
        self.id
    }
}

impl SchemaTag for Task {
    const SCHEMA_TAG: &'static str = "test::task_manager_recipes::Task";
}

/// The one equality index: the list.
struct ByList;
impl IndexedField<ByList> for Task {
    type IndexValue = Uuid;
    fn indexed_value(&self) -> &Uuid {
        &self.list_id
    }
}

/// Manual order: scannable (so `update` moves one task) and ordered by
/// `(list, sort_order)` (so a list pages in its own order).
struct Manual;
impl ScannableField<Manual> for Task {
    type ScanValue = i64;
    fn scannable_value(&self) -> i64 {
        self.sort_order
    }
    fn set_scannable_value(&mut self, value: i64) {
        self.sort_order = value;
    }
}
impl OrderedField<Manual> for Task {
    type Key = (Uuid, i64);
    fn order_key(&self) -> (Uuid, i64) {
        (self.list_id, self.sort_order)
    }
}

/// Due date within a list.
struct DueInList;
impl OrderedField<DueInList> for Task {
    type Key = (Uuid, i64);
    fn order_key(&self) -> (Uuid, i64) {
        (self.list_id, self.due_at)
    }
}

type Core = GenericMmapStore<Task, ByList, Manual>;
type Tasks = Ordered<Ordered<Core, Task, Manual>, Task, DueInList>;

fn list(n: u128) -> Uuid {
    Uuid::from_u128(n << 64)
}

fn task(n: u128, list_id: Uuid, status: u8, due_at: i64, sort_order: i64) -> Task {
    Task {
        id: Uuid::from_u128(n),
        list_id,
        status,
        due_at,
        sort_order,
    }
}

fn open(label: &str, tasks: Vec<Task>) -> (Tasks, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "rusty_multimodal_db_engine_{label}_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let core = Core::create(tasks, &dir.join("tasks.mmap")).unwrap();
    (Ordered::new(Ordered::new(core)), dir)
}

fn sorted(mut ids: Vec<Uuid>) -> Vec<Uuid> {
    ids.sort_unstable();
    ids
}

fn ids(tasks: &[u128]) -> Vec<Uuid> {
    tasks.iter().map(|n| Uuid::from_u128(*n)).collect()
}

/// A key strictly between `before` and `after` (the neighbours the task is
/// dropped between; `None` at either end of the list), or `None` when the
/// two are adjacent and the list needs renumbering.
fn key_between(before: Option<i64>, after: Option<i64>) -> Option<i64> {
    const STEP: i64 = 1 << 20;
    let (low, high) = match (before, after) {
        (None, None) => return Some(0),
        (Some(b), None) => return b.checked_add(STEP),
        (None, Some(a)) => return a.checked_sub(STEP),
        (Some(b), Some(a)) => (b, a),
    };
    let mid = low + (high - low) / 2;
    (mid != low && mid != high).then_some(mid)
}

#[test]
fn equality_on_the_list_then_the_other_fields_on_its_records() {
    let (a, b) = (list(1), list(2));
    let (store, dir) = open(
        "equality",
        vec![
            task(1, a, 0, 10, 0),
            task(2, a, 1, 20, 1),
            task(3, a, 0, 30, 2),
            task(4, b, 0, 10, 0),
        ],
    );
    let in_list = FilterEq::<Task, ByList>::filter_eq(&store, &a);
    let open_in_list: Vec<Uuid> = in_list
        .into_iter()
        .filter(|id| store.get(*id).is_some_and(|t| t.status == 0))
        .collect();
    assert_eq!(sorted(open_in_list), ids(&[1, 3]));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_tuple_key_answers_one_lists_due_range_and_survives_writes() {
    let (a, b) = (list(1), list(2));
    let (mut store, dir) = open(
        "due_range",
        vec![
            task(1, a, 0, 10, 0),
            task(2, a, 0, 20, 1),
            task(3, a, 0, 30, 2),
            task(4, b, 0, 20, 0),
        ],
    );
    let (nil, max) = (Uuid::nil(), Uuid::from_u128(u128::MAX));
    // List `a`, due in [15, 30): the pair bounds carry the list on both
    // sides so list `b` never enters the walk.
    let due = |store: &Tasks, from: i64, to: i64| {
        RangeBy::<Task, DueInList>::range_by(
            store,
            Included(((a, from), nil)),
            Excluded(((a, to), nil)),
        )
    };
    assert_eq!(due(&store, 15, 30), ids(&[2]));
    // The whole list, whatever the due date.
    let whole = RangeBy::<Task, DueInList>::range_by(
        &store,
        Included(((a, i64::MIN), nil)),
        Included(((a, i64::MAX), max)),
    );
    assert_eq!(whole, ids(&[1, 2, 3]));
    // A replace that changes due date and list moves the pair.
    store.replace(task(2, b, 0, 5, 1)).unwrap();
    assert_eq!(due(&store, 15, 40), ids(&[3]));
    // Inverted bounds are empty, never a panic.
    assert!(due(&store, 30, 15).is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn range_by_reaches_the_inner_order_through_inner() {
    let a = list(1);
    let (store, dir) = open(
        "inner_range",
        vec![
            task(1, a, 0, 10, 5),
            task(2, a, 0, 20, 1),
            task(3, a, 0, 30, 3),
        ],
    );
    // The outer layer is `DueInList`; `Manual` is the one beneath it.
    let by_manual = RangeBy::<Task, Manual>::range_by(
        store.inner(),
        Included(((a, i64::MIN), Uuid::nil())),
        Included(((a, i64::MAX), Uuid::from_u128(u128::MAX))),
    );
    assert_eq!(by_manual, ids(&[2, 3, 1]));
    assert_eq!(
        RangeBy::<Task, Manual>::range_count(
            store.inner(),
            Included(((a, 2), Uuid::nil())),
            Included(((a, 5), Uuid::from_u128(u128::MAX))),
        ),
        2
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_reorder_writes_one_record_until_the_gap_runs_out() {
    let a = list(1);
    let (mut store, dir) = open(
        "reorder",
        vec![
            task(1, a, 0, 0, 0),
            task(2, a, 0, 0, 1000),
            task(3, a, 0, 0, 2000),
            task(4, a, 0, 0, 3000),
        ],
    );
    let page = |store: &Tasks| PageBy::<Task, Manual>::page_by(store.inner(), None, 10);
    assert_eq!(page(&store), ids(&[1, 2, 3, 4]));

    // Drag task 4 between 1 and 2: one `update`, no other record touched.
    let key = key_between(Some(0), Some(1000)).unwrap();
    UpdateField::<Task, Manual>::update(&mut store, Uuid::from_u128(4), key).unwrap();
    assert_eq!(page(&store), ids(&[1, 4, 2, 3]));
    for untouched in [1u128, 2, 3] {
        let stored = store.get(Uuid::from_u128(untouched)).unwrap();
        assert_eq!(stored.sort_order, [0, 1000, 2000][(untouched - 1) as usize]);
    }

    // The order is durable: a reopen rebuilds it from the stored keys.
    drop(store);
    let core = Core::open_portable(&dir.join("tasks.mmap")).unwrap();
    let reopened: Tasks = Ordered::new(Ordered::new(core));
    assert_eq!(page(&reopened), ids(&[1, 4, 2, 3]));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn key_between_stops_at_adjacent_keys_and_handles_the_ends() {
    assert_eq!(key_between(None, None), Some(0));
    assert_eq!(key_between(Some(10), Some(20)), Some(15));
    assert_eq!(key_between(Some(10), Some(11)), None, "renumber the list");
    assert_eq!(key_between(Some(10), Some(10)), None);
    assert!(key_between(Some(10), None).unwrap() > 10);
    assert!(key_between(None, Some(10)).unwrap() < 10);
    assert_eq!(key_between(Some(i64::MAX), None), None);
}

#[test]
fn inserting_through_the_outer_layer_keeps_both_orders() {
    let a = list(1);
    let (mut store, dir) = open("insert", vec![task(1, a, 0, 10, 0)]);
    store.insert(task(2, a, 0, 5, 1)).unwrap();
    let by_due = PageBy::<Task, DueInList>::page_by(&store, None, 10);
    let by_manual = PageBy::<Task, Manual>::page_by(store.inner(), None, 10);
    assert_eq!((by_due, by_manual), (ids(&[2, 1]), ids(&[1, 2])));
    let _ = std::fs::remove_dir_all(dir);
}
