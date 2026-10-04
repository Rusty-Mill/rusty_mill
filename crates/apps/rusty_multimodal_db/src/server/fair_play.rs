//! [`ConnectionStore`] adapters for the `Fair Play` domain
//! (`FPL-FR-007`, ADR-0137): the three tables of
//! [`crate::generic::fair_play`] — `person`, `card`, `card_default` —
//! each a [`crate::generic::production::GenericProductionStore`] over
//! its production stack, served together on one listener by
//! [`crate::server::serve_tables`] (ADR-0050) with `card` as the
//! primary. `server`-gated alone, like `reminder`/`memory`: real,
//! deployable capability.
//!
//! # Ids on the wire
//!
//! The wire has no UUID value kind, so `parent_card_id`, `owner_id` and
//! `baseline_id` travel as `Str` of `Uuid::to_string()`, `""` for none
//! (the `node_id` convention `server::relation` uses); `number` is
//! `U32`, `0` for none. All four are declared through
//! [`ConnectionStore::nullable_fields`] (ADR-0128), so a client at
//! protocol 32 or later sees and sends `Null` and the sentinels never
//! leave the adapter's own boundary.
//!
//! # One directed relation on the wire, two in-process
//!
//! The protocol's single `Parent`/`Children` pair carries one relation
//! per table. `card`'s is the self-referential tree (`ParentCard`), the
//! one a front end walks; `OwnedBy` (into `person`) and `BaselineOf`
//! (into `card_default`) are **not** wire relations — `owner_id` and
//! `baseline_id` are plain fields a client reads and follows with a
//! `Use person`/`Use card_default` + `GetById` of its own. A
//! `JOIN card ON parent` works; a join to `person` does not. Named here,
//! not hidden.
//!
//! # Writes go through the domain
//!
//! `card`'s `insert_record`/`replace_record` call
//! [`crate::generic::fair_play::insert_card`]/
//! [`crate::generic::fair_play::replace_card`] under one acquisition of
//! the store's lock, so the invariants the library enforces in-process
//! (`origin == Deck` iff `number` iff `baseline_id`, no self-parent, the
//! parent exists, no cycle, the immutables never change) hold on the
//! wire too; each refusal is
//! [`crate::server::protocol::ErrorCode::Malformed`]. `person` inserts
//! and replaces freely. `card_default` refuses every write with
//! [`crate::server::protocol::ErrorCode::Unsupported`] — the baseline's
//! read-only-ness, convention in-process, is enforced here.
//! `delete_record` is `Unsupported` on all three: a person with
//! cards must not vanish from under them, a parent card's children would
//! dangle, and a baseline's card would lose its reset target; the
//! domain's merge/unsplit is not built (its module docs).
//!
//! No journal and no MVCC: `new(store)` only, the simplest shape the
//! trait allows; `Transaction` batches update the one scannable field
//! (`position`/`player`) under the store's own lock.

use super::nullable::NullableField;
use super::protocol::{
    DomainSchema, ErrorCode, FieldCapabilities, FieldDescriptor, FieldRef, JoinRelation,
    ParentLookup, Predicate, RecordId, RelationCapabilities, RelationDescriptor, ScanValue,
    TransactionOp, ValueKind,
};
use super::{
    default_relation_descriptors, predicate_matches, ConnectionStore, InsertOutcome,
    ReplaceIfOutcome, ReplaceOutcome,
};
use crate::generic::fair_play::{
    insert_card, origin_from_u32, origin_to_u32, replace_card, suit_from_u32, suit_to_u32, Card,
    CardDefault, CardDefaultProductionStack, CardError, CardProductionStack, NameField,
    NumberField, ParentCard, Person, PersonProductionStack, PlayerField, PositionField, SuitField,
};
use crate::generic::production::GenericProductionStore;
use crate::generic::query::{GetById, UpdateField};
use crate::generic::traits::ScannableField;
use crate::generic::{DeleteError, GuardedReplace, InsertError, ReplaceError};
use uuid::Uuid;

// ---------------------------------------------------------------------
// Field tags
// ---------------------------------------------------------------------

pub const PERSON_NAME: FieldRef = 0;
pub const PERSON_PLAYER: FieldRef = 1;

pub const CARD_NUMBER: FieldRef = 0;
pub const CARD_NAME: FieldRef = 1;
pub const CARD_SUIT: FieldRef = 2;
pub const CARD_PARENT_CARD_ID: FieldRef = 3;
pub const CARD_POSITION: FieldRef = 4;
pub const CARD_OWNER_ID: FieldRef = 5;
pub const CARD_CONCEPTION: FieldRef = 6;
pub const CARD_PLANNING: FieldRef = 7;
pub const CARD_EXECUTION: FieldRef = 8;
pub const CARD_MINIMUM_STANDARD_OF_CARE: FieldRef = 9;
pub const CARD_NOTES: FieldRef = 10;
pub const CARD_ORIGIN: FieldRef = 11;
pub const CARD_BASELINE_ID: FieldRef = 12;
const CARD_FIELD_COUNT: FieldRef = 13;

pub const DEFAULT_NUMBER: FieldRef = 0;
pub const DEFAULT_NAME: FieldRef = 1;
pub const DEFAULT_SUIT: FieldRef = 2;
pub const DEFAULT_CONCEPTION: FieldRef = 3;
pub const DEFAULT_PLANNING: FieldRef = 4;
pub const DEFAULT_EXECUTION: FieldRef = 5;
pub const DEFAULT_MINIMUM_STANDARD_OF_CARE: FieldRef = 6;
const DEFAULT_FIELD_COUNT: FieldRef = 7;

/// `card`'s four optional fields and their stored sentinels (ADR-0128).
static CARD_NULLABLE: [NullableField; 4] = [
    NullableField {
        tag: CARD_NUMBER,
        sentinel: ScanValue::U32(0),
    },
    NullableField {
        tag: CARD_PARENT_CARD_ID,
        sentinel: ScanValue::Str(String::new()),
    },
    NullableField {
        tag: CARD_OWNER_ID,
        sentinel: ScanValue::Str(String::new()),
    },
    NullableField {
        tag: CARD_BASELINE_ID,
        sentinel: ScanValue::Str(String::new()),
    },
];

// ---------------------------------------------------------------------
// Helpers shared by the three adapters
// ---------------------------------------------------------------------

fn field(
    tag: FieldRef,
    name: &str,
    value_kind: ValueKind,
    (filter_eq, scan, update): (bool, bool, bool),
) -> FieldDescriptor {
    FieldDescriptor {
        tag,
        name: name.into(),
        value_kind,
        capabilities: FieldCapabilities {
            filter_eq,
            scan,
            update,
        },
    }
}

const READ_ONLY: (bool, bool, bool) = (false, false, false);
const INDEXED: (bool, bool, bool) = (true, false, false);
const SCANNED: (bool, bool, bool) = (false, true, true);

/// An optional id as the wire carries it: its string, or `""`.
fn id_str(id: Option<Uuid>) -> ScanValue {
    ScanValue::Str(id.map(|id| id.to_string()).unwrap_or_default())
}

/// The inverse of [`id_str`]: `""` is none, anything else must parse.
fn parse_id(text: &str) -> Result<Option<Uuid>, ErrorCode> {
    if text.is_empty() {
        return Ok(None);
    }
    Uuid::parse_str(text)
        .map(Some)
        .map_err(|_| ErrorCode::Malformed)
}

