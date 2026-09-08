//! `Memory` — this crate's sixth domain and third front-door one
//! (`MEM-FR-001`, ADR-0048, `docs/design/SERVER-MEMORY-DOMAIN-DESIGN.md`):
//! the consumer's own `memories` table, the one `ADR-0036` scoped out
//! ("schema-less memory content … a genuinely different kind of
//! database") and `ADR-0045` named as gate (ii) — "a `Memory` domain is
//! the obvious candidate" for the second table. It is buildable now
//! because the two rounds before it made a table *writable*: records
//! (`ADR-0046`) and, for the domains that have them, edges (`ADR-0047`).
//!
//! # Shape — a bounded projection of `memories`, not the whole table
//!
//! `rusty_remind_me`'s `memories` has twenty-eight columns
//! (`schema_tables.sql:55-64`). This record carries `id` and the eleven
//! fields a memory *is* — what the
//! consumer's `add_memory` writes and its `list`/`get` read back — and
//! names what it leaves out (the module docs of `ADR-0048` and the
//! design's Non-goals): the retrieval-scoring triple (`decay_rate`,
//! `vitality`, `base_weight` — `f64`, which no stored `ValueKind`
//! carries), the sync bookkeeping (`node_id`, `client`), the SPO triple
//! and `superseded_by` (nullable, and the wire has no null), the
//! document chunking pair, the capture provenance ids and `accessed_at`
//! (nullable), `remind_at` (the `Reminder` domain's job),
//! and `deleted_at` (a soft-delete stamp; a record here is deleted
//! outright through `Delete`, `ADR-0051`).
//!
//! - `category` is the equality-filterable `IndexedField` — the
//!   consumer indexes it (`idx_memories_category`) and every list filters
//!   on it. Open `String`, as `Entity::kind` is (`ADR-0039`).
//! - `access_count` is the durably-mutable `ScannableField` — the one
//!   counter the consumer bumps on every retrieval, a plain `i64` as
//!   `Entity::mention_count` is.
//! - `tags` is a `Vec<String>` (`ScanValue::StrList`, protocol 11) —
//!   the consumer stores a JSON array and a `memory_tags` link table.
//! - `metadata_json` carries the consumer's JSON object *as text*: the
//!   store never parses it. This is the honest way to hold "schema-less
//!   content" in a fixed-schema record — it is a field the caller owns.
//! - `content`, `source`, `memory_type`, `status`, the two timestamps,
//!   and `sensitive` cannot be changed one at a time (`UpdateField`
//!   moves the one scannable value) — they change *whole*, with every
//!   other field, through [`crate::generic::query::Replace`]
//!   (`ADR-0049`): the consumer's `update_memory` edits `content`/
//!   `category`/`tags`/`metadata`/`sensitive` in place, and that is a
//!   whole-record replacement on the wire (`Request::Replace`).
//!
//! # Identity
//!
//! The consumer mints `mem_<uuid4 simple>` (`db/queries.rs:101`) and
//! its own `ADR-0016` declares ids opaque. This crate's `RecordId` is
//! `Uuid`; a bridge strips the prefix on the way in and restores it on
//! the way out, losslessly. No derivation, no convention beyond that.
//!
//! # One relation: `mentions`, foreign to the `entity` table
//!
//! `memory_entities` (memory → entity) is the consumer's link table;
//! since `ADR-0050` (`TBL-FR-008`) it is this stack's one relation,
//! `MEMORY_RELATION_LABELS[0]`, a `MultiSymmetric` label whose far
//! endpoint is an `Entity` id — another table's record, which the
//! layer therefore never checks (`MultiSymmetric::with_foreign_labels`;
//! the server checks it against the `MEMORY_FOREIGN_TABLE`). The edge
//! is stored both ways, so an entity id's neighbors under `mentions`
//! are the memories that mention it: the consumer's two lookups, from
//! one edge set. `memory_associations` (memory ↔ memory) is not
//! modeled yet; it would be a second, local label on this same layer.

use super::mmap_store::GenericMmapStore;
use super::store::{MultiSymmetric, Ordered};
use super::traits::{IndexedField, OrderedField, Record, ScannableField, SchemaTag};
use crate::durability::DurabilityError;
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

