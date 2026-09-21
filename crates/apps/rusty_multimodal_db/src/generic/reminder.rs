//! `Reminder` — this crate's fourth domain, and the generic schema
//! library's first front-door (not `research`-gated) appearance
//! outside its own reference material (`RMD-FR-001`, ADR-0036,
//! `docs/design/SERVER-REMINDER-DOMAIN-DESIGN.md`). Unlike `Order`/
//! `Customer` and `Employee`, `Reminder` has no relation of either
//! kind — the one combination no existing domain has — so
//! `ReminderProductionStack` needs no `Symmetric`/`Reversed`
//! composition layer at all: `GenericMmapStore` (`super::mmap_store`)
//! directly, the simplest domain shape this library supports.
//!
//! `status` (not a plain number, the shape every existing domain
//! uses) is the durably-mutable `ScannableField` (`RMD-FR-002`); a
//! new combination for this library — every existing `ScannableField`
//! so far has been a plain number — made safe by encoding it as its
//! `u32` discriminant, the identical fixed-mapping shape
//! `server::order`'s `status_to_u32`/`status_from_u32` already
//! established for an enum `IndexedField`, now reused for an enum
//! `ScannableField` instead. `due_at_unix_ms` is the equality-
//! filterable `IndexedField` (`RMD-FR-003`); `title` is read-only
//! over the wire (`RMD-FR-004`) — present in every `GetById`/`Query`
//! result, never independently `scan`/`update`/`filter_eq`-able, the
//! same shape `Order::created_at_unix_ms`/`Employee::name` already
//! have.

use super::mmap_store::GenericMmapStore;
use super::store::Ordered;
use super::traits::{IndexedField, OrderedField, Record, ScannableField, SchemaTag};
use crate::durability::DurabilityError;
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

/// A reminder's lifecycle state — `RMD-FR-001`. `Cancelled` is
/// included alongside `Done` rather than deleting a cancelled
/// reminder outright, matching this crate's own "no runtime deletion,
/// fixed schema" invariant every other domain already relies on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReminderStatus {
    Pending,
    Done,
    Snoozed,
    Cancelled,
}

/// `ReminderStatus`'s wire/scan encoding — a fixed discriminant, not
/// `ReminderStatus` itself, the identical shape `server::order`'s own
/// `status_to_u32` established for `OrderStatus`.
pub fn status_to_u32(status: ReminderStatus) -> u32 {
    match status {
        ReminderStatus::Pending => 0,
        ReminderStatus::Done => 1,
        ReminderStatus::Snoozed => 2,
        ReminderStatus::Cancelled => 3,
    }
}