/// The `validate_batch` every adapter here shares: the one scannable
/// `u32` field (`mutable`, if any) may be updated on an existing record;
/// every other known tag is `Unsupported`, an unknown one `UnknownField`.
fn validate_u32_batch(
    updates: &[TransactionOp],
    mutable: Option<FieldRef>,
    field_count: FieldRef,
    exists: impl Fn(RecordId) -> bool,
) -> Result<(), (usize, ErrorCode)> {
    for (i, op) in updates.iter().enumerate() {
        if op.field >= field_count {
            return Err((i, ErrorCode::UnknownField));
        }
        if Some(op.field) != mutable {
            return Err((i, ErrorCode::Unsupported));
        }
        if !matches!(op.value, ScanValue::U32(_)) {
            return Err((i, ErrorCode::Malformed));
        }
        if !exists(op.id) {
            return Err((i, ErrorCode::RecordNotFound));
        }
    }
    Ok(())
}

fn apply_u32_batch<S, R, Marker>(
    inner: &mut S,
    updates: &[TransactionOp],
) -> Result<(), (usize, ErrorCode)>
where
    R: ScannableField<Marker, ScanValue = u32, Id = Uuid>,
    S: UpdateField<R, Marker>,
{
    for (i, op) in updates.iter().enumerate() {
        if let ScanValue::U32(value) = op.value {
            UpdateField::<R, Marker>::update(inner, op.id, value)
                .map_err(|_| (i, ErrorCode::RecordNotFound))?;
        }
    }
    Ok(())
}

/// `ISO-FR-002`/`ISO-FR-006` over an adapter's own wire shape — see
/// `DogConnectionStore::check_read_set`.
fn check_read_set(
    reads: &[(RecordId, FieldRef, ScanValue)],
    get: impl Fn(RecordId) -> Option<Vec<(FieldRef, ScanValue)>>,
) -> Result<(), (usize, ErrorCode)> {
    for (id, field, value) in reads {
        let current = get(*id).and_then(|fields| {
            fields
                .into_iter()
                .find(|(tag, _)| tag == field)
                .map(|(_, v)| v)
        });
        if current.as_ref() != Some(value) {
            return Err((0, ErrorCode::Conflict));
        }
    }
    Ok(())
}

/// Every tag of an incoming record exactly once, in any order — the
/// slot each tag fills, `Malformed` on a repeat or a tag's wrong kind,
/// `UnknownField` past `field_count`.
fn slots(
    fields: Vec<(FieldRef, ScanValue)>,
    field_count: FieldRef,
) -> Result<Vec<ScanValue>, ErrorCode> {
    let mut slots: Vec<Option<ScanValue>> = vec![None; field_count as usize];
    for (tag, value) in fields {
        let slot = slots.get_mut(tag as usize).ok_or(ErrorCode::UnknownField)?;
        if slot.is_some() {
            return Err(ErrorCode::Malformed);
        }
        *slot = Some(value);
    }
    slots
        .into_iter()
        .map(|slot| slot.ok_or(ErrorCode::Malformed))
        .collect()
}

fn take_str(slot: &mut ScanValue) -> Result<String, ErrorCode> {
    match std::mem::replace(slot, ScanValue::U32(0)) {
        ScanValue::Str(s) => Ok(s),
        _ => Err(ErrorCode::Malformed),
    }
}

fn take_u32(slot: &ScanValue) -> Result<u32, ErrorCode> {
    match slot {
        ScanValue::U32(v) => Ok(*v),
        _ => Err(ErrorCode::Malformed),
    }
}

fn take_str_list(slot: &mut ScanValue) -> Result<Vec<String>, ErrorCode> {
    match std::mem::replace(slot, ScanValue::U32(0)) {
        ScanValue::StrList(v) => Ok(v),
        _ => Err(ErrorCode::Malformed),
    }
}

fn no_relation<T>(_id: RecordId) -> Result<T, ErrorCode> {
    Err(ErrorCode::Unsupported)
}

// ---------------------------------------------------------------------
// person
// ---------------------------------------------------------------------

/// The `person` table: `name` (indexed), `player` (scannable, updatable).
pub struct PersonConnectionStore {
    store: GenericProductionStore<PersonProductionStack>,
}

impl PersonConnectionStore {
    pub fn new(store: GenericProductionStore<PersonProductionStack>) -> Self {
        Self { store }
    }

    fn fields_of(person: Person) -> Vec<(FieldRef, ScanValue)> {
        vec![
            (PERSON_NAME, ScanValue::Str(person.name)),
            (PERSON_PLAYER, ScanValue::U32(person.player)),
        ]
    }

    fn person_from_fields(
        id: RecordId,
        fields: Vec<(FieldRef, ScanValue)>,
    ) -> Result<Person, ErrorCode> {
        let mut s = slots(fields, 2)?;
        Ok(Person {
            id,
            name: take_str(&mut s[PERSON_NAME as usize])?,
            player: take_u32(&s[PERSON_PLAYER as usize])?,
        })
    }
}

impl ConnectionStore for PersonConnectionStore {
    fn get(&self, id: RecordId) -> Option<Vec<(FieldRef, ScanValue)>> {
        self.store.get::<Person>(id).map(Self::fields_of)
    }

    fn scan_all(&self) -> Vec<(RecordId, Vec<(FieldRef, ScanValue)>)> {
        self.store
            .all_ids::<Person>()
            .into_iter()
            .filter_map(|id| self.get(id).map(|fields| (id, fields)))
            .collect()
    }

    fn record_count(&self) -> Option<usize> {
        Some(self.store.id_count::<Person>())
    }

    fn filter_eq(&self, field: FieldRef, value: &ScanValue) -> Result<Vec<RecordId>, ErrorCode> {
        match (field, value) {
            (PERSON_NAME, ScanValue::Str(name)) => {
                Ok(self.store.filter_eq::<Person, NameField>(name))
            }
            (PERSON_NAME, _) => Err(ErrorCode::Malformed),
            (PERSON_PLAYER, _) => Err(ErrorCode::Unsupported),
            _ => Err(ErrorCode::UnknownField),
        }
    }

    fn scan_field(&self, field: FieldRef) -> Result<Vec<ScanValue>, ErrorCode> {
        match field {
            PERSON_PLAYER => Ok(self
                .store
                .scan::<Person, PlayerField>()
                .into_iter()
                .map(ScanValue::U32)
                .collect()),
            PERSON_NAME => Err(ErrorCode::Unsupported),
            _ => Err(ErrorCode::UnknownField),
        }
    }

    fn update_field(
        &self,
        id: RecordId,
        field: FieldRef,
        value: ScanValue,
    ) -> Result<bool, ErrorCode> {
        match (field, value) {
            (PERSON_PLAYER, ScanValue::U32(player)) => {
                Ok(self.store.update::<Person, PlayerField>(id, player).is_ok())
            }
            (PERSON_PLAYER, _) => Err(ErrorCode::Malformed),
            (PERSON_NAME, _) => Err(ErrorCode::Unsupported),
            _ => Err(ErrorCode::UnknownField),
        }
    }