/// One memory — see the module docs for what each field carries and
/// what the consumer's table holds that this does not.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Memory {
    pub id: Uuid,
    /// The memory's text. Read-only over the wire after insert.
    pub content: String,
    /// The consumer's `category` (default `general`) — the
    /// equality-filterable `IndexedField` (`MEM-FR-002`).
    pub category: String,
    /// The consumer's JSON-array `tags`, as a list.
    pub tags: Vec<String>,
    /// The consumer's `source` (default `manual`).
    pub source: String,
    /// The consumer's `metadata` JSON object, carried as text and never
    /// parsed here (`MEM-FR-004`).
    pub metadata_json: String,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
    /// The consumer's classification: `unclassified`, `decision`,
    /// `fact`, `reference`, … — open, as `category` is.
    pub memory_type: String,
    /// The consumer's lifecycle state, `active` unless something
    /// archived it — open `String`.
    pub status: String,
    /// The consumer's "don't surface by default" flag — a convenience,
    /// not access control, on both sides.
    pub sensitive: bool,
    /// The consumer's retrieval counter — the durably-mutable
    /// `ScannableField` (`MEM-FR-003`).
    pub access_count: i64,
    /// The consumer's soft-delete stamp (`SYN-FR-001`, ADR-0056): Unix
    /// milliseconds, or **`0` for a live record** — the documented
    /// sentinel for the consumer's `NULL`, lossless because the epoch is
    /// never a deletion time. Data, not a rule: the server hides nothing
    /// and `Delete` stays the hard delete. Validated non-negative.
    pub deleted_at_unix_ms: i64,
    /// The consumer's writing node (`SYN-FR-001`): a node's name, or
    /// **`""` for an unattributed record** — the sentinel for `NULL`,
    /// lossless because a node has a name.
    pub node_id: String,
}

impl Record for Memory {
    type Id = Uuid;
    fn id(&self) -> Uuid {
        self.id
    }
}

// Part of the on-disk format (the blob and log headers) — see `Order`/
// `Reminder`'s own impls for the same caveat. `@2` since `ADR-0056`
// (`SYN-FR-002`): the layout gained two fields, and a blob or log written
// at the twelve-field layout must fail distinctly (`RecordBlobUnreadable`
// on the schema tag) rather than mis-decode — `bincode` writes no field
// markers. There is no upgrade; the remedy is a re-push from the
// consumer, and the day a directory cannot be re-pushed is the trigger
// for a layout version with an in-place upgrade (the design's option L2).
impl SchemaTag for Memory {
    const SCHEMA_TAG: &'static str = "memory::Memory@2";
}

/// `MEM-FR-002`: the equality-filterable field.
pub struct CategoryField;
impl IndexedField<CategoryField> for Memory {
    type IndexValue = String;
    fn indexed_value(&self) -> &String {
        &self.category
    }
}

/// `MEM-FR-003`: the durably-mutable field.
pub struct AccessCountField;
impl ScannableField<AccessCountField> for Memory {
    type ScanValue = i64;
    fn scannable_value(&self) -> i64 {
        self.access_count
    }
    fn set_scannable_value(&mut self, value: i64) {
        self.access_count = value;
    }
}

/// The durable production stack — `MEM-FR-005`: no relation of either
/// kind, so `GenericMmapStore` directly.
/// `TBL-FR-008` (ADR-0050): `Memory`'s one relation — the consumer's
/// `memory_entities` link table, a memory *mentions* an entity. Its far
/// endpoint is an `Entity` id, another table's record
/// ([`MEMORY_FOREIGN_TABLE`]); the `MultiSymmetric` layer stores the
/// edge in both directions, so an entity id's neighbors under this
/// label are the memories that mention it.
pub const MEMORY_RELATION_LABELS: [&str; 1] = ["mentions"];

/// The table [`MEMORY_RELATION_LABELS`]' far endpoints live in — what
/// `MemoryConnectionStore::describe_relations` reports as
/// `target_table`, and the name a server must register the `Entity`
/// adapter under for a cross-table `Join`/`Link` to resolve.
pub const MEMORY_FOREIGN_TABLE: &str = "entity";

/// The durable production stack — since `ADR-0050` a `MultiSymmetric`
/// over the core (the change `ADR-0048` said would happen once, in the
/// round that gave this table its second table): one relation,
/// `mentions`, foreign to `entity`.
/// `ORD-FR-004` (ADR-0059): the hub's sync walk orders by
/// `updated_at_unix_ms`; this is the field [`MemoryProductionStack`]
/// keeps a sorted index over.
pub struct UpdatedAtOrder;
impl OrderedField<UpdatedAtOrder> for Memory {
    type Key = i64;
    fn order_key(&self) -> i64 {
        self.updated_at_unix_ms
    }
}