pub fn status_from_u32(value: u32) -> Option<ReminderStatus> {
    match value {
        0 => Some(ReminderStatus::Pending),
        1 => Some(ReminderStatus::Done),
        2 => Some(ReminderStatus::Snoozed),
        3 => Some(ReminderStatus::Cancelled),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reminder {
    pub id: Uuid,
    pub title: String,
    pub due_at_unix_ms: i64,
    pub status: ReminderStatus,
}

impl Record for Reminder {
    type Id = Uuid;
    fn id(&self) -> Uuid {
        self.id
    }
}

// The name written into every `Reminder` companion blob's header —
// part of the on-disk format; see `Order`/`Employee`'s own impls for
// the same caveat.
impl SchemaTag for Reminder {
    const SCHEMA_TAG: &'static str = "reminder::Reminder";
}

/// `RMD-FR-003`: the equality-filterable field. Range/ordering search
/// (`due_at < now`, the actually common case) goes through
/// `Request::Query`/`Request::Aggregate` instead, which already filter
/// on any field regardless of this capability (`ADR-0034`/`ADR-0035`).
pub struct DueAtField;
impl IndexedField<DueAtField> for Reminder {
    type IndexValue = i64;
    fn indexed_value(&self) -> &i64 {
        &self.due_at_unix_ms
    }
}

/// `RMD-FR-002`: the durably-mutable field — an enum discriminant, a
/// new combination for this library (every existing `ScannableField`
/// has been a plain number). `set_scannable_value` is only ever
/// called with an already-validated discriminant: the server
/// adapter's own `validate_batch` (`status_from_u32`) rejects
/// anything else before an update reaches this method, the identical
/// "validate before write" shape every existing domain's
/// `validate_batch` already guarantees.
pub struct StatusField;
impl ScannableField<StatusField> for Reminder {
    type ScanValue = u32;
    fn scannable_value(&self) -> u32 {
        status_to_u32(self.status)
    }
    fn set_scannable_value(&mut self, value: u32) {
        self.status = status_from_u32(value)
            .expect("ReminderConnectionStore validates the discriminant before writing");
    }
}

/// `RDO-FR-001` (ADR-0080): the field [`ReminderProductionStack`] keeps
/// a sorted index over — the consumer's "what is due" listing orders by
/// it and bounds it (`due_at_unix_ms <= now`), the shape `ADR-0059`
/// gave `Memory` and `ADR-0075` a range path for.
pub struct DueAtOrder;
impl OrderedField<DueAtOrder> for Reminder {
    type Key = i64;
    fn order_key(&self) -> i64 {
        self.due_at_unix_ms
    }
}

/// The mmap core — `RMD-FR-005`: no relation of either kind, so no
/// `Symmetric`/`Reversed` layer.
pub type ReminderCore = GenericMmapStore<Reminder, DueAtField, StatusField>;

/// The durable production stack: the core under a sorted index on
/// `due_at_unix_ms` (`RDO-FR-001`, ADR-0080) — memory only, rebuilt
/// from the records at every open; the files are unchanged.
pub type ReminderProductionStack = Ordered<ReminderCore, Reminder, DueAtOrder>;

/// Build a fresh, durable production store for `Reminder` at `path` —
/// the generic analogue of `create_order_production_stack`/
/// `create_employee_production_stack`. Writes two files: `path` (the
/// mmap file) and `<path>.records` (the record blob) — no edge blob,
/// since `Reminder` has no relation.
///
/// # Errors
///
/// Returns [`DurabilityError::Io`] under the same conditions
/// [`GenericMmapStore::create`] does; [`DurabilityError::Serde`] if
/// `reminders` can't be serialized.
pub fn create_reminder_production_stack(
    reminders: Vec<Reminder>,
    path: &Path,
) -> Result<ReminderProductionStack, DurabilityError> {
    Ok(Ordered::new(ReminderCore::create(reminders, path)?))
}

/// Reopen an existing durable production store for `Reminder` at
/// `path` — the generic analogue of `open_order_production_stack`/
/// `open_employee_production_stack`.
///
/// # Errors
///
/// Returns [`DurabilityError::Io`]/[`DurabilityError::InvalidMagic`]/
/// [`DurabilityError::SchemaVersionMismatch`] under the same
/// conditions [`GenericMmapStore::open`] does;
/// [`DurabilityError::Serde`] if a stale companion blob can't be
/// serialized.
pub fn open_reminder_production_stack(
    reminders: Vec<Reminder>,
    path: &Path,
) -> Result<ReminderProductionStack, DurabilityError> {
    Ok(Ordered::new(ReminderCore::open(reminders, path)?))
}

/// Reopen from the files alone — records and insert log — under the
/// sorted index, rebuilt (`RDO-FR-001`, ADR-0080); the generic
/// analogue of `open_memory_production_stack_portable`.
///
/// # Errors
///
/// Everything [`GenericMmapStore::open_portable`] can return.
pub fn open_reminder_production_stack_portable(
    path: &Path,
) -> Result<ReminderProductionStack, DurabilityError> {
    Ok(Ordered::new(ReminderCore::open_portable(path)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generic::query::{FilterEq, GetById, UpdateField};
    use crate::test_support::fresh_temp_dir;

    fn sample_reminders() -> Vec<Reminder> {
        vec![
            Reminder {
                id: Uuid::from_u128(1),
                title: "Pay rent".into(),
                due_at_unix_ms: 1_000,
                status: ReminderStatus::Pending,
            },
            Reminder {
                id: Uuid::from_u128(2),
                title: "Call dentist".into(),
                due_at_unix_ms: 2_000,
                status: ReminderStatus::Snoozed,
            },
        ]
    }

    #[test]
    fn status_to_u32_and_back_round_trips_every_variant() {
        for status in [
            ReminderStatus::Pending,
            ReminderStatus::Done,
            ReminderStatus::Snoozed,
            ReminderStatus::Cancelled,
        ] {
            assert_eq!(status_from_u32(status_to_u32(status)), Some(status));
        }
        assert_eq!(status_from_u32(4), None, "out of range is rejected");
    }

    #[test]
    fn create_then_get_and_filter_eq_by_due_at() {
        let dir = fresh_temp_dir("generic_reminder").unwrap();
        let path = dir.join("reminders.mmap");
        let store = create_reminder_production_stack(sample_reminders(), &path).unwrap();

        let got = GetById::<Reminder>::get(&store, Uuid::from_u128(1)).unwrap();
        assert_eq!(got.title, "Pay rent");
        assert_eq!(got.status, ReminderStatus::Pending);

        let matches = FilterEq::<Reminder, DueAtField>::filter_eq(&store, &2_000);
        assert_eq!(matches, vec![Uuid::from_u128(2)]);
        assert!(FilterEq::<Reminder, DueAtField>::filter_eq(&store, &9_999).is_empty());
    }

    /// `RDO-FR-001` (ADR-0080): the stack keeps a sorted index over
    /// `due_at_unix_ms` — a page walks it in `(due_at, id)` order from a
    /// strict cursor, a range walks it between bounds, an insert lands
    /// by its stamp, a status update leaves the order, and a portable
    /// reopen rebuilds the same order.
    #[test]
    fn page_by_and_range_by_due_at_walk_the_sorted_index_and_survive_reopen() {
        use crate::generic::query::{Insert, PageBy, RangeBy};
        use std::ops::Bound::{Included, Unbounded};
        let dir = fresh_temp_dir("generic_reminder_ordered").unwrap();
        let path = dir.join("reminders.mmap");
        let id = Uuid::from_u128;
        let (min, max) = (Uuid::nil(), Uuid::max());
        let mut store = create_reminder_production_stack(sample_reminders(), &path).unwrap();
        // Two seeded: 1 at 1_000, 2 at 2_000. Insert 3 at 1_000 (ties 1).
        Insert::<Reminder>::insert(
            &mut store,
            Reminder {
                id: id(3),
                title: "Water plants".into(),
                due_at_unix_ms: 1_000,
                status: ReminderStatus::Pending,
            },
        )
        .unwrap();
        assert_eq!(store.page_by(None, 10), vec![id(1), id(3), id(2)]);
        assert_eq!(store.page_by(Some((1_000, id(1))), 10), vec![id(3), id(2)]);
        assert_eq!(
            store.range_by(Unbounded, Included((1_000, max))),
            vec![id(1), id(3)],
            "due_at <= 1_000"
        );
        assert_eq!(
            store.range_by(Included((2_000, min)), Unbounded),
            vec![id(2)],
            "due_at >= 2_000"
        );
        assert_eq!(
            store.range_by_limited(Unbounded, Unbounded, 2),
            None,
            "three in range, budget two"
        );
        UpdateField::<Reminder, StatusField>::update(
            &mut store,
            id(1),
            status_to_u32(ReminderStatus::Done),
        )
        .unwrap();
        assert_eq!(
            store.page_by(None, 10),
            vec![id(1), id(3), id(2)],
            "status leaves the order"
        );
        drop(store);
        let reopened = open_reminder_production_stack_portable(&path).unwrap();
        assert_eq!(reopened.page_by(None, 10), vec![id(1), id(3), id(2)]);
        assert_eq!(reopened.indexed_len(), 3);
    }

    #[test]
    fn update_status_is_durable_across_reopen() {
        let dir = fresh_temp_dir("generic_reminder_reopen").unwrap();
        let path = dir.join("reminders.mmap");
        let reminders = sample_reminders();
        {
            let mut store = create_reminder_production_stack(reminders.clone(), &path).unwrap();
            UpdateField::<Reminder, StatusField>::update(
                &mut store,
                Uuid::from_u128(1),
                status_to_u32(ReminderStatus::Done),
            )
            .unwrap();
        }
        let reopened = open_reminder_production_stack(reminders, &path).unwrap();
        let got = GetById::<Reminder>::get(&reopened, Uuid::from_u128(1)).unwrap();
        assert_eq!(got.status, ReminderStatus::Done);
    }

    /// `INS` acceptance criterion 1 (ADR-0046): an inserted reminder is
    /// visible to every read immediately, updatable, refused on repeat
    /// with nothing changed, and — after a reopen from the files alone —
    /// folded into the blob with the log gone; a second reopen writes
    /// nothing.
    #[test]
    fn insert_is_immediately_visible_durable_and_folded_at_reopen() {
        use crate::generic::query::{AllIds, Insert, ScanField};
        let dir = fresh_temp_dir("generic_reminder_insert").unwrap();
        let path = dir.join("reminders.mmap");
        let log = crate::generic::insert_log::log_path(&path);
        let blob = crate::generic::record_blob::blob_path(&path);
        let new = Reminder {
            id: Uuid::from_u128(3),
            title: "Water plants".into(),
            due_at_unix_ms: 2_000,
            status: ReminderStatus::Pending,
        };
        {
            let mut store = create_reminder_production_stack(sample_reminders(), &path).unwrap();
            assert!(!log.exists(), "a fresh store has no insert log");
            Insert::<Reminder>::insert(&mut store, new.clone()).unwrap();
            assert!(log.exists(), "the insert is logged");

            assert_eq!(
                GetById::<Reminder>::get(&store, Uuid::from_u128(3)),
                Some(new.clone())
            );
            let mut due = FilterEq::<Reminder, DueAtField>::filter_eq(&store, &2_000);
            due.sort();
            assert_eq!(due, vec![Uuid::from_u128(2), Uuid::from_u128(3)]);
            assert_eq!(ScanField::<Reminder, StatusField>::scan(&store).len(), 3);
            assert_eq!(AllIds::<Reminder>::all_ids(&store).len(), 3);
            UpdateField::<Reminder, StatusField>::update(
                &mut store,
                Uuid::from_u128(3),
                status_to_u32(ReminderStatus::Done),
            )
            .unwrap();

            match Insert::<Reminder>::insert(&mut store, new.clone()) {
                Err(crate::generic::InsertError::Duplicate(id)) => {
                    assert_eq!(id, Uuid::from_u128(3))
                }
                other => panic!("expected Duplicate, got {other:?}"),
            }
            assert_eq!(AllIds::<Reminder>::all_ids(&store).len(), 3);
        }

        let records = ReminderCore::read_portable_records(&path).unwrap();
        assert_eq!(records.len(), 3, "blob records then the logged one");
        assert_eq!(records[2].id, Uuid::from_u128(3));

        let reopened = open_reminder_production_stack_portable(&path).unwrap();
        let got = GetById::<Reminder>::get(&reopened, Uuid::from_u128(3)).unwrap();
        assert_eq!(got.title, "Water plants");
        assert_eq!(got.status, ReminderStatus::Done, "the slot survived too");
        assert!(!log.exists(), "the fold removed the log");
        let blob_bytes = std::fs::read(&blob).unwrap();
        drop(reopened);

        let again = open_reminder_production_stack_portable(&path).unwrap();
        assert_eq!(AllIds::<Reminder>::all_ids(&again).len(), 3);
        assert_eq!(
            std::fs::read(&blob).unwrap(),
            blob_bytes,
            "a reopen with nothing inserted rewrites nothing"
        );
        assert!(!log.exists());
    }
}