    fn insert_record(
        &self,
        id: RecordId,
        fields: Vec<(FieldRef, ScanValue)>,
    ) -> Result<InsertOutcome, ErrorCode> {
        let person = Self::person_from_fields(id, fields)?;
        match self.store.insert(person) {
            Ok(()) => Ok(InsertOutcome::Inserted),
            Err(InsertError::Duplicate(_)) => Ok(InsertOutcome::Duplicate),
            Err(InsertError::Durability(_)) => Err(ErrorCode::Storage),
        }
    }

    fn replace_record(
        &self,
        id: RecordId,
        fields: Vec<(FieldRef, ScanValue)>,
    ) -> Result<ReplaceOutcome, ErrorCode> {
        let person = Self::person_from_fields(id, fields)?;
        match self.store.replace(person) {
            Ok(()) => Ok(ReplaceOutcome::Replaced),
            Err(ReplaceError::NotFound(_)) => Ok(ReplaceOutcome::NotFound),
            Err(ReplaceError::Durability(_)) => Err(ErrorCode::Storage),
        }
    }

    fn replace_record_if(
        &self,
        id: RecordId,
        fields: Vec<(FieldRef, ScanValue)>,
        guard: &Predicate,
    ) -> Result<ReplaceIfOutcome, ErrorCode> {
        let person = Self::person_from_fields(id, fields)?;
        let holds = |stored: &Person| predicate_matches(&Self::fields_of(stored.clone()), guard);
        match self.store.replace_if(person, holds) {
            Ok(GuardedReplace::Replaced) => Ok(ReplaceIfOutcome::Replaced),
            Ok(GuardedReplace::Refused) => Ok(ReplaceIfOutcome::GuardFailed),
            Err(ReplaceError::NotFound(_)) => Ok(ReplaceIfOutcome::NotFound),
            Err(ReplaceError::Durability(_)) => Err(ErrorCode::Storage),
        }
    }

    fn compact(&self) -> Result<crate::generic::CompactionReport, ErrorCode> {
        self.store.compact().map_err(|_| ErrorCode::Storage)
    }

    fn parent(&self, id: RecordId) -> Result<ParentLookup, ErrorCode> {
        no_relation(id)
    }

    fn children(&self, id: RecordId) -> Result<Vec<RecordId>, ErrorCode> {
        no_relation(id)
    }

    fn neighbors(&self, id: RecordId) -> Result<Vec<RecordId>, ErrorCode> {
        no_relation(id)
    }

    fn neighbors_by_relation(&self, id: RecordId, _: &str) -> Result<Vec<RecordId>, ErrorCode> {
        no_relation(id)
    }

    fn list_relation_kinds(&self) -> Vec<String> {
        Vec::new()
    }

    fn validate_op(&self, op: &TransactionOp) -> Result<(), ErrorCode> {
        validate_u32_batch(std::slice::from_ref(op), Some(PERSON_PLAYER), 2, |id| {
            self.store.get::<Person>(id).is_some()
        })
        .map_err(|(_, code)| code)
    }

    fn table_name(&self) -> &str {
        "person"
    }

    fn describe(&self) -> DomainSchema {
        DomainSchema {
            fields: vec![
                field(PERSON_NAME, "name", ValueKind::Str, INDEXED),
                field(PERSON_PLAYER, "player", ValueKind::U32, SCANNED),
            ],
            relations: RelationCapabilities {
                parent_children: false,
                neighbors: false,
            },
        }
    }

    fn apply_transaction(
        &self,
        updates: &[TransactionOp],
        read_set: &[(RecordId, FieldRef, ScanValue)],
    ) -> Result<(), (usize, ErrorCode)> {
        self.store.with_exclusive(|inner| {
            validate_u32_batch(updates, Some(PERSON_PLAYER), 2, |id| {
                GetById::<Person>::get(inner, id).is_some()
            })?;
            check_read_set(read_set, |id| {
                GetById::<Person>::get(inner, id).map(Self::fields_of)
            })?;
            apply_u32_batch::<_, Person, PlayerField>(inner, updates)
        })
    }
}

// ---------------------------------------------------------------------
// card
// ---------------------------------------------------------------------

/// The `card` table: `suit` (indexed), `position` (scannable,
/// updatable), the `ParentCard` tree as `parent`/`children`, and writes
/// validated by the domain.
pub struct CardConnectionStore {
    store: GenericProductionStore<CardProductionStack>,
}

impl CardConnectionStore {
    pub fn new(store: GenericProductionStore<CardProductionStack>) -> Self {
        Self { store }
    }

    fn fields_of(card: Card) -> Vec<(FieldRef, ScanValue)> {
        vec![
            (
                CARD_NUMBER,
                ScanValue::U32(card.number.map_or(0, u32::from)),
            ),
            (CARD_NAME, ScanValue::Str(card.name)),
            (CARD_SUIT, ScanValue::U32(suit_to_u32(card.suit))),
            (CARD_PARENT_CARD_ID, id_str(card.parent_card_id)),
            (CARD_POSITION, ScanValue::U32(card.position)),
            (CARD_OWNER_ID, id_str(card.owner_id)),
            (CARD_CONCEPTION, ScanValue::Str(card.conception)),
            (CARD_PLANNING, ScanValue::Str(card.planning)),
            (CARD_EXECUTION, ScanValue::Str(card.execution)),
            (
                CARD_MINIMUM_STANDARD_OF_CARE,
                ScanValue::StrList(card.minimum_standard_of_care),
            ),
            (CARD_NOTES, ScanValue::Str(card.notes)),
            (CARD_ORIGIN, ScanValue::U32(origin_to_u32(card.origin))),
            (CARD_BASELINE_ID, id_str(card.baseline_id)),
        ]
    }

    /// Every tag exactly once with a value of its kind; `suit` and
    /// `origin` known discriminants, `number` within `u16`, the three ids
    /// parseable. The domain's own invariants are [`insert_card`]/
    /// [`replace_card`]'s, applied at the write.
    fn card_from_fields(
        id: RecordId,
        fields: Vec<(FieldRef, ScanValue)>,
    ) -> Result<Card, ErrorCode> {
        let mut s = slots(fields, CARD_FIELD_COUNT)?;
        let number = match take_u32(&s[CARD_NUMBER as usize])? {
            0 => None,
            n => Some(u16::try_from(n).map_err(|_| ErrorCode::Malformed)?),
        };
        Ok(Card {
            id,
            number,
            name: take_str(&mut s[CARD_NAME as usize])?,
            suit: suit_from_u32(take_u32(&s[CARD_SUIT as usize])?).ok_or(ErrorCode::Malformed)?,
            parent_card_id: parse_id(&take_str(&mut s[CARD_PARENT_CARD_ID as usize])?)?,
            position: take_u32(&s[CARD_POSITION as usize])?,
            owner_id: parse_id(&take_str(&mut s[CARD_OWNER_ID as usize])?)?,
            conception: take_str(&mut s[CARD_CONCEPTION as usize])?,
            planning: take_str(&mut s[CARD_PLANNING as usize])?,
            execution: take_str(&mut s[CARD_EXECUTION as usize])?,
            minimum_standard_of_care: take_str_list(
                &mut s[CARD_MINIMUM_STANDARD_OF_CARE as usize],
            )?,
            notes: take_str(&mut s[CARD_NOTES as usize])?,
            origin: origin_from_u32(take_u32(&s[CARD_ORIGIN as usize])?)
                .ok_or(ErrorCode::Malformed)?,
            baseline_id: parse_id(&take_str(&mut s[CARD_BASELINE_ID as usize])?)?,
        })
    }