/// The edge layer over the mmap core, under a sorted index on
/// `updated_at_unix_ms` (`ORD-FR-004`, ADR-0059) — memory only, rebuilt
/// from the records at every open; the files are `ADR-0056`'s.
pub type MemoryProductionStack = Ordered<
    MultiSymmetric<GenericMmapStore<Memory, CategoryField, AccessCountField>, Memory>,
    Memory,
    UpdatedAtOrder,
>;

fn labeled(mentions: &[(Uuid, Uuid)]) -> Vec<(String, Vec<(Uuid, Uuid)>)> {
    vec![(MEMORY_RELATION_LABELS[0].to_string(), mentions.to_vec())]
}

/// Build a fresh durable store at `path` with `mentions` edges — each a
/// `(memory id, entity id)` pair — writing the record files, the edge
/// blob, and the label manifest.
pub fn create_memory_production_stack(
    memories: Vec<Memory>,
    mentions: &[(Uuid, Uuid)],
    path: &Path,
) -> Result<MemoryProductionStack, DurabilityError> {
    let core = GenericMmapStore::<Memory, CategoryField, AccessCountField>::create(memories, path)?;
    Ok(Ordered::new(
        MultiSymmetric::create(core, &labeled(mentions), path)?
            .with_foreign_labels(&MEMORY_RELATION_LABELS),
    ))
}

/// Reopen an existing store at `path` from a caller-supplied record and
/// edge set — the `open` analogue of [`create_memory_production_stack`].
pub fn open_memory_production_stack(
    memories: Vec<Memory>,
    mentions: &[(Uuid, Uuid)],
    path: &Path,
) -> Result<MemoryProductionStack, DurabilityError> {
    let core = GenericMmapStore::<Memory, CategoryField, AccessCountField>::open(memories, path)?;
    Ok(Ordered::new(
        MultiSymmetric::open(core, &labeled(mentions), path)?
            .with_foreign_labels(&MEMORY_RELATION_LABELS),
    ))
}

/// Reopen from the files alone — records, insert log, edge blob, edge
/// log, and manifest.
pub fn open_memory_production_stack_portable(
    path: &Path,
) -> Result<MemoryProductionStack, DurabilityError> {
    let core = GenericMmapStore::<Memory, CategoryField, AccessCountField>::open_portable(path)?;
    Ok(Ordered::new(
        MultiSymmetric::open_portable(core, path, &MEMORY_RELATION_LABELS)?
            .with_foreign_labels(&MEMORY_RELATION_LABELS),
    ))
}

