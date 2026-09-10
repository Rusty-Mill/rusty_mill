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
use super::traits::{IndexedField, Record, ScannableField, SchemaTag};
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

/// The durable production stack — `RMD-FR-005`: no relation of either
/// kind, so no `Symmetric`/`Reversed` layer, just `GenericMmapStore`
/// directly.
pub type ReminderProductionStack = GenericMmapStore<Reminder, DueAtField, StatusField>;

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
    GenericMmapStore::<Reminder, DueAtField, StatusField>::create(reminders, path)
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
    GenericMmapStore::<Reminder, DueAtField, StatusField>::open(reminders, path)
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

        let records = ReminderProductionStack::read_portable_records(&path).unwrap();
        assert_eq!(records.len(), 3, "blob records then the logged one");
        assert_eq!(records[2].id, Uuid::from_u128(3));

        let reopened = ReminderProductionStack::open_portable(&path).unwrap();
        let got = GetById::<Reminder>::get(&reopened, Uuid::from_u128(3)).unwrap();
        assert_eq!(got.title, "Water plants");
        assert_eq!(got.status, ReminderStatus::Done, "the slot survived too");
        assert!(!log.exists(), "the fold removed the log");
        let blob_bytes = std::fs::read(&blob).unwrap();
        drop(reopened);

        let again = ReminderProductionStack::open_portable(&path).unwrap();
        assert_eq!(AllIds::<Reminder>::all_ids(&again).len(), 3);
        assert_eq!(
            std::fs::read(&blob).unwrap(),
            blob_bytes,
            "a reopen with nothing inserted rewrites nothing"
        );
        assert!(!log.exists());
    }
}