    /// A domain refusal is `Malformed`; a missing record is `Ok(None)`
    /// for the caller to name; a durability failure is `Storage`; a
    /// duplicate is `Ok(Some(Duplicate))`.
    fn map_card_error(error: CardError) -> Result<Option<InsertOutcome>, ErrorCode> {
        match error {
            CardError::OriginMismatch(_)
            | CardError::SelfParent(_)
            | CardError::ParentNotFound { .. }
            | CardError::Cycle(_)
            | CardError::ImmutableChanged(_)
            | CardError::BaselineMissing(..)
            | CardError::NotADeckCard(_)
            | CardError::HasChildren(_)
            | CardError::HoldsCards { .. }
            | CardError::BadOrder(_) => Err(ErrorCode::Malformed),
            CardError::NotFound(_)
            | CardError::Replace(ReplaceError::NotFound(_))
            | CardError::Delete(DeleteError::NotFound(_)) => Ok(None),
            CardError::Insert(InsertError::Duplicate(_)) => Ok(Some(InsertOutcome::Duplicate)),
            CardError::Insert(InsertError::Durability(_))
            | CardError::Replace(ReplaceError::Durability(_))
            | CardError::Delete(DeleteError::Durability(_)) => Err(ErrorCode::Storage),
        }
    }
}

impl ConnectionStore for CardConnectionStore {
    fn get(&self, id: RecordId) -> Option<Vec<(FieldRef, ScanValue)>> {
        self.store.get::<Card>(id).map(Self::fields_of)
    }

    /// `NLC-FR-001` (ADR-0128): `number`, `parent_card_id`, `owner_id`,
    /// `baseline_id`.
    fn nullable_fields(&self) -> &[NullableField] {
        &CARD_NULLABLE
    }

    fn scan_all(&self) -> Vec<(RecordId, Vec<(FieldRef, ScanValue)>)> {
        self.store
            .all_ids::<Card>()
            .into_iter()
            .filter_map(|id| self.get(id).map(|fields| (id, fields)))
            .collect()
    }

    fn record_count(&self) -> Option<usize> {
        Some(self.store.id_count::<Card>())
    }

    fn filter_eq(&self, field: FieldRef, value: &ScanValue) -> Result<Vec<RecordId>, ErrorCode> {
        match (field, value) {
            (CARD_SUIT, ScanValue::U32(raw)) => {
                let suit = suit_from_u32(*raw).ok_or(ErrorCode::Malformed)?;
                Ok(self.store.filter_eq::<Card, SuitField>(&suit))
            }
            (CARD_SUIT, _) => Err(ErrorCode::Malformed),
            (tag, _) if tag < CARD_FIELD_COUNT => Err(ErrorCode::Unsupported),
            _ => Err(ErrorCode::UnknownField),
        }
    }

    fn scan_field(&self, field: FieldRef) -> Result<Vec<ScanValue>, ErrorCode> {
        match field {
            CARD_POSITION => Ok(self
                .store
                .scan::<Card, PositionField>()
                .into_iter()
                .map(ScanValue::U32)
                .collect()),
            tag if tag < CARD_FIELD_COUNT => Err(ErrorCode::Unsupported),
            _ => Err(ErrorCode::UnknownField),
        }
    }

    fn update_field(
        &self,
        id: RecordId,
        field: FieldRef,
        value: ScanValue,
    ) -> Result<bool, ErrorCode> {
        match (field, value) {
            (CARD_POSITION, ScanValue::U32(position)) => Ok(self
                .store
                .update::<Card, PositionField>(id, position)
                .is_ok()),
            (CARD_POSITION, _) => Err(ErrorCode::Malformed),
            (tag, _) if tag < CARD_FIELD_COUNT => Err(ErrorCode::Unsupported),
            _ => Err(ErrorCode::UnknownField),
        }
    }

    /// The wire's `Insert` through [`insert_card`]: the domain's checks
    /// and the write under one acquisition of the store's lock.
    fn insert_record(
        &self,
        id: RecordId,
        fields: Vec<(FieldRef, ScanValue)>,
    ) -> Result<InsertOutcome, ErrorCode> {
        let card = Self::card_from_fields(id, fields)?;
        match self.store.with_exclusive(|inner| insert_card(inner, card)) {
            Ok(()) => Ok(InsertOutcome::Inserted),
            Err(e) => Self::map_card_error(e)?.ok_or(ErrorCode::RecordNotFound),
        }
    }

    /// The wire's `Replace` through [`replace_card`]; an unknown id is
    /// the normal outcome.
    fn replace_record(
        &self,
        id: RecordId,
        fields: Vec<(FieldRef, ScanValue)>,
    ) -> Result<ReplaceOutcome, ErrorCode> {
        let card = Self::card_from_fields(id, fields)?;
        match self.store.with_exclusive(|inner| replace_card(inner, card)) {
            Ok(()) => Ok(ReplaceOutcome::Replaced),
            Err(e) => match Self::map_card_error(e)? {
                None => Ok(ReplaceOutcome::NotFound),
                Some(_) => Err(ErrorCode::Malformed),
            },
        }
    }

    /// `GRD-FR-003`: the guard over the stored card's wire shape, then
    /// [`replace_card`], under one acquisition of the lock.
    fn replace_record_if(
        &self,
        id: RecordId,
        fields: Vec<(FieldRef, ScanValue)>,
        guard: &Predicate,
    ) -> Result<ReplaceIfOutcome, ErrorCode> {
        let card = Self::card_from_fields(id, fields)?;
        let outcome = self.store.with_exclusive(|inner| {
            let Some(stored) = GetById::<Card>::get(inner, id) else {
                return Ok(ReplaceIfOutcome::NotFound);
            };
            if !predicate_matches(&Self::fields_of(stored), guard) {
                return Ok(ReplaceIfOutcome::GuardFailed);
            }
            replace_card(inner, card).map(|()| ReplaceIfOutcome::Replaced)
        });
        match outcome {
            Ok(outcome) => Ok(outcome),
            Err(e) => match Self::map_card_error(e)? {
                None => Ok(ReplaceIfOutcome::NotFound),
                Some(_) => Err(ErrorCode::Malformed),
            },
        }
    }

    fn compact(&self) -> Result<crate::generic::CompactionReport, ErrorCode> {
        self.store.compact().map_err(|_| ErrorCode::Storage)
    }

    /// The `ParentCard` tree.
    fn parent(&self, id: RecordId) -> Result<ParentLookup, ErrorCode> {
        match self.store.parent::<Card, ParentCard>(id) {
            Ok(Some(parent)) => Ok(ParentLookup::Parent(parent)),
            Ok(None) => Ok(ParentLookup::NoParent),
            Err(_not_found) => Ok(ParentLookup::ChildNotFound),
        }
    }

    fn children(&self, id: RecordId) -> Result<Vec<RecordId>, ErrorCode> {
        Ok(self.store.children::<Card, Card, ParentCard>(id))
    }