/// Open the stack at `path` if its slot file exists, else create an
/// **empty** one there — the shape a served process needs across
/// restarts (`DDR-FR-001`, ADR-0053): the first start creates, every
/// later start reopens from the files alone, and no caller-supplied
/// record list is ever authoritative over what runtime writes left.
/// The decision is on `path` itself (the slot file); a directory that
/// has the slot file but lacks a companion is reported by the reopen,
/// never silently recreated.
///
/// # Errors
///
/// Everything [`create_memory_production_stack`] or
/// [`open_memory_production_stack_portable`] can return.
pub fn open_or_create_memory_production_stack(
    path: &Path,
) -> Result<MemoryProductionStack, DurabilityError> {
    if path.exists() {
        return open_memory_production_stack_portable(path);
    }
    create_memory_production_stack(Vec::new(), &[], path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generic::query::{
        AllIds, FilterEq, GetById, Insert, MultiLink, ScanField, UpdateField,
    };
    use crate::test_support::fresh_temp_dir;

    pub(crate) fn memory(n: u128, category: &str, sensitive: bool) -> Memory {
        Memory {
            id: Uuid::from_u128(n),
            content: format!("memory {n}"),
            category: category.into(),
            tags: vec!["t".into(), format!("n{n}")],
            source: "manual".into(),
            metadata_json: "{}".into(),
            created_at_unix_ms: 1_000 * n as i64,
            updated_at_unix_ms: 1_000 * n as i64,
            memory_type: "unclassified".into(),
            status: "active".into(),
            sensitive,
            access_count: 0,
            deleted_at_unix_ms: 0,
            node_id: String::new(),
        }
    }

    fn sample() -> Vec<Memory> {
        vec![
            memory(1, "general", false),
            memory(2, "preference", false),
            memory(3, "general", true),
        ]
    }

    /// `MEM` acceptance criterion 1: every read and the one write work
    /// on the stack, and an insert with every field — a `StrList`
    /// included — survives a reopen from the files alone.
    #[test]
    fn create_get_filter_scan_update_insert_and_portable_reopen() {
        let dir = fresh_temp_dir("generic_memory").unwrap();
        let path = dir.join("memories.mmap");
        {
            let mut store = create_memory_production_stack(sample(), &[], &path).unwrap();
            let got = GetById::<Memory>::get(&store, Uuid::from_u128(3)).unwrap();
            assert_eq!(got.content, "memory 3");
            assert!(got.sensitive);
            let mut general =
                FilterEq::<Memory, CategoryField>::filter_eq(&store, &"general".into());
            general.sort();
            assert_eq!(general, vec![Uuid::from_u128(1), Uuid::from_u128(3)]);
            UpdateField::<Memory, AccessCountField>::update(&mut store, Uuid::from_u128(1), 7)
                .unwrap();
            let mut counts = ScanField::<Memory, AccessCountField>::scan(&store);
            counts.sort_unstable();
            assert_eq!(counts, vec![0, 0, 7]);

            let mut new = memory(4, "decision", false);
            new.tags = vec!["important".into()];
            new.metadata_json = r#"{"source_url":"https://example"}"#.into();
            Insert::<Memory>::insert(&mut store, new.clone()).unwrap();
            assert_eq!(GetById::<Memory>::get(&store, new.id), Some(new));
            assert_eq!(AllIds::<Memory>::all_ids(&store).len(), 4);
        }
        let reopened = open_memory_production_stack_portable(&path).unwrap();
        let got = GetById::<Memory>::get(&reopened, Uuid::from_u128(4)).unwrap();
        assert_eq!(got.tags, vec!["important".to_string()]);
        assert_eq!(got.metadata_json, r#"{"source_url":"https://example"}"#);
        assert_eq!(
            GetById::<Memory>::get(&reopened, Uuid::from_u128(1))
                .unwrap()
                .access_count,
            7,
            "the slot survived"
        );
        assert_eq!(
            FilterEq::<Memory, CategoryField>::filter_eq(&reopened, &"decision".into()),
            vec![Uuid::from_u128(4)]
        );
    }

    /// `REP` acceptance criterion 1 (ADR-0049) on the simplest front-door
    /// stack: a whole-record replace changes `content`, `tags`, the
    /// indexed `category` (the old bucket loses the id, the new gains
    /// it), and the scannable `access_count` (the slot, in place), and
    /// all of it survives a reopen from the files alone.
    #[test]
    fn replace_changes_every_field_and_survives_portable_reopen() {
        use crate::generic::query::Replace;
        let dir = fresh_temp_dir("generic_memory_replace").unwrap();
        let path = dir.join("memories.mmap");
        let mut edited = memory(1, "decision", true);
        edited.content = "memory 1, revised".into();
        edited.tags = vec!["revised".into()];
        edited.access_count = 12;
        {
            let mut store = create_memory_production_stack(sample(), &[], &path).unwrap();
            Replace::<Memory>::replace(&mut store, edited.clone()).unwrap();
            assert_eq!(
                GetById::<Memory>::get(&store, edited.id),
                Some(edited.clone())
            );
            assert_eq!(
                FilterEq::<Memory, CategoryField>::filter_eq(&store, &"general".into()),
                vec![Uuid::from_u128(3)]
            );
            assert_eq!(
                FilterEq::<Memory, CategoryField>::filter_eq(&store, &"decision".into()),
                vec![Uuid::from_u128(1)]
            );
            let mut counts = ScanField::<Memory, AccessCountField>::scan(&store);
            counts.sort_unstable();
            assert_eq!(counts, vec![0, 0, 12]);
            assert_eq!(AllIds::<Memory>::all_ids(&store).len(), 3, "no new record");
            match Replace::<Memory>::replace(&mut store, memory(9, "general", false)) {
                Err(crate::generic::ReplaceError::NotFound(id)) => {
                    assert_eq!(id, Uuid::from_u128(9))
                }
                other => panic!("expected NotFound, got {other:?}"),
            }
        }
        let reopened = open_memory_production_stack_portable(&path).unwrap();
        assert_eq!(GetById::<Memory>::get(&reopened, edited.id), Some(edited));
        assert_eq!(
            FilterEq::<Memory, CategoryField>::filter_eq(&reopened, &"general".into()),
            vec![Uuid::from_u128(3)]
        );
    }

    /// `TBL-FR-008` (ADR-0050): the stack's one relation, `mentions`,
    /// with entity ids the store never holds — seeded at `create`, added
    /// at runtime through `MultiLink`, listed by `relation_kinds`, read
    /// from both ends, and surviving a portable reopen and a caller-list
    /// reopen alike.
    #[test]
    fn mentions_edges_to_foreign_entity_ids_survive_every_reopen() {
        use crate::generic::query::{MultiLink, MultiNeighbors};
        use crate::generic::LinkOutcome;
        let dir = fresh_temp_dir("generic_memory_mentions").unwrap();
        let path = dir.join("memories.mmap");
        let (ada, engine) = (Uuid::from_u128(0xada), Uuid::from_u128(0xe1e));
        {
            let mut store =
                create_memory_production_stack(sample(), &[(Uuid::from_u128(1), ada)], &path)
                    .unwrap();
            assert!(store.inner().is_foreign("mentions"));
            assert_eq!(
                MultiNeighbors::<Memory>::relation_kinds(&store),
                vec!["mentions".to_string()]
            );
            assert_eq!(
                MultiNeighbors::<Memory>::neighbors_by_relation(
                    &store,
                    "mentions",
                    Uuid::from_u128(1)
                ),
                Some(vec![ada])
            );
            assert_eq!(
                MultiLink::<Memory>::link(&mut store, "mentions", Uuid::from_u128(2), engine)
                    .unwrap(),
                LinkOutcome::Linked
            );
            assert_eq!(
                MultiNeighbors::<Memory>::neighbors_by_relation(&store, "mentions", engine),
                Some(vec![Uuid::from_u128(2)]),
                "an entity id's neighbors are the memories that mention it"
            );
        }
        let reopened = open_memory_production_stack_portable(&path).unwrap();
        assert!(reopened.inner().is_foreign("mentions"));
        assert_eq!(
            MultiNeighbors::<Memory>::neighbors_by_relation(
                &reopened,
                "mentions",
                Uuid::from_u128(2)
            ),
            Some(vec![engine])
        );
        drop(reopened);
        // A caller-list reopen: the caller's list *is* the dataset (the
        // `Symmetric::open` precedent) — the seeded edge stays, the
        // runtime one, already folded into the blob, is not in the list
        // and is rewritten away. A restart that wants every runtime edge
        // reopens portably, as the servers do.
        let reopened =
            open_memory_production_stack(sample(), &[(Uuid::from_u128(1), ada)], &path).unwrap();
        assert_eq!(
            MultiNeighbors::<Memory>::neighbors_by_relation(
                &reopened,
                "mentions",
                Uuid::from_u128(1)
            ),
            Some(vec![ada])
        );
        assert_eq!(
            MultiNeighbors::<Memory>::neighbors_by_relation(
                &reopened,
                "mentions",
                Uuid::from_u128(2)
            ),
            Some(vec![])
        );
    }

    /// `DEL-FR-004`/`005` (ADR-0051) on `Memory`'s stack: deleting a
    /// memory drops its `mentions` edge (the entity's side no longer
    /// lists it); detaching an entity id drops every memory's edge to it
    /// with the memories intact; both survive a portable reopen.
    #[test]
    fn delete_and_detach_drop_mentions_edges_and_survive_reopen() {
        use crate::generic::query::{AllIds, Delete, Detach, MultiNeighbors};
        let dir = fresh_temp_dir("generic_memory_delete").unwrap();
        let path = dir.join("memories.mmap");
        let (ada, engine) = (Uuid::from_u128(0xada), Uuid::from_u128(0xe1e));
        let (one, two, three) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
        {
            let mut store = create_memory_production_stack(
                sample(),
                &[(one, ada), (two, ada), (three, engine)],
                &path,
            )
            .unwrap();
            Delete::<Memory>::delete(&mut store, one).unwrap();
            assert!(GetById::<Memory>::get(&store, one).is_none());
            assert_eq!(
                MultiNeighbors::<Memory>::neighbors_by_relation(&store, "mentions", ada),
                Some(vec![two])
            );
            assert_eq!(
                Detach::<Memory>::detach(&mut store, "mentions", ada).unwrap(),
                1
            );
            assert_eq!(
                MultiNeighbors::<Memory>::neighbors_by_relation(&store, "mentions", two),
                Some(vec![])
            );
            assert!(
                GetById::<Memory>::get(&store, two).is_some(),
                "the memory stays"
            );
            assert_eq!(
                MultiNeighbors::<Memory>::neighbors_by_relation(&store, "mentions", engine),
                Some(vec![three])
            );
            assert_eq!(AllIds::<Memory>::all_ids(&store).len(), 2);
        }
        let reopened = open_memory_production_stack_portable(&path).unwrap();
        assert!(GetById::<Memory>::get(&reopened, one).is_none());
        assert_eq!(
            MultiNeighbors::<Memory>::neighbors_by_relation(&reopened, "mentions", ada),
            Some(vec![])
        );
        assert_eq!(
            MultiNeighbors::<Memory>::neighbors_by_relation(&reopened, "mentions", engine),
            Some(vec![three])
        );
    }

    /// `DDR-FR-001` (ADR-0053): the first call at a path creates an
    /// empty stack; runtime writes on it survive a second call, which
    /// reopens rather than recreates; a third call after a delete and a
    /// compaction still reopens.
    #[test]
    fn open_or_create_creates_empty_then_reopens_with_runtime_writes() {
        use crate::generic::query::{Compact, Delete, MultiNeighbors};
        let dir = fresh_temp_dir("generic_memory_open_or_create").unwrap();
        let path = dir.join("memories.mmap");

        let mut store = open_or_create_memory_production_stack(&path).unwrap();
        assert!(store.all_ids().is_empty(), "created empty");
        store.insert(memory(1, "preference", false)).unwrap();
        store.insert(memory(2, "fact", true)).unwrap();
        store
            .link("mentions", Uuid::from_u128(1), Uuid::from_u128(0xada))
            .unwrap();
        drop(store);

        let mut store = open_or_create_memory_production_stack(&path).unwrap();
        let mut ids = store.all_ids();
        ids.sort();
        assert_eq!(
            ids,
            vec![Uuid::from_u128(1), Uuid::from_u128(2)],
            "reopened, not recreated"
        );
        assert_eq!(
            store.neighbors_by_relation("mentions", Uuid::from_u128(1)),
            Some(vec![Uuid::from_u128(0xada)])
        );
        store.delete(Uuid::from_u128(2)).unwrap();
        store.compact().unwrap();
        drop(store);

        let store = open_or_create_memory_production_stack(&path).unwrap();
        assert_eq!(store.all_ids(), vec![Uuid::from_u128(1)]);
        assert!(GetById::<Memory>::get(&store, Uuid::from_u128(2)).is_none());
    }

    /// `SYN-FR-002` (ADR-0056): a directory written at the twelve-field
    /// layout — modelled by a blob carrying the pre-`@2` schema tag — is
    /// refused distinctly on the portable reopen, naming the record blob;
    /// nothing is rewritten and nothing is recreated.
    #[test]
    fn a_twelve_field_layout_directory_is_refused_distinctly_not_misread() {
        use crate::durability::DurabilityError;
        use crate::generic::record_blob::{blob_path, tag_hash, TAG_OFFSET};
        let dir = fresh_temp_dir("generic_memory_old_layout").unwrap();
        let path = dir.join("memories.mmap");
        drop(create_memory_production_stack(vec![memory(1, "fact", false)], &[], &path).unwrap());
        // Stamp the blob's header with the old tag, exactly as a
        // pre-ADR-0056 build would have written it.
        let blob = blob_path(&path);
        let mut bytes = std::fs::read(&blob).unwrap();
        bytes[TAG_OFFSET..TAG_OFFSET + 8]
            .copy_from_slice(&tag_hash("memory::Memory").to_le_bytes());
        std::fs::write(&blob, &bytes).unwrap();
        let before = std::fs::read(&blob).unwrap();

        match open_memory_production_stack_portable(&path) {
            Err(DurabilityError::RecordBlobUnreadable { path: named, .. }) => {
                assert_eq!(named, blob, "the record blob is named");
            }
            Err(other) => panic!("expected RecordBlobUnreadable, got {other:?}"),
            Ok(_) => panic!("a twelve-field blob must not open"),
        }
        match open_or_create_memory_production_stack(&path) {
            Err(DurabilityError::RecordBlobUnreadable { .. }) => {}
            Err(other) => panic!("expected RecordBlobUnreadable, got {other:?}"),
            Ok(_) => panic!("open_or_create must not recreate"),
        }
        assert_eq!(std::fs::read(&blob).unwrap(), before, "nothing rewritten");
    }

    /// `CNT-FR-001` (ADR-0057): the edge count under a foreign label is
    /// each edge once — stored under both endpoints, halved — and follows
    /// a runtime link and a delete; an unknown label is `None`.
    #[test]
    fn edge_count_counts_each_mentions_edge_once_and_tracks_links_and_deletes() {
        use crate::generic::query::{Delete, MultiNeighbors};
        let dir = fresh_temp_dir("generic_memory_edge_count").unwrap();
        let path = dir.join("memories.mmap");
        let mentions = [
            (Uuid::from_u128(1), Uuid::from_u128(0xada)),
            (Uuid::from_u128(2), Uuid::from_u128(0xada)),
        ];
        let mut store = create_memory_production_stack(
            vec![memory(1, "fact", false), memory(2, "fact", false)],
            &mentions,
            &path,
        )
        .unwrap();
        assert_eq!(store.edge_count("mentions"), Some(2));
        assert_eq!(store.edge_count("no_such_label"), None);
        store
            .link("mentions", Uuid::from_u128(2), Uuid::from_u128(0xe1e))
            .unwrap();
        assert_eq!(store.edge_count("mentions"), Some(3));
        store.delete(Uuid::from_u128(2)).unwrap();
        assert_eq!(
            store.edge_count("mentions"),
            Some(1),
            "both of 2's edges gone"
        );
        drop(store);
        let reopened = open_memory_production_stack_portable(&path).unwrap();
        assert_eq!(reopened.edge_count("mentions"), Some(1));
    }

    /// `ORD-FR-002`–`004` (ADR-0059): the sorted index answers pages in
    /// `(updated_at, id)` order from a strict cursor; an insert, a
    /// replace, an `access_count` update (not the ordered field), and a
    /// delete keep it exact; a portable reopen rebuilds it from the
    /// records.
    #[test]
    fn page_by_updated_at_walks_the_sorted_index_and_tracks_every_write() {
        use crate::generic::query::{Delete, PageBy, Replace};
        let dir = fresh_temp_dir("memory_page_by").unwrap();
        let path = dir.join("memories.mmap");
        let id = Uuid::from_u128;
        // updated_at is 1_000 * n, so ids 1..=5 are already in order;
        // give 3 the same stamp as 2 to exercise the id tie-break.
        let mut third = memory(3, "general", false);
        third.updated_at_unix_ms = 2_000;
        let seeded = vec![
            memory(5, "general", false),
            memory(2, "general", false),
            third,
            memory(1, "general", false),
            memory(4, "general", false),
        ];
        let mut store = create_memory_production_stack(seeded, &[], &path).unwrap();
        assert_eq!(store.indexed_len(), 5);
        assert_eq!(
            store.page_by(None, 10),
            vec![id(1), id(2), id(3), id(4), id(5)]
        );
        assert_eq!(store.page_by(None, 2), vec![id(1), id(2)]);
        assert_eq!(store.page_by(Some((2_000, id(2))), 2), vec![id(3), id(4)]);
        assert_eq!(store.page_by(Some((5_000, id(5))), 2), Vec::<Uuid>::new());

        // Insert lands by its stamp; replace moves; a non-ordered
        // update leaves the order alone; delete removes.
        let mut sixth = memory(6, "general", false);
        sixth.updated_at_unix_ms = 500;
        store.insert(sixth).unwrap();
        assert_eq!(store.page_by(None, 1), vec![id(6)]);
        let mut moved = memory(1, "general", false);
        moved.updated_at_unix_ms = 9_000;
        store.replace(moved).unwrap();
        assert_eq!(store.page_by(Some((5_000, id(5))), 2), vec![id(1)]);
        store.update(id(6), 7).unwrap();
        assert_eq!(store.page_by(None, 1), vec![id(6)]);
        store.delete(id(6)).unwrap();
        assert_eq!(store.page_by(None, 1), vec![id(2)]);
        assert_eq!(store.indexed_len(), 5);
        drop(store);

        let reopened = open_memory_production_stack_portable(&path).unwrap();
        assert_eq!(reopened.indexed_len(), 5);
        assert_eq!(
            reopened.page_by(None, 10),
            vec![id(2), id(3), id(4), id(5), id(1)]
        );
    }
}