    fn neighbors(&self, id: RecordId) -> Result<Vec<RecordId>, ErrorCode> {
        no_relation(id)
    }

    fn neighbors_by_relation(&self, id: RecordId, _: &str) -> Result<Vec<RecordId>, ErrorCode> {
        no_relation(id)
    }

    fn list_relation_kinds(&self) -> Vec<String> {
        Vec::new()
    }

    /// The tree is self-referential, so `parent`/`children` are joinable
    /// within this table (`target_table: None`), as `Employee`'s are.
    fn describe_relations(&self) -> Vec<RelationDescriptor> {
        let mut relations =
            default_relation_descriptors(&self.describe(), self.list_relation_kinds());
        relations.push(RelationDescriptor {
            name: "parent".to_string(),
            kind: JoinRelation::Parent,
            target_table: None,
        });
        relations.push(RelationDescriptor {
            name: "children".to_string(),
            kind: JoinRelation::Children,
            target_table: None,
        });
        relations
    }

    fn validate_op(&self, op: &TransactionOp) -> Result<(), ErrorCode> {
        validate_u32_batch(
            std::slice::from_ref(op),
            Some(CARD_POSITION),
            CARD_FIELD_COUNT,
            |id| self.store.get::<Card>(id).is_some(),
        )
        .map_err(|(_, code)| code)
    }

    fn table_name(&self) -> &str {
        "card"
    }

    fn describe(&self) -> DomainSchema {
        DomainSchema {
            fields: vec![
                field(CARD_NUMBER, "number", ValueKind::U32, READ_ONLY),
                field(CARD_NAME, "name", ValueKind::Str, READ_ONLY),
                field(CARD_SUIT, "suit", ValueKind::U32, INDEXED),
                field(
                    CARD_PARENT_CARD_ID,
                    "parent_card_id",
                    ValueKind::Str,
                    READ_ONLY,
                ),
                field(CARD_POSITION, "position", ValueKind::U32, SCANNED),
                field(CARD_OWNER_ID, "owner_id", ValueKind::Str, READ_ONLY),
                field(CARD_CONCEPTION, "conception", ValueKind::Str, READ_ONLY),
                field(CARD_PLANNING, "planning", ValueKind::Str, READ_ONLY),
                field(CARD_EXECUTION, "execution", ValueKind::Str, READ_ONLY),
                field(
                    CARD_MINIMUM_STANDARD_OF_CARE,
                    "minimum_standard_of_care",
                    ValueKind::StrList,
                    READ_ONLY,
                ),
                field(CARD_NOTES, "notes", ValueKind::Str, READ_ONLY),
                field(CARD_ORIGIN, "origin", ValueKind::U32, READ_ONLY),
                field(CARD_BASELINE_ID, "baseline_id", ValueKind::Str, READ_ONLY),
            ],
            relations: RelationCapabilities {
                parent_children: true,
                neighbors: false,
            },
        }
    }

    fn apply_transaction(
        &self,
        updates: &[TransactionOp],
        read_set: &[(RecordId, FieldRef, ScanValue)],
    ) -> Result<(), (usize, ErrorCode)> {
        self.store.with_exclusive(|inner| {
            validate_u32_batch(updates, Some(CARD_POSITION), CARD_FIELD_COUNT, |id| {
                GetById::<Card>::get(inner, id).is_some()
            })?;
            check_read_set(read_set, |id| {
                GetById::<Card>::get(inner, id).map(Self::fields_of)
            })?;
            apply_u32_batch::<_, Card, PositionField>(inner, updates)
        })
    }
}

// ---------------------------------------------------------------------
// card_default
// ---------------------------------------------------------------------

/// The `card_default` table: read-only on the wire — every write is
/// `Unsupported`, which is how the baseline's immutability is enforced
/// here (in-process it is convention only).
pub struct CardDefaultConnectionStore {
    store: GenericProductionStore<CardDefaultProductionStack>,
}

impl CardDefaultConnectionStore {
    pub fn new(store: GenericProductionStore<CardDefaultProductionStack>) -> Self {
        Self { store }
    }

    fn fields_of(default: CardDefault) -> Vec<(FieldRef, ScanValue)> {
        vec![
            (DEFAULT_NUMBER, ScanValue::U32(u32::from(default.number))),
            (DEFAULT_NAME, ScanValue::Str(default.name)),
            (DEFAULT_SUIT, ScanValue::U32(suit_to_u32(default.suit))),
            (DEFAULT_CONCEPTION, ScanValue::Str(default.conception)),
            (DEFAULT_PLANNING, ScanValue::Str(default.planning)),
            (DEFAULT_EXECUTION, ScanValue::Str(default.execution)),
            (
                DEFAULT_MINIMUM_STANDARD_OF_CARE,
                ScanValue::StrList(default.minimum_standard_of_care),
            ),
        ]
    }
}

impl ConnectionStore for CardDefaultConnectionStore {
    fn get(&self, id: RecordId) -> Option<Vec<(FieldRef, ScanValue)>> {
        self.store.get::<CardDefault>(id).map(Self::fields_of)
    }

    fn scan_all(&self) -> Vec<(RecordId, Vec<(FieldRef, ScanValue)>)> {
        self.store
            .all_ids::<CardDefault>()
            .into_iter()
            .filter_map(|id| self.get(id).map(|fields| (id, fields)))
            .collect()
    }

    fn record_count(&self) -> Option<usize> {
        Some(self.store.id_count::<CardDefault>())
    }

    fn filter_eq(&self, field: FieldRef, value: &ScanValue) -> Result<Vec<RecordId>, ErrorCode> {
        match (field, value) {
            (DEFAULT_SUIT, ScanValue::U32(raw)) => {
                let suit = suit_from_u32(*raw).ok_or(ErrorCode::Malformed)?;
                Ok(self.store.filter_eq::<CardDefault, SuitField>(&suit))
            }
            (DEFAULT_SUIT, _) => Err(ErrorCode::Malformed),
            (tag, _) if tag < DEFAULT_FIELD_COUNT => Err(ErrorCode::Unsupported),
            _ => Err(ErrorCode::UnknownField),
        }
    }

    fn scan_field(&self, field: FieldRef) -> Result<Vec<ScanValue>, ErrorCode> {
        match field {
            DEFAULT_NUMBER => Ok(self
                .store
                .scan::<CardDefault, NumberField>()
                .into_iter()
                .map(ScanValue::U32)
                .collect()),
            tag if tag < DEFAULT_FIELD_COUNT => Err(ErrorCode::Unsupported),
            _ => Err(ErrorCode::UnknownField),
        }
    }

    /// Read-only: no field of a baseline changes over the wire.
    fn update_field(&self, _: RecordId, field: FieldRef, _: ScanValue) -> Result<bool, ErrorCode> {
        if field < DEFAULT_FIELD_COUNT {
            return Err(ErrorCode::Unsupported);
        }
        Err(ErrorCode::UnknownField)
    }

    fn compact(&self) -> Result<crate::generic::CompactionReport, ErrorCode> {
        self.store.compact().map_err(|_| ErrorCode::Storage)
    }

    fn parent(&self, id: RecordId) -> Result<ParentLookup, ErrorCode> {
        no_relation(id)
    }

    fn children(&self, id: RecordId) -> Result<Vec<RecordId>, ErrorCode> {
        no_relation(id)
    }

    fn neighbors(&self, id: RecordId) -> Result<Vec<RecordId>, ErrorCode> {
        no_relation(id)
    }

    fn neighbors_by_relation(&self, id: RecordId, _: &str) -> Result<Vec<RecordId>, ErrorCode> {
        no_relation(id)
    }

    fn list_relation_kinds(&self) -> Vec<String> {
        Vec::new()
    }

    fn validate_op(&self, op: &TransactionOp) -> Result<(), ErrorCode> {
        validate_u32_batch(std::slice::from_ref(op), None, DEFAULT_FIELD_COUNT, |_| {
            true
        })
        .map_err(|(_, code)| code)
    }

    fn table_name(&self) -> &str {
        "card_default"
    }

    fn describe(&self) -> DomainSchema {
        DomainSchema {
            fields: vec![
                field(
                    DEFAULT_NUMBER,
                    "number",
                    ValueKind::U32,
                    (false, true, false),
                ),
                field(DEFAULT_NAME, "name", ValueKind::Str, READ_ONLY),
                field(DEFAULT_SUIT, "suit", ValueKind::U32, INDEXED),
                field(DEFAULT_CONCEPTION, "conception", ValueKind::Str, READ_ONLY),
                field(DEFAULT_PLANNING, "planning", ValueKind::Str, READ_ONLY),
                field(DEFAULT_EXECUTION, "execution", ValueKind::Str, READ_ONLY),
                field(
                    DEFAULT_MINIMUM_STANDARD_OF_CARE,
                    "minimum_standard_of_care",
                    ValueKind::StrList,
                    READ_ONLY,
                ),
            ],
            relations: RelationCapabilities {
                parent_children: false,
                neighbors: false,
            },
        }
    }

    /// Nothing is updatable, so a non-empty batch is refused by
    /// `validate_u32_batch` before any write.
    fn apply_transaction(
        &self,
        updates: &[TransactionOp],
        read_set: &[(RecordId, FieldRef, ScanValue)],
    ) -> Result<(), (usize, ErrorCode)> {
        validate_u32_batch(updates, None, DEFAULT_FIELD_COUNT, |_| true)?;
        check_read_set(read_set, |id| self.get(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generic::fair_play::fixtures::{deck, people, spec};
    use crate::generic::fair_play::{
        card_default_id, create_card_default_production_stack, create_card_production_stack,
        create_person_production_stack, deck_card_id, person_id, split_card, split_card_id,
    };
    use crate::server::nullable::{to_wire, WireContext};
    use crate::server::protocol::Response;
    use crate::test_support::fresh_temp_dir;

    struct Adapters {
        people: PersonConnectionStore,
        cards: CardConnectionStore,
        defaults: CardDefaultConnectionStore,
    }

    /// The domain tests' six deck cards and two people, card 1 split
    /// into `1/a` (unowned) and `1/b` (Bob's).
    fn adapters(label: &str) -> Adapters {
        let dir = fresh_temp_dir(label).unwrap();
        let defaults =
            create_card_default_production_stack(deck(), &dir.join("defaults.mmap")).unwrap();
        let mut cards = create_card_production_stack(
            deck().iter().map(CardDefault::to_card).collect(),
            &dir.join("cards.mmap"),
        )
        .unwrap();
        split_card(
            &mut cards,
            deck_card_id(1),
            vec![
                spec("1/a", "a", None),
                spec("1/b", "b", Some(person_id("Bob"))),
            ],
            None,
        )
        .unwrap();
        let people = create_person_production_stack(people(), &dir.join("people.mmap")).unwrap();
        Adapters {
            people: PersonConnectionStore::new(GenericProductionStore::new(people)),
            cards: CardConnectionStore::new(GenericProductionStore::new(cards)),
            defaults: CardDefaultConnectionStore::new(GenericProductionStore::new(defaults)),
        }
    }

    fn with(
        fields: &[(FieldRef, ScanValue)],
        tag: FieldRef,
        value: ScanValue,
    ) -> Vec<(FieldRef, ScanValue)> {
        fields
            .iter()
            .map(|(t, v)| (*t, if *t == tag { value.clone() } else { v.clone() }))
            .collect()
    }

    #[test]
    fn get_returns_every_field_of_every_table() {
        let a = adapters("fp_adapter_get");
        assert_eq!(
            a.people.get(person_id("Ada")).unwrap(),
            vec![
                (PERSON_NAME, ScanValue::Str("Ada".into())),
                (PERSON_PLAYER, ScanValue::U32(1)),
            ]
        );
        let one = a.cards.get(deck_card_id(1)).unwrap();
        assert_eq!(
            one,
            vec![
                (CARD_NUMBER, ScanValue::U32(1)),
                (CARD_NAME, ScanValue::Str("Card 1".into())),
                (CARD_SUIT, ScanValue::U32(0)),
                (CARD_PARENT_CARD_ID, ScanValue::Str(String::new())),
                (CARD_POSITION, ScanValue::U32(1)),
                (CARD_OWNER_ID, ScanValue::Str(String::new())),
                (CARD_CONCEPTION, ScanValue::Str("conceive 1".into())),
                (CARD_PLANNING, ScanValue::Str("plan 1".into())),
                (CARD_EXECUTION, ScanValue::Str("execute 1".into())),
                (
                    CARD_MINIMUM_STANDARD_OF_CARE,
                    ScanValue::StrList(vec!["std 1 a".into(), "std 1 b".into()])
                ),
                (CARD_NOTES, ScanValue::Str(String::new())),
                (CARD_ORIGIN, ScanValue::U32(0)),
                (
                    CARD_BASELINE_ID,
                    ScanValue::Str(card_default_id(1).to_string())
                ),
            ]
        );
        assert_eq!(one.len(), a.cards.describe().fields.len());
        let b = a.cards.get(split_card_id("1/b")).unwrap();
        assert_eq!(b[CARD_NUMBER as usize].1, ScanValue::U32(0));
        assert_eq!(
            b[CARD_PARENT_CARD_ID as usize].1,
            ScanValue::Str(deck_card_id(1).to_string())
        );
        assert_eq!(
            b[CARD_OWNER_ID as usize].1,
            ScanValue::Str(person_id("Bob").to_string())
        );
        assert_eq!(b[CARD_ORIGIN as usize].1, ScanValue::U32(1));
        let d = a.defaults.get(card_default_id(6)).unwrap();
        assert_eq!(d.len(), a.defaults.describe().fields.len());
        assert_eq!(d[DEFAULT_NUMBER as usize].1, ScanValue::U32(6));
        assert_eq!(d[DEFAULT_SUIT as usize].1, ScanValue::U32(4));
        assert!(a.cards.get(Uuid::from_u128(99)).is_none());
        assert_eq!(a.cards.record_count(), Some(8));
        assert_eq!(a.cards.scan_all().len(), 8);
    }

    #[test]
    fn nullable_sentinels_translate_to_null_on_the_wire() {
        let a = adapters("fp_adapter_null");
        let id = split_card_id("1/a");
        let wire = to_wire(
            Response::Record {
                id,
                fields: a.cards.get(id).unwrap(),
            },
            a.cards.nullable_fields(),
            &WireContext::default(),
        );
        let Response::Record { fields, .. } = wire else {
            panic!("not a record");
        };
        assert_eq!(fields[CARD_NUMBER as usize].1, ScanValue::Null);
        assert_eq!(fields[CARD_OWNER_ID as usize].1, ScanValue::Null);
        assert_eq!(fields[CARD_BASELINE_ID as usize].1, ScanValue::Null);
        assert_eq!(
            fields[CARD_PARENT_CARD_ID as usize].1,
            ScanValue::Str(deck_card_id(1).to_string()),
            "a set id is not null"
        );
        assert!(a.people.nullable_fields().is_empty());
        assert!(a.defaults.nullable_fields().is_empty());
    }

    #[test]
    fn filter_eq_by_suit_and_name_scan_and_update_the_scannable() {
        let a = adapters("fp_adapter_filter");
        let mut home = a.cards.filter_eq(CARD_SUIT, &ScanValue::U32(0)).unwrap();
        home.sort();
        let mut expected = vec![
            deck_card_id(1),
            deck_card_id(2),
            deck_card_id(3),
            split_card_id("1/a"),
            split_card_id("1/b"),
        ];
        expected.sort();
        assert_eq!(home, expected, "the children copied the parent's suit");
        assert_eq!(
            a.cards.filter_eq(CARD_SUIT, &ScanValue::U32(9)),
            Err(ErrorCode::Malformed)
        );
        assert_eq!(
            a.cards.filter_eq(CARD_NAME, &ScanValue::Str("x".into())),
            Err(ErrorCode::Unsupported)
        );
        assert_eq!(
            a.cards.filter_eq(99, &ScanValue::U32(0)),
            Err(ErrorCode::UnknownField)
        );
        assert_eq!(
            a.defaults
                .filter_eq(DEFAULT_SUIT, &ScanValue::U32(1))
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            a.people
                .filter_eq(PERSON_NAME, &ScanValue::Str("Ada".into()))
                .unwrap(),
            vec![person_id("Ada")]
        );
        assert_eq!(a.cards.scan_field(CARD_POSITION).unwrap().len(), 8);
        assert_eq!(a.defaults.scan_field(DEFAULT_NUMBER).unwrap().len(), 6);
        assert_eq!(
            a.cards
                .update_field(split_card_id("1/a"), CARD_POSITION, ScanValue::U32(7)),
            Ok(true)
        );
        assert_eq!(
            a.cards.get(split_card_id("1/a")).unwrap()[CARD_POSITION as usize].1,
            ScanValue::U32(7)
        );
        assert_eq!(
            a.cards
                .update_field(Uuid::from_u128(99), CARD_POSITION, ScanValue::U32(7)),
            Ok(false)
        );
        assert_eq!(
            a.cards
                .update_field(deck_card_id(1), CARD_NAME, ScanValue::Str("x".into())),
            Err(ErrorCode::Unsupported)
        );
        assert_eq!(
            a.defaults
                .update_field(card_default_id(1), DEFAULT_NUMBER, ScanValue::U32(7)),
            Err(ErrorCode::Unsupported)
        );
        assert_eq!(
            a.people
                .update_field(person_id("Ada"), PERSON_PLAYER, ScanValue::U32(2)),
            Ok(true)
        );
    }

    #[test]
    fn parent_and_children_answer_the_split_tree_only_on_card() {
        let a = adapters("fp_adapter_tree");
        let mut kids = a.cards.children(deck_card_id(1)).unwrap();
        kids.sort();
        let mut expected = vec![split_card_id("1/a"), split_card_id("1/b")];
        expected.sort();
        assert_eq!(kids, expected);
        assert_eq!(
            a.cards.parent(split_card_id("1/b")),
            Ok(ParentLookup::Parent(deck_card_id(1)))
        );
        assert_eq!(a.cards.parent(deck_card_id(1)), Ok(ParentLookup::NoParent));
        assert_eq!(
            a.cards.parent(Uuid::from_u128(99)),
            Ok(ParentLookup::ChildNotFound)
        );
        assert!(a.cards.children(deck_card_id(2)).unwrap().is_empty());
        assert_eq!(
            a.cards.neighbors(deck_card_id(1)),
            Err(ErrorCode::Unsupported)
        );
        let names: Vec<_> = a
            .cards
            .describe_relations()
            .into_iter()
            .map(|r| (r.name, r.target_table))
            .collect();
        assert_eq!(
            names,
            vec![("parent".to_string(), None), ("children".to_string(), None)]
        );
        assert!(a.cards.describe().relations.parent_children);
        assert_eq!(
            a.people.parent(person_id("Ada")),
            Err(ErrorCode::Unsupported)
        );
        assert_eq!(
            a.defaults.children(card_default_id(1)),
            Err(ErrorCode::Unsupported)
        );
        assert!(a.people.describe_relations().is_empty());
    }

    #[test]
    fn card_writes_go_through_the_domain_and_its_refusals_are_malformed() {
        let a = adapters("fp_adapter_writes");
        let one = a.cards.get(deck_card_id(1)).unwrap();
        let new = Uuid::from_u128(0xbad);
        // Deck without a number.
        let bad = with(&one, CARD_NUMBER, ScanValue::U32(0));
        assert_eq!(
            a.cards.insert_record(new, bad.clone()),
            Err(ErrorCode::Malformed)
        );
        assert!(a.cards.get(new).is_none(), "nothing written");
        // Family with a number.
        let bad = with(&one, CARD_ORIGIN, ScanValue::U32(1));
        assert_eq!(a.cards.insert_record(new, bad), Err(ErrorCode::Malformed));
        // An unknown suit, origin, a bad id string, a number past u16.
        for (tag, value) in [
            (CARD_SUIT, ScanValue::U32(6)),
            (CARD_ORIGIN, ScanValue::U32(2)),
            (CARD_OWNER_ID, ScanValue::Str("not-a-uuid".into())),
            (CARD_NUMBER, ScanValue::U32(70_000)),
            (CARD_NAME, ScanValue::U32(1)),
        ] {
            assert_eq!(
                a.cards.insert_record(new, with(&one, tag, value)),
                Err(ErrorCode::Malformed)
            );
        }
        // A parent that does not exist; a self-parent.
        let orphan = with(
            &one,
            CARD_PARENT_CARD_ID,
            ScanValue::Str(Uuid::from_u128(77).to_string()),
        );
        assert_eq!(
            a.cards.insert_record(new, orphan),
            Err(ErrorCode::Malformed)
        );
        let selfish = with(&one, CARD_PARENT_CARD_ID, ScanValue::Str(new.to_string()));
        assert_eq!(
            a.cards.insert_record(new, selfish),
            Err(ErrorCode::Malformed)
        );
        // A missing field, a repeated one, an unknown tag.
        let mut short = one.clone();
        short.pop();
        assert_eq!(a.cards.insert_record(new, short), Err(ErrorCode::Malformed));
        let mut twice = one.clone();
        twice.push(one[0].clone());
        assert_eq!(a.cards.insert_record(new, twice), Err(ErrorCode::Malformed));
        let mut extra = one.clone();
        extra.push((99, ScanValue::U32(0)));
        assert_eq!(
            a.cards.insert_record(new, extra),
            Err(ErrorCode::UnknownField)
        );

        // A valid family card under card 2, owned by Ada.
        let mut custom = with(&one, CARD_NUMBER, ScanValue::U32(0));
        custom = with(&custom, CARD_ORIGIN, ScanValue::U32(1));
        custom = with(&custom, CARD_BASELINE_ID, ScanValue::Str(String::new()));
        custom = with(
            &custom,
            CARD_PARENT_CARD_ID,
            ScanValue::Str(deck_card_id(2).to_string()),
        );
        custom = with(
            &custom,
            CARD_OWNER_ID,
            ScanValue::Str(person_id("Ada").to_string()),
        );
        assert_eq!(
            a.cards.insert_record(new, custom.clone()),
            Ok(InsertOutcome::Inserted)
        );
        assert_eq!(
            a.cards.insert_record(new, custom.clone()),
            Ok(InsertOutcome::Duplicate)
        );
        assert_eq!(a.cards.children(deck_card_id(2)).unwrap(), vec![new]);
        assert_eq!(a.cards.get(new).unwrap(), custom);

        // Replace: reassign to Bob; an immutable change and a cycle are
        // refused; an unknown id is NotFound.
        let reassigned = with(
            &custom,
            CARD_OWNER_ID,
            ScanValue::Str(person_id("Bob").to_string()),
        );
        assert_eq!(
            a.cards.replace_record(new, reassigned.clone()),
            Ok(ReplaceOutcome::Replaced)
        );
        assert_eq!(a.cards.get(new).unwrap(), reassigned);
        assert_eq!(
            a.cards
                .replace_record(new, with(&reassigned, CARD_NUMBER, ScanValue::U32(9))),
            Err(ErrorCode::Malformed)
        );
        assert_eq!(
            a.cards.replace_record(
                deck_card_id(2),
                with(
                    &a.cards.get(deck_card_id(2)).unwrap(),
                    CARD_PARENT_CARD_ID,
                    ScanValue::Str(new.to_string()),
                ),
            ),
            Err(ErrorCode::Malformed),
            "2 -> new -> 2 is a cycle"
        );
        assert_eq!(
            a.cards
                .replace_record(Uuid::from_u128(4242), reassigned.clone()),
            Ok(ReplaceOutcome::NotFound)
        );
        // Guarded replace over the wire shape.
        let guard = Predicate {
            field: CARD_OWNER_ID,
            op: crate::server::protocol::CompareOp::Eq,
            value: ScanValue::Str(person_id("Bob").to_string()),
        };
        let back = with(&reassigned, CARD_OWNER_ID, ScanValue::Str(String::new()));
        assert_eq!(
            a.cards.replace_record_if(new, back.clone(), &guard),
            Ok(ReplaceIfOutcome::Replaced)
        );
        assert_eq!(
            a.cards.replace_record_if(new, back, &guard),
            Ok(ReplaceIfOutcome::GuardFailed)
        );
        // Delete is never offered.
        assert_eq!(a.cards.delete_record(new), Err(ErrorCode::Unsupported));
        assert_eq!(
            a.people.delete_record(person_id("Ada")),
            Err(ErrorCode::Unsupported)
        );
    }

    #[test]
    fn person_inserts_and_replaces_and_card_default_refuses_every_write() {
        let a = adapters("fp_adapter_person_default");
        let cy = person_id("Cy");
        let fields = vec![
            (PERSON_NAME, ScanValue::Str("Cy".into())),
            (PERSON_PLAYER, ScanValue::U32(3)),
        ];
        assert_eq!(
            a.people.insert_record(cy, fields.clone()),
            Ok(InsertOutcome::Inserted)
        );
        assert_eq!(
            a.people.insert_record(cy, fields.clone()),
            Ok(InsertOutcome::Duplicate)
        );
        assert_eq!(
            a.people.insert_record(
                Uuid::from_u128(5),
                vec![
                    (PERSON_NAME, ScanValue::U32(1)),
                    (PERSON_PLAYER, ScanValue::U32(1))
                ]
            ),
            Err(ErrorCode::Malformed)
        );
        assert_eq!(
            a.people
                .replace_record(cy, with(&fields, PERSON_PLAYER, ScanValue::U32(4))),
            Ok(ReplaceOutcome::Replaced)
        );
        assert_eq!(
            a.people.get(cy).unwrap()[PERSON_PLAYER as usize].1,
            ScanValue::U32(4)
        );
        assert_eq!(
            a.people.replace_record(Uuid::from_u128(5), fields),
            Ok(ReplaceOutcome::NotFound)
        );

        let baseline = a.defaults.get(card_default_id(1)).unwrap();
        assert_eq!(
            a.defaults
                .insert_record(Uuid::from_u128(7), baseline.clone()),
            Err(ErrorCode::Unsupported)
        );
        assert_eq!(
            a.defaults.replace_record(card_default_id(1), baseline),
            Err(ErrorCode::Unsupported)
        );
        assert_eq!(
            a.defaults.delete_record(card_default_id(1)),
            Err(ErrorCode::Unsupported)
        );
        assert_eq!(
            a.defaults.validate_op(&TransactionOp {
                id: card_default_id(1),
                field: DEFAULT_NUMBER,
                value: ScanValue::U32(2),
            }),
            Err(ErrorCode::Unsupported)
        );
        assert_eq!(a.defaults.record_count(), Some(6));
    }

    #[test]
    fn transactions_update_the_scannable_under_the_lock() {
        let a = adapters("fp_adapter_txn");
        let op = |id: Uuid, field: FieldRef, value: ScanValue| TransactionOp { id, field, value };
        assert_eq!(
            a.cards.apply_transaction(
                &[
                    op(split_card_id("1/a"), CARD_POSITION, ScanValue::U32(5)),
                    op(split_card_id("1/b"), CARD_POSITION, ScanValue::U32(4)),
                ],
                &[(deck_card_id(1), CARD_POSITION, ScanValue::U32(1))],
            ),
            Ok(())
        );
        assert_eq!(
            a.cards.get(split_card_id("1/b")).unwrap()[CARD_POSITION as usize].1,
            ScanValue::U32(4)
        );
        assert_eq!(
            a.cards.apply_transaction(
                &[op(split_card_id("1/a"), CARD_POSITION, ScanValue::U32(6))],
                &[(deck_card_id(1), CARD_POSITION, ScanValue::U32(2))],
            ),
            Err((0, ErrorCode::Conflict))
        );
        assert_eq!(
            a.cards.apply_transaction(
                &[op(deck_card_id(1), CARD_NAME, ScanValue::Str("x".into()))],
                &[],
            ),
            Err((0, ErrorCode::Unsupported))
        );
        assert_eq!(
            a.cards.apply_transaction(
                &[op(Uuid::from_u128(99), CARD_POSITION, ScanValue::U32(1))],
                &[],
            ),
            Err((0, ErrorCode::RecordNotFound))
        );
        assert_eq!(
            a.people.apply_transaction(
                &[op(person_id("Ada"), PERSON_PLAYER, ScanValue::U32(9))],
                &[],
            ),
            Ok(())
        );
        assert_eq!(
            a.people.get(person_id("Ada")).unwrap()[PERSON_PLAYER as usize].1,
            ScanValue::U32(9)
        );
    }
}
