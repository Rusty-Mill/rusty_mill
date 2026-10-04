//! `Fair Play` — `rusty_multimodal_db`'s seventh domain and fourth
//! front-door one (`FPL-FR-001`, ADR-0137), as its own libs crate: Eve
//! Rodsky's household-task card system as three tables — `Person`,
//! `Card`, `CardDefault` — on the engine's one-index/one-scan stack (the
//! shape `Memory`, ADR-0048, and `Reminder`, ADR-0036, use), plus the
//! optional-parent `ChildOf` tree the `Rule`/`Source` spikes validated.
//! A library, not an app, so that `rusty_multimodal_db` (its wire
//! adapters, re-exported there as `generic::fair_play`) and
//! `rusty_fair_play` (the web app) can both depend on it under ADR-0003's
//! layer rule.
//!
//! # Shape — the two Fair Play rules that drive it
//!
//! **Whoever holds a card owns all of Conception, Planning and
//! Execution (CPE) for it.** CPE is never assigned separately, so it is
//! three text fields on [`Card`], not a child table.
//!
//! **A card can be split into cards with different owners.** "Cleaning"
//! can become "Bathrooms" held by one partner and "Floors" by the other,
//! each a full card with its own CPE, minimum standard and owner. So
//! `Card` is a **self-referential tree** (`parent_card_id`, an optional
//! `ChildOf<ParentCard>`) and there is no separate `Task` table: a
//! split-off card is the same record type as the deck card it came
//! from, which keeps every query — held by, by suit, balance — one
//! query over one table whatever the depth.
//!
//! # Origin and baseline — customization is derived, never stored
//!
//! Every deck card has a [`CardDefault`] holding its text as shipped,
//! written once by the seed loader (`examples/fair_play_seed.rs`) and
//! then **read-only by convention**: this module exposes no update or
//! replace path for it, but the library has no enforced immutability —
//! a caller reaching for `Replace<CardDefault>` on the raw stack can
//! still write it. A card's state — [`CardState::Original`], `Edited`
//! or `Custom` — is computed by [`card_state`] from `origin` and a
//! six-field comparison against the baseline (`name`, `suit`,
//! `conception`, `planning`, `execution`, `minimum_standard_of_care`),
//! never from a stored flag that could drift. `notes`, `owner_id`,
//! `parent_card_id` and `position` are play state or annotation and
//! never make a card `Edited`. Invariant: `origin == Deck` iff
//! `number.is_some()` iff `baseline_id.is_some()` — checked by
//! [`Card::validate`] on every write that goes through [`insert_card`]/
//! [`replace_card`]; a write through the raw `Insert`/`Replace` traits
//! bypasses it (a documented gap, tested).
//!
//! # Ownership — explicit on every card, never inherited
//!
//! A card has at most one owner (`owner_id`, an optional
//! `ChildOf<OwnedBy>` into the `Person` table), who holds all CPE for
//! *that* card. A parent may have a different owner from its children,
//! or none; the parent's owner holds what is left at that level. The
//! real "still undealt" list is therefore the **unowned leaf cards**
//! ([`unassigned_leaf_cards`]), and a balance that counts a split parent
//! and its children would double-count the same work, so
//! [`balance`] has a leaf-only variant. CPE text is not copied from
//! parent to child on a split; a "default from parent" is a hook for
//! later.
//!
//! # The one index, the one scannable
//!
//! `suit` is the `IndexedField` on both `Card` and `CardDefault`: the
//! deck is dealt suit by suit, and it is the one equality filter every
//! view wants. `position` is `Card`'s `ScannableField`: it is the one
//! field that changes alone (a sibling reorder), so `UpdateField` moves
//! it in place without a whole-record `Replace`; `number` never changes
//! and `origin` has two values, so neither earns the slot. `position` is
//! `u32`, not `u16`, because the slot file holds `u32`/`i64` only
//! (`MmapFieldValue`) and a narrowing `set_scannable_value` would have to
//! panic or truncate. `CardDefault` scans `number`; `Person` indexes
//! `name` (the seed loader resolves owners by it) and scans `player`,
//! the seat number the deck's `(Player 1)`/`(Player 2)` cards refer to.
//!
//! # `split_card` is not atomic
//!
//! The library has no multi-record transaction. [`split_card`] inserts
//! the children first — each already pointing at an existing parent —
//! and replaces the parent last, so a crash at any point leaves a valid
//! store: some or all children present under an unchanged parent, or
//! everything. Nothing ever dangles. See the ADR for the exact windows.
//!
//! # Not in v1 (hooks named, not built)
//!
//! Merge/unsplit (needs `Delete` of the children and a decision about
//! their owners), deal history (an append-only `Assignment` table
//! layered over `Replace`), and per-card status or recurrence (Fair
//! Play divides ownership; it is not a checklist).

/// The seed loader: the supplied deck, people and splits, from CSV text
/// or files, into a data directory (`FPL-FR-006`).
pub mod seed;

use rusty_multimodal_db_engine::durability::DurabilityError;
use rusty_multimodal_db_engine::generic::mmap_store::GenericMmapStore;
use rusty_multimodal_db_engine::generic::query::{
    AllIds, Children, Delete, FilterEq, GetById, Insert, Parent, Replace, UpdateField,
};
use rusty_multimodal_db_engine::generic::store::Reversed;
use rusty_multimodal_db_engine::generic::traits::{
    ChildOf, IndexedField, Record, ScannableField, SchemaTag,
};
use rusty_multimodal_db_engine::generic::{DeleteError, InsertError, NotFound, ReplaceError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use uuid::Uuid;

// ---------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------

/// The six suits of the deck — derived from the supplied seed file's
/// own `suit` column (22/22/22/22/10/2 cards), not from any prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Suit {
    Home,
    Out,
    Caregiving,
    Magic,
    Wild,
    UnicornSpace,
}

impl Suit {
    pub const ALL: [Suit; 6] = [
        Suit::Home,
        Suit::Out,
        Suit::Caregiving,
        Suit::Magic,
        Suit::Wild,
        Suit::UnicornSpace,
    ];

    /// The seed file's spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Suit::Home => "Home",
            Suit::Out => "Out",
            Suit::Caregiving => "Caregiving",
            Suit::Magic => "Magic",
            Suit::Wild => "Wild",
            Suit::UnicornSpace => "Unicorn Space",
        }
    }

    /// The inverse of [`Self::as_str`]; `None` for anything else.
    pub fn parse(text: &str) -> Option<Suit> {
        Suit::ALL.into_iter().find(|s| s.as_str() == text)
    }
}

/// `Suit`'s wire encoding — a fixed discriminant, the `status_to_u32`
/// shape `Reminder`/`Order` use for an enum field.
pub fn suit_to_u32(suit: Suit) -> u32 {
    match suit {
        Suit::Home => 0,
        Suit::Out => 1,
        Suit::Caregiving => 2,
        Suit::Magic => 3,
        Suit::Wild => 4,
        Suit::UnicornSpace => 5,
    }
}

pub fn suit_from_u32(value: u32) -> Option<Suit> {
    Suit::ALL.into_iter().find(|s| suit_to_u32(*s) == value)
}

/// Where a card came from. Set at creation, never changed
/// ([`replace_card`] refuses a change).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Origin {
    /// One of the 100 original deck cards; `number` and `baseline_id`
    /// are set.
    Deck,
    /// Anything the family created, every split-off card included;
    /// `number` and `baseline_id` are `None`.
    Family,
}

pub fn origin_to_u32(origin: Origin) -> u32 {
    match origin {
        Origin::Deck => 0,
        Origin::Family => 1,
    }
}

pub fn origin_from_u32(value: u32) -> Option<Origin> {
    match value {
        0 => Some(Origin::Deck),
        1 => Some(Origin::Family),
        _ => None,
    }
}

// ---------------------------------------------------------------------
// Deterministic ids
// ---------------------------------------------------------------------

/// A deterministic id for a seeded row — SHA-256 over
/// `fair_play:<kind>:<key>`, first 16 bytes, the `entity_id` recipe
/// (ADR-0042) with a namespace. What makes the seed loader idempotent:
/// the same deck number or person name always mints the same id, so a
/// re-run finds the row it wrote before instead of creating a second.
pub fn fair_play_id(kind: &str, key: &str) -> Uuid {
    let digest = Sha256::digest(format!("fair_play:{kind}:{key}").as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    Uuid::from_bytes(bytes)
}

pub fn person_id(name: &str) -> Uuid {
    fair_play_id("person", name.trim())
}

pub fn card_default_id(number: u16) -> Uuid {
    fair_play_id("card_default", &number.to_string())
}

pub fn deck_card_id(number: u16) -> Uuid {
    fair_play_id("card", &number.to_string())
}

/// `path` is the split file's `parent_path/name`, e.g. `17/Bathrooms`.
pub fn split_card_id(path: &str) -> Uuid {
    fair_play_id("split", path)
}

/// The three slot files a Fair Play data directory holds — one name
/// shared by the seed loader, the server binary and every test, so a
/// directory one of them wrote the others reopen.
pub const PERSON_FILE: &str = "people.mmap";
pub const CARD_FILE: &str = "cards.mmap";
pub const CARD_DEFAULT_FILE: &str = "card_defaults.mmap";
/// The directory lock file: whoever writes the three stores holds it, so
/// a running service is never written under from a second process.
pub const LOCK_FILE: &str = "store.lock";

// ---------------------------------------------------------------------
// Person
// ---------------------------------------------------------------------

/// One partner. Two rows in practice; nothing here assumes it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Person {
    pub id: Uuid,
    /// The equality-filterable `IndexedField` — the seed loader and a
    /// front end both resolve a person by name.
    pub name: String,
    /// The seat the deck's `(Player 1)`/`(Player 2)` cards refer to —
    /// the `ScannableField`, assigned by the seed loader from file
    /// order (1-based).
    pub player: u32,
}

impl Record for Person {
    type Id = Uuid;
    fn id(&self) -> Uuid {
        self.id
    }
}

impl SchemaTag for Person {
    const SCHEMA_TAG: &'static str = "fair_play::Person";
}

pub struct NameField;
impl IndexedField<NameField> for Person {
    type IndexValue = String;
    fn indexed_value(&self) -> &String {
        &self.name
    }
}

pub struct PlayerField;
impl ScannableField<PlayerField> for Person {
    type ScanValue = u32;
    fn scannable_value(&self) -> u32 {
        self.player
    }
    fn set_scannable_value(&mut self, value: u32) {
        self.player = value;
    }
}

/// The durable `Person` stack: no relation of its own (cards point at
/// people, not the other way), so the mmap core directly.
pub type PersonProductionStack = GenericMmapStore<Person, NameField, PlayerField>;

pub fn create_person_production_stack(
    people: Vec<Person>,
    path: &Path,
) -> Result<PersonProductionStack, DurabilityError> {
    PersonProductionStack::create(people, path)
}

pub fn open_person_production_stack_portable(
    path: &Path,
) -> Result<PersonProductionStack, DurabilityError> {
    PersonProductionStack::open_portable(path)
}

/// Open if the slot file exists, else create empty (`DDR-FR-001`).
pub fn open_or_create_person_production_stack(
    path: &Path,
) -> Result<PersonProductionStack, DurabilityError> {
    if path.exists() {
        return open_person_production_stack_portable(path);
    }
    create_person_production_stack(Vec::new(), path)
}

// ---------------------------------------------------------------------
// CardDefault
// ---------------------------------------------------------------------

/// One original deck card's text as shipped — the baseline a family's
/// edits are measured against and reset to. Written by the seed loader;
/// read-only by convention afterwards (see the module docs).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CardDefault {
    pub id: Uuid,
    /// 1..=100, unique by the seed loader's check — the stack itself
    /// cannot enforce uniqueness of a non-id field (a reported gap).
    pub number: u16,
    pub name: String,
    pub suit: Suit,
    pub conception: String,
    pub planning: String,
    pub execution: String,
    pub minimum_standard_of_care: Vec<String>,
}

impl Record for CardDefault {
    type Id = Uuid;
    fn id(&self) -> Uuid {
        self.id
    }
}

impl SchemaTag for CardDefault {
    const SCHEMA_TAG: &'static str = "fair_play::CardDefault";
}

/// The `IndexedField` marker shared by `Card` and `CardDefault`.
pub struct SuitField;
impl IndexedField<SuitField> for CardDefault {
    type IndexValue = Suit;
    fn indexed_value(&self) -> &Suit {
        &self.suit
    }
}

pub struct NumberField;
impl ScannableField<NumberField> for CardDefault {
    type ScanValue = u32;
    fn scannable_value(&self) -> u32 {
        u32::from(self.number)
    }
    fn set_scannable_value(&mut self, value: u32) {
        // Only reachable through `UpdateField`, which this module never
        // exposes for a baseline; clamped rather than panicking.
        self.number = u16::try_from(value).unwrap_or(u16::MAX);
    }
}

pub type CardDefaultProductionStack = GenericMmapStore<CardDefault, SuitField, NumberField>;

pub fn create_card_default_production_stack(
    defaults: Vec<CardDefault>,
    path: &Path,
) -> Result<CardDefaultProductionStack, DurabilityError> {
    CardDefaultProductionStack::create(defaults, path)
}

pub fn open_card_default_production_stack_portable(
    path: &Path,
) -> Result<CardDefaultProductionStack, DurabilityError> {
    CardDefaultProductionStack::open_portable(path)
}

pub fn open_or_create_card_default_production_stack(
    path: &Path,
) -> Result<CardDefaultProductionStack, DurabilityError> {
    if path.exists() {
        return open_card_default_production_stack_portable(path);
    }
    create_card_default_production_stack(Vec::new(), path)
}

// ---------------------------------------------------------------------
// Card
// ---------------------------------------------------------------------

/// One card — a deck card or anything the family made from one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Card {
    pub id: Uuid,
    /// The deck index 1..=100 for an original card; `None` for a card
    /// the family created. `Some` iff `origin == Deck`.
    pub number: Option<u16>,
    pub name: String,
    /// The `IndexedField`. A split-off card copies its parent's.
    pub suit: Suit,
    /// `ChildOf<ParentCard>`: `None` is a top-level deck card.
    pub parent_card_id: Option<Uuid>,
    /// Sibling order under `parent_card_id` — the `ScannableField`.
    pub position: u32,
    /// `ChildOf<OwnedBy>`: `None` is unassigned. Explicit on every
    /// card, never inherited.
    pub owner_id: Option<Uuid>,
    pub conception: String,
    pub planning: String,
    pub execution: String,
    /// One concrete, checkable standard per entry.
    pub minimum_standard_of_care: Vec<String>,
    /// Family-only annotation; never counts as a customization.
    pub notes: String,
    /// Set at creation, never changed.
    pub origin: Origin,
    /// `ChildOf<BaselineOf>`: the [`CardDefault`] of a deck card;
    /// `None` for a family card. `Some` iff `origin == Deck`.
    pub baseline_id: Option<Uuid>,
}

impl Record for Card {
    type Id = Uuid;
    fn id(&self) -> Uuid {
        self.id
    }
}

impl SchemaTag for Card {
    const SCHEMA_TAG: &'static str = "fair_play::Card";
}

impl IndexedField<SuitField> for Card {
    type IndexValue = Suit;
    fn indexed_value(&self) -> &Suit {
        &self.suit
    }
}

pub struct PositionField;
impl ScannableField<PositionField> for Card {
    type ScanValue = u32;
    fn scannable_value(&self) -> u32 {
        self.position
    }
    fn set_scannable_value(&mut self, value: u32) {
        self.position = value;
    }
}

/// The tree: a card under its parent card.
pub struct ParentCard;
impl ChildOf<ParentCard> for Card {
    type ParentId = Uuid;
    fn parent_id(&self) -> Option<Uuid> {
        self.parent_card_id
    }
}

/// A card held by a person — cross-table, into `Person`.
pub struct OwnedBy;
impl ChildOf<OwnedBy> for Card {
    type ParentId = Uuid;
    fn parent_id(&self) -> Option<Uuid> {
        self.owner_id
    }
}

/// A deck card's baseline — cross-table, into `CardDefault`. Read
/// through the free `Parent` blanket impl only; no reverse index, since
/// no required query walks from a baseline to its card.
pub struct BaselineOf;
impl ChildOf<BaselineOf> for Card {
    type ParentId = Uuid;
    fn parent_id(&self) -> Option<Uuid> {
        self.baseline_id
    }
}

/// The six fields whose change makes a deck card `Edited`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextField {
    Name,
    Suit,
    Conception,
    Planning,
    Execution,
    MinimumStandardOfCare,
}

impl TextField {
    pub const ALL: [TextField; 6] = [
        TextField::Name,
        TextField::Suit,
        TextField::Conception,
        TextField::Planning,
        TextField::Execution,
        TextField::MinimumStandardOfCare,
    ];

    pub fn name(self) -> &'static str {
        match self {
            TextField::Name => "name",
            TextField::Suit => "suit",
            TextField::Conception => "conception",
            TextField::Planning => "planning",
            TextField::Execution => "execution",
            TextField::MinimumStandardOfCare => "minimum_standard_of_care",
        }
    }
}

/// Why a card is refused by [`Card::validate`], [`insert_card`] or
/// [`replace_card`].
#[derive(Debug, thiserror::Error)]
pub enum CardError {
    #[error("card {0}: origin Deck requires a number and a baseline; Family requires neither")]
    OriginMismatch(Uuid),
    #[error("card {0}: a card cannot be its own parent")]
    SelfParent(Uuid),
    #[error("card {card}: parent card {parent} has no record")]
    ParentNotFound { card: Uuid, parent: Uuid },
    #[error("card {0}: the new parent chain reaches the card itself")]
    Cycle(Uuid),
    #[error("card {0}: number, origin and baseline_id never change")]
    ImmutableChanged(Uuid),
    #[error("card {0} has no record")]
    NotFound(Uuid),
    #[error("card {0}: a deck card's baseline {1} has no record")]
    BaselineMissing(Uuid, Uuid),
    #[error("card {0}: only a deck card has a baseline to reset to")]
    NotADeckCard(Uuid),
    #[error("{0}")]
    Insert(#[from] InsertError<Uuid>),
    #[error("{0}")]
    Replace(#[from] ReplaceError<Uuid>),
    #[error("card {0} still has children; unsplit it first")]
    HasChildren(Uuid),
    #[error("person {person} still holds {cards} card(s); reassign them first")]
    HoldsCards { person: Uuid, cards: usize },
    #[error("card {0}: the order must name each current child exactly once")]
    BadOrder(Uuid),
    #[error("{0}")]
    Delete(#[from] DeleteError<Uuid>),
}

impl Card {
    /// The record-local invariants: `origin == Deck` iff `number` and
    /// `baseline_id` are set, and a card is not its own parent.
    pub fn validate(&self) -> Result<(), CardError> {
        let deck = self.origin == Origin::Deck;
        if deck != self.number.is_some() || deck != self.baseline_id.is_some() {
            return Err(CardError::OriginMismatch(self.id));
        }
        if self.parent_card_id == Some(self.id) {
            return Err(CardError::SelfParent(self.id));
        }
        Ok(())
    }

    /// One of the six comparable fields as text — the suit by its seed
    /// spelling, the standards joined by `|` as the seed file writes them.
    pub fn text_field(&self, field: TextField) -> String {
        match field {
            TextField::Name => self.name.clone(),
            TextField::Suit => self.suit.as_str().to_string(),
            TextField::Conception => self.conception.clone(),
            TextField::Planning => self.planning.clone(),
            TextField::Execution => self.execution.clone(),
            TextField::MinimumStandardOfCare => self.minimum_standard_of_care.join("|"),
        }
    }
}

impl CardDefault {
    pub fn text_field(&self, field: TextField) -> String {
        match field {
            TextField::Name => self.name.clone(),
            TextField::Suit => self.suit.as_str().to_string(),
            TextField::Conception => self.conception.clone(),
            TextField::Planning => self.planning.clone(),
            TextField::Execution => self.execution.clone(),
            TextField::MinimumStandardOfCare => self.minimum_standard_of_care.join("|"),
        }
    }

    /// The live card the seed loader creates for this baseline.
    pub fn to_card(&self) -> Card {
        Card {
            id: deck_card_id(self.number),
            number: Some(self.number),
            name: self.name.clone(),
            suit: self.suit,
            parent_card_id: None,
            position: u32::from(self.number),
            owner_id: None,
            conception: self.conception.clone(),
            planning: self.planning.clone(),
            execution: self.execution.clone(),
            minimum_standard_of_care: self.minimum_standard_of_care.clone(),
            notes: String::new(),
            origin: Origin::Deck,
            baseline_id: Some(self.id),
        }
    }
}

/// The durable `Card` stack: the mmap core (`suit` index, `position`
/// slot) under two memory-only reverse indexes — `OwnedBy` (cards held
/// by a person) inside, `ParentCard` (children of a card) outside. The
/// outer `Reversed` answers `Children` for its own marker only; the
/// inner one's is forwarded by the concrete impl below, the per-pair
/// technique `forward_scannable_pairs!` exists for (E0119 forbids a
/// generic forwarding impl). `BaselineOf` has no reverse index.
pub type CardProductionStack = Reversed<
    Reversed<GenericMmapStore<Card, SuitField, PositionField>, Person, Card, OwnedBy>,
    Card,
    Card,
    ParentCard,
>;

impl Children<Person, Card, OwnedBy> for CardProductionStack {
    fn children(&self, parent_id: Uuid) -> Vec<Uuid> {
        Children::<Person, Card, OwnedBy>::children(self.inner(), parent_id)
    }
}

fn reverse(
    core: GenericMmapStore<Card, SuitField, PositionField>,
    cards: &[Card],
) -> CardProductionStack {
    let owned = Reversed::<_, Person, Card, OwnedBy>::new(core, cards);
    Reversed::<_, Card, Card, ParentCard>::new(owned, cards)
}

pub fn create_card_production_stack(
    cards: Vec<Card>,
    path: &Path,
) -> Result<CardProductionStack, DurabilityError> {
    let core = GenericMmapStore::<Card, SuitField, PositionField>::create(cards.clone(), path)?;
    Ok(reverse(core, &cards))
}

/// Reopen from the files alone; both reverse indexes are rebuilt from
/// the records, in the blob's (creation) order.
pub fn open_card_production_stack_portable(
    path: &Path,
) -> Result<CardProductionStack, DurabilityError> {
    let cards = GenericMmapStore::<Card, SuitField, PositionField>::read_portable_records(path)?;
    let core = GenericMmapStore::<Card, SuitField, PositionField>::open(cards.clone(), path)?;
    Ok(reverse(core, &cards))
}

pub fn open_or_create_card_production_stack(
    path: &Path,
) -> Result<CardProductionStack, DurabilityError> {
    if path.exists() {
        return open_card_production_stack_portable(path);
    }
    create_card_production_stack(Vec::new(), path)
}

// ---------------------------------------------------------------------
// Validated writes
// ---------------------------------------------------------------------

/// Walks from `start` up the parent chain; `true` if it reaches
/// `target`. Stops where the trail goes cold or repeats.
fn chain_reaches<S>(store: &S, start: Option<Uuid>, target: Uuid) -> bool
where
    S: GetById<Card>,
{
    let mut seen = HashSet::new();
    let mut current = start;
    while let Some(id) = current {
        if id == target {
            return true;
        }
        if !seen.insert(id) {
            return false;
        }
        current = match store.get(id) {
            Some(card) => card.parent_card_id,
            None => return false,
        };
    }
    false
}

fn check_parent<S>(store: &S, card: &Card) -> Result<(), CardError>
where
    S: GetById<Card>,
{
    let Some(parent) = card.parent_card_id else {
        return Ok(());
    };
    if store.get(parent).is_none() {
        return Err(CardError::ParentNotFound {
            card: card.id,
            parent,
        });
    }
    if chain_reaches(store, Some(parent), card.id) {
        return Err(CardError::Cycle(card.id));
    }
    Ok(())
}

/// `Insert` with the domain's checks first: [`Card::validate`], the
/// parent exists, and the parent chain does not already reach this id.
/// The owner and baseline are other tables' records and are not
/// checked here (the caller holds those stores).
pub fn insert_card<S>(store: &mut S, card: Card) -> Result<(), CardError>
where
    S: GetById<Card> + Insert<Card>,
{
    card.validate()?;
    check_parent(store, &card)?;
    store.insert(card)?;
    Ok(())
}

/// `Replace` with the domain's checks first: everything [`insert_card`]
/// checks, plus `number`, `origin` and `baseline_id` must equal the
/// stored record's. Every reassign, re-parent, edit and reset goes
/// through here.
pub fn replace_card<S>(store: &mut S, card: Card) -> Result<(), CardError>
where
    S: GetById<Card> + Replace<Card>,
{
    card.validate()?;
    let old = store.get(card.id).ok_or(CardError::NotFound(card.id))?;
    if old.number != card.number || old.origin != card.origin || old.baseline_id != card.baseline_id
    {
        return Err(CardError::ImmutableChanged(card.id));
    }
    check_parent(store, &card)?;
    store.replace(card)?;
    Ok(())
}

/// Query 6: hand a card to `owner` (`None` for nobody), everything else
/// untouched — one `Replace`.
pub fn reassign_card<S>(store: &mut S, id: Uuid, owner: Option<Uuid>) -> Result<(), CardError>
where
    S: GetById<Card> + Replace<Card>,
{
    let mut card = store.get(id).ok_or(CardError::NotFound(id))?;
    card.owner_id = owner;
    replace_card(store, card)
}

/// Move a card among its siblings — the one field with its own slot.
pub fn set_position<S>(store: &mut S, id: Uuid, position: u32) -> Result<(), NotFound<Uuid>>
where
    S: UpdateField<Card, PositionField>,
{
    store.update(id, position)
}

/// Delete a leaf card. A card with children is refused
/// (`CardError::HasChildren`): delete its subtree with [`unsplit_card`]
/// first, so no child is left pointing at a missing parent.
pub fn delete_card<S>(store: &mut S, id: Uuid) -> Result<(), CardError>
where
    S: GetById<Card> + Delete<Card> + Children<Card, Card, ParentCard>,
{
    if store.get(id).is_none() {
        return Err(CardError::NotFound(id));
    }
    if !is_leaf(store, id) {
        return Err(CardError::HasChildren(id));
    }
    store.delete(id)?;
    Ok(())
}

/// Undo a split: delete every card under `id`, deepest first, and keep
/// `id` itself. Returns the deleted ids in deletion order. An
/// interrupted unsplit leaves a smaller, still well-formed subtree, so a
/// rerun finishes the job.
pub fn unsplit_card<S>(store: &mut S, id: Uuid) -> Result<Vec<Uuid>, CardError>
where
    S: GetById<Card> + Delete<Card> + Children<Card, Card, ParentCard>,
{
    let tree = card_tree(store, id).map_err(|NotFound(id)| CardError::NotFound(id))?;
    fn post_order(node: &CardTree, out: &mut Vec<Uuid>) {
        for child in &node.children {
            post_order(child, out);
            out.push(child.card.id);
        }
    }
    let mut order = Vec::new();
    post_order(&tree, &mut order);
    for child in &order {
        store.delete(*child)?;
    }
    Ok(order)
}

/// Delete a person who holds no card (`CardError::HoldsCards` otherwise),
/// so no card is left owned by a missing person.
pub fn delete_person<P, C>(people: &mut P, cards: &C, id: Uuid) -> Result<(), CardError>
where
    P: GetById<Person> + Delete<Person>,
    C: Children<Person, Card, OwnedBy>,
{
    if people.get(id).is_none() {
        return Err(CardError::NotFound(id));
    }
    let held = cards_held_by(cards, id).len();
    if held > 0 {
        return Err(CardError::HoldsCards {
            person: id,
            cards: held,
        });
    }
    people.delete(id)?;
    Ok(())
}

/// Reorder the children of `parent`: `order` must name each current child
/// exactly once (`CardError::BadOrder` otherwise, before anything is
/// written), and gets positions `0..n` in that order. The slots are then
/// written one at a time, so a crash part way leaves some positions
/// applied; it is not a transaction.
pub fn reorder_children<S>(store: &mut S, parent: Uuid, order: &[Uuid]) -> Result<(), CardError>
where
    S: GetById<Card> + Children<Card, Card, ParentCard> + UpdateField<Card, PositionField>,
{
    if store.get(parent).is_none() {
        return Err(CardError::NotFound(parent));
    }
    let current: HashSet<Uuid> = Children::<Card, Card, ParentCard>::children(store, parent)
        .into_iter()
        .collect();
    let given: HashSet<Uuid> = order.iter().copied().collect();
    if given.len() != order.len() || given != current {
        return Err(CardError::BadOrder(parent));
    }
    for (position, id) in (0u32..).zip(order) {
        set_position(store, *id, position).map_err(|NotFound(id)| CardError::NotFound(id))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------
// Reads
// ---------------------------------------------------------------------

fn records<S>(store: &S, ids: Vec<Uuid>) -> Vec<Card>
where
    S: GetById<Card>,
{
    ids.into_iter().filter_map(|id| store.get(id)).collect()
}

/// Query 1: every card `person` holds, at any depth, unordered.
pub fn cards_held_by<S>(store: &S, person: Uuid) -> Vec<Uuid>
where
    S: Children<Person, Card, OwnedBy>,
{
    Children::<Person, Card, OwnedBy>::children(store, person)
}

pub fn is_leaf<S>(store: &S, id: Uuid) -> bool
where
    S: Children<Card, Card, ParentCard>,
{
    Children::<Card, Card, ParentCard>::children(store, id).is_empty()
}

/// Query 2: every card with no owner — a scan, since `owner_id` has no
/// index bucket for `None` (`Reversed` skips a record with no parent).
pub fn unassigned_cards<S>(store: &S) -> Vec<Uuid>
where
    S: AllIds<Card> + GetById<Card>,
{
    records(store, store.all_ids())
        .into_iter()
        .filter(|c| c.owner_id.is_none())
        .map(|c| c.id)
        .collect()
}

/// Query 2, the real "still undealt" list: unowned cards nobody split.
pub fn unassigned_leaf_cards<S>(store: &S) -> Vec<Uuid>
where
    S: AllIds<Card> + GetById<Card> + Children<Card, Card, ParentCard>,
{
    unassigned_cards(store)
        .into_iter()
        .filter(|id| is_leaf(store, *id))
        .collect()
}

/// Query 3: the index.
pub fn cards_by_suit<S>(store: &S, suit: Suit) -> Vec<Uuid>
where
    S: FilterEq<Card, SuitField>,
{
    store.filter_eq(&suit)
}

/// Query 4: the deck card numbered `number`. A scan — `number` has no
/// index — returning every match so a duplicate (which the stack cannot
/// forbid) is visible rather than hidden. The seed loader's ids make
/// `GetById(deck_card_id(number))` the O(1) path when its convention
/// holds.
pub fn cards_by_number<S>(store: &S, number: u16) -> Vec<Uuid>
where
    S: AllIds<Card> + GetById<Card>,
{
    records(store, store.all_ids())
        .into_iter()
        .filter(|c| c.number == Some(number))
        .map(|c| c.id)
        .collect()
}

/// Query 5: cards held per person, through the `OwnedBy` index — all
/// cards, or leaf cards only (a split parent and its children would
/// otherwise count the same work twice).
pub fn balance<S>(store: &S, people: &[Uuid], leaf_only: bool) -> Vec<(Uuid, usize)>
where
    S: Children<Person, Card, OwnedBy> + Children<Card, Card, ParentCard>,
{
    people
        .iter()
        .map(|person| {
            let held = cards_held_by(store, *person);
            let n = if leaf_only {
                held.iter().filter(|id| is_leaf(store, **id)).count()
            } else {
                held.len()
            };
            (*person, n)
        })
        .collect()
}

/// Query 7: direct children, by `(position, id)`.
pub fn children_ordered<S>(store: &S, id: Uuid) -> Vec<Card>
where
    S: GetById<Card> + Children<Card, Card, ParentCard>,
{
    let mut kids = records(
        store,
        Children::<Card, Card, ParentCard>::children(store, id),
    );
    kids.sort_by_key(|c| (c.position, c.id));
    kids
}

/// Query 8: `id`, its parent, …, the top-level card — `rule_trace`'s
/// `chain_to_root`, with the same cycle guard and the same "stop where
/// the trail goes cold" behaviour for a dangling parent. `Err` only if
/// `id` itself has no record.
pub fn chain_to_root<S>(store: &S, id: Uuid) -> Result<Vec<Uuid>, NotFound<Uuid>>
where
    S: Parent<Card, ParentCard>,
{
    let mut chain = Vec::new();
    let mut seen = HashSet::new();
    let mut current = id;
    let mut next = store.parent(current);
    while let Ok(parent) = next {
        if !seen.insert(current) {
            break;
        }
        chain.push(current);
        match parent {
            Some(p) => {
                current = p;
                next = store.parent(current);
            }
            None => break,
        }
    }
    if chain.is_empty() {
        return Err(NotFound(id));
    }
    Ok(chain)
}

/// The last element of [`chain_to_root`].
pub fn root_card_id<S>(store: &S, id: Uuid) -> Result<Uuid, NotFound<Uuid>>
where
    S: Parent<Card, ParentCard>,
{
    chain_to_root(store, id).map(|chain| chain[chain.len() - 1])
}

/// Query 9's shape: a card with its children, nested, each level in
/// `(position, id)` order.
#[derive(Debug, Clone, PartialEq)]
pub struct CardTree {
    pub card: Card,
    pub children: Vec<CardTree>,
}

impl CardTree {
    /// Every card in the tree, parents before children.
    pub fn flatten(&self) -> Vec<&Card> {
        let mut out = vec![&self.card];
        for child in &self.children {
            out.extend(child.flatten());
        }
        out
    }
}

fn build_tree<S>(store: &S, card: Card, seen: &mut HashSet<Uuid>) -> CardTree
where
    S: GetById<Card> + Children<Card, Card, ParentCard>,
{
    let children = if seen.insert(card.id) {
        children_ordered(store, card.id)
            .into_iter()
            .map(|child| build_tree(store, child, seen))
            .collect()
    } else {
        Vec::new() // cycle guard: a repeated card gets no subtree
    };
    CardTree { card, children }
}

/// Query 9: the full tree under `id`.
pub fn card_tree<S>(store: &S, id: Uuid) -> Result<CardTree, NotFound<Uuid>>
where
    S: GetById<Card> + Children<Card, Card, ParentCard>,
{
    let card = store.get(id).ok_or(NotFound(id))?;
    Ok(build_tree(store, card, &mut HashSet::new()))
}

/// Query 10: the leaf cards under `id` (`id` itself if it has no
/// children), in tree order.
pub fn leaf_cards_under<S>(store: &S, id: Uuid) -> Result<Vec<Uuid>, NotFound<Uuid>>
where
    S: GetById<Card> + Children<Card, Card, ParentCard>,
{
    let tree = card_tree(store, id)?;
    fn walk(node: &CardTree, out: &mut Vec<Uuid>) {
        if node.children.is_empty() {
            out.push(node.card.id);
            return;
        }
        for child in &node.children {
            walk(child, out);
        }
    }
    let mut out = Vec::new();
    walk(&tree, &mut out);
    Ok(out)
}

/// Who holds the work under a root: each leaf's owner and its count.
pub fn owner_coverage_under<S>(
    store: &S,
    id: Uuid,
) -> Result<BTreeMap<Option<Uuid>, usize>, NotFound<Uuid>>
where
    S: GetById<Card> + Children<Card, Card, ParentCard>,
{
    let mut coverage = BTreeMap::new();
    for leaf in records(store, leaf_cards_under(store, id)?) {
        *coverage.entry(leaf.owner_id).or_insert(0) += 1;
    }
    Ok(coverage)
}

fn next_position(siblings: &[Card]) -> u32 {
    siblings
        .iter()
        .map(|c| c.position)
        .max()
        .map_or(0, |p| p.saturating_add(1))
}

// ---------------------------------------------------------------------
// Split and create
// ---------------------------------------------------------------------

/// One child of a split: the caller mints the id ([`split_card_id`] for
/// the seed loader), names it, and may hand it to someone. CPE text is
/// not copied from the parent — a "default from parent" is a hook for
/// later.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SplitSpec {
    pub id: Uuid,
    pub name: String,
    pub owner_id: Option<Uuid>,
    pub conception: String,
    pub planning: String,
    pub execution: String,
    pub minimum_standard_of_care: Vec<String>,
    pub notes: String,
}

/// Query 11: create `specs` as children of `parent_id` — `Family`
/// cards, the parent's suit, positions continuing the parent's
/// existing children in `specs` order — then, if `parent_update` is
/// given, replace the parent with it (its id must be `parent_id`).
///
/// **Not atomic.** Writes are ordered so a crash anywhere leaves a
/// valid store: children are inserted one at a time, each already
/// pointing at the existing parent, and the parent is replaced last.
/// A crash after `k` children leaves `k` children under the unchanged
/// parent; a crash during the parent's replace leaves the children and
/// whichever version of the parent the insert log holds (`ADR-0049`'s
/// window: the log entry lands before the slot write, so at worst
/// `position` is the old value). A rerun with the same ids is refused
/// as `Duplicate` on the first child already present, so the caller
/// knows how far the previous attempt got.
pub fn split_card<S>(
    store: &mut S,
    parent_id: Uuid,
    specs: Vec<SplitSpec>,
    parent_update: Option<Card>,
) -> Result<Vec<Uuid>, CardError>
where
    S: GetById<Card> + Insert<Card> + Replace<Card> + Children<Card, Card, ParentCard>,
{
    split_card_with(store, parent_id, specs, parent_update, |_| {})
}

/// One durable step of [`split_card`], reported to the observer after
/// it has returned — what the crash-interruption test
/// (`tests/fair_play_crash.rs`) pauses on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitStep {
    ChildInserted(Uuid),
    ParentReplaced,
}

/// [`split_card`] with an observer called after each durable step.
pub fn split_card_with<S>(
    store: &mut S,
    parent_id: Uuid,
    specs: Vec<SplitSpec>,
    parent_update: Option<Card>,
    mut on_step: impl FnMut(SplitStep),
) -> Result<Vec<Uuid>, CardError>
where
    S: GetById<Card> + Insert<Card> + Replace<Card> + Children<Card, Card, ParentCard>,
{
    let parent = store.get(parent_id).ok_or(CardError::NotFound(parent_id))?;
    if let Some(update) = &parent_update {
        if update.id != parent_id {
            return Err(CardError::NotFound(update.id));
        }
    }
    let mut position = next_position(&children_ordered(store, parent_id));
    let mut created = Vec::with_capacity(specs.len());
    for spec in specs {
        let child = Card {
            id: spec.id,
            number: None,
            name: spec.name,
            suit: parent.suit,
            parent_card_id: Some(parent_id),
            position,
            owner_id: spec.owner_id,
            conception: spec.conception,
            planning: spec.planning,
            execution: spec.execution,
            minimum_standard_of_care: spec.minimum_standard_of_care,
            notes: spec.notes,
            origin: Origin::Family,
            baseline_id: None,
        };
        insert_card(store, child)?;
        on_step(SplitStep::ChildInserted(spec.id));
        created.push(spec.id);
        position = position.saturating_add(1);
    }
    if let Some(update) = parent_update {
        replace_card(store, update)?;
        on_step(SplitStep::ParentReplaced);
    }
    Ok(created)
}

/// Query 15's input: a brand-new family card, not a split.
#[derive(Debug, Clone, PartialEq)]
pub struct NewCustomCard {
    pub id: Uuid,
    pub name: String,
    pub suit: Suit,
    pub parent_card_id: Option<Uuid>,
    pub owner_id: Option<Uuid>,
    pub conception: String,
    pub planning: String,
    pub execution: String,
    pub minimum_standard_of_care: Vec<String>,
    pub notes: String,
}

/// Query 15: create a `Family` card, optionally under a parent and
/// optionally owned; its position follows its siblings (the top-level
/// cards, when it has no parent — a scan, as roots have no index).
pub fn create_custom_card<S>(store: &mut S, new: NewCustomCard) -> Result<(), CardError>
where
    S: GetById<Card> + Insert<Card> + AllIds<Card> + Children<Card, Card, ParentCard>,
{
    let siblings = match new.parent_card_id {
        Some(parent) => children_ordered(store, parent),
        None => records(store, store.all_ids())
            .into_iter()
            .filter(|c| c.parent_card_id.is_none())
            .collect(),
    };
    insert_card(
        store,
        Card {
            id: new.id,
            number: None,
            name: new.name,
            suit: new.suit,
            parent_card_id: new.parent_card_id,
            position: next_position(&siblings),
            owner_id: new.owner_id,
            conception: new.conception,
            planning: new.planning,
            execution: new.execution,
            minimum_standard_of_care: new.minimum_standard_of_care,
            notes: new.notes,
            origin: Origin::Family,
            baseline_id: None,
        },
    )
}

// ---------------------------------------------------------------------
// State: Original / Edited / Custom
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CardState {
    /// A deck card whose six text fields equal its baseline's.
    Original,
    /// A deck card with at least one of the six changed.
    Edited,
    /// A family card.
    Custom,
}

impl CardState {
    pub const ALL: [CardState; 3] = [CardState::Original, CardState::Edited, CardState::Custom];
}

/// Query 13: the fields on which `card` differs from `baseline`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDiff {
    pub field: TextField,
    pub card: String,
    pub baseline: String,
}

pub fn baseline_diff(card: &Card, baseline: &CardDefault) -> Vec<FieldDiff> {
    TextField::ALL
        .into_iter()
        .filter_map(|field| {
            let (c, b) = (card.text_field(field), baseline.text_field(field));
            (c != b).then_some(FieldDiff {
                field,
                card: c,
                baseline: b,
            })
        })
        .collect()
}

/// Query 12's test on one record: `Custom` for a family card; else
/// `Original` or `Edited` by the six-field comparison. A deck card
/// whose baseline is missing is an error, not a guess.
pub fn card_state(card: &Card, baseline: Option<&CardDefault>) -> Result<CardState, CardError> {
    if card.origin == Origin::Family {
        return Ok(CardState::Custom);
    }
    let baseline_id = card.baseline_id.ok_or(CardError::OriginMismatch(card.id))?;
    let baseline = baseline.ok_or(CardError::BaselineMissing(card.id, baseline_id))?;
    if baseline_diff(card, baseline).is_empty() {
        Ok(CardState::Original)
    } else {
        Ok(CardState::Edited)
    }
}

/// Query 13: a card's baseline — `Parent<Card, BaselineOf>` resolved
/// in the `CardDefault` store. `Ok(None)` for a family card.
pub fn baseline_of<C, D>(
    cards: &C,
    defaults: &D,
    id: Uuid,
) -> Result<Option<CardDefault>, CardError>
where
    C: GetById<Card>,
    D: GetById<CardDefault>,
{
    let card = cards.get(id).ok_or(CardError::NotFound(id))?;
    let Some(baseline_id) = card.baseline_id else {
        return Ok(None);
    };
    defaults
        .get(baseline_id)
        .map(Some)
        .ok_or(CardError::BaselineMissing(id, baseline_id))
}

pub fn state_of<C, D>(cards: &C, defaults: &D, id: Uuid) -> Result<CardState, CardError>
where
    C: GetById<Card>,
    D: GetById<CardDefault>,
{
    let card = cards.get(id).ok_or(CardError::NotFound(id))?;
    let baseline = baseline_of(cards, defaults, id)?;
    card_state(&card, baseline.as_ref())
}

/// Query 12: every card in `state` — a scan of the card table (an
/// `origin` index would only split `Custom` from the rest; `Original`
/// vs `Edited` needs the baseline either way). Measured in the ADR.
pub fn cards_by_state<C, D>(
    cards: &C,
    defaults: &D,
    state: CardState,
) -> Result<Vec<Uuid>, CardError>
where
    C: AllIds<Card> + GetById<Card>,
    D: GetById<CardDefault>,
{
    let mut out = Vec::new();
    for card in records(cards, cards.all_ids()) {
        let baseline = baseline_of(cards, defaults, card.id)?;
        if card_state(&card, baseline.as_ref())? == state {
            out.push(card.id);
        }
    }
    Ok(out)
}

/// Query 16: how many cards are in each state, overall and within each
/// suit — one scan.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StateCounts {
    pub total: BTreeMap<CardState, usize>,
    pub by_suit: BTreeMap<Suit, BTreeMap<CardState, usize>>,
}

pub fn state_counts<C, D>(cards: &C, defaults: &D) -> Result<StateCounts, CardError>
where
    C: AllIds<Card> + GetById<Card>,
    D: GetById<CardDefault>,
{
    let mut counts = StateCounts::default();
    for card in records(cards, cards.all_ids()) {
        let baseline = baseline_of(cards, defaults, card.id)?;
        let state = card_state(&card, baseline.as_ref())?;
        *counts.total.entry(state).or_insert(0) += 1;
        *counts
            .by_suit
            .entry(card.suit)
            .or_default()
            .entry(state)
            .or_insert(0) += 1;
    }
    Ok(counts)
}

/// Query 14: put the baseline's six fields back on a deck card, keeping
/// owner, parent, position and notes — one `Replace`. Returns the card
/// as written.
pub fn reset_to_baseline<C, D>(cards: &mut C, defaults: &D, id: Uuid) -> Result<Card, CardError>
where
    C: GetById<Card> + Replace<Card>,
    D: GetById<CardDefault>,
{
    let mut card = cards.get(id).ok_or(CardError::NotFound(id))?;
    let baseline = baseline_of(cards, defaults, id)?.ok_or(CardError::NotADeckCard(id))?;
    card.name = baseline.name;
    card.suit = baseline.suit;
    card.conception = baseline.conception;
    card.planning = baseline.planning;
    card.execution = baseline.execution;
    card.minimum_standard_of_care = baseline.minimum_standard_of_care;
    replace_card(cards, card.clone())?;
    Ok(card)
}

/// Small fixtures shared with dependents' tests (`rusty_multimodal_db`'s
/// wire adapters): a six-card deck, two people, a split spec.
pub mod fixtures {
    use super::*;

    pub fn default(n: u16, suit: Suit) -> CardDefault {
        CardDefault {
            id: card_default_id(n),
            number: n,
            name: format!("Card {n}"),
            suit,
            conception: format!("conceive {n}"),
            planning: format!("plan {n}"),
            execution: format!("execute {n}"),
            minimum_standard_of_care: vec![format!("std {n} a"), format!("std {n} b")],
        }
    }

    /// Six deck cards: 1–3 Home, 4–5 Out, 6 Wild.
    pub fn deck() -> Vec<CardDefault> {
        vec![
            default(1, Suit::Home),
            default(2, Suit::Home),
            default(3, Suit::Home),
            default(4, Suit::Out),
            default(5, Suit::Out),
            default(6, Suit::Wild),
        ]
    }

    pub fn people() -> Vec<Person> {
        vec![
            Person {
                id: person_id("Ada"),
                name: "Ada".into(),
                player: 1,
            },
            Person {
                id: person_id("Bob"),
                name: "Bob".into(),
                player: 2,
            },
        ]
    }

    pub fn spec(path: &str, name: &str, owner: Option<Uuid>) -> SplitSpec {
        SplitSpec {
            id: split_card_id(path),
            name: name.into(),
            owner_id: owner,
            minimum_standard_of_care: vec![format!("{name} done")],
            ..SplitSpec::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{deck, default, people, spec};
    use super::*;
    use rusty_multimodal_db_engine::generic::production::GenericProductionStore;

    /// A fresh, uniquely named directory under the OS temp dir.
    fn fresh_temp_dir(label: &str) -> std::io::Result<std::path::PathBuf> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "rusty_fair_play_domain_{label}_{}_{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    struct Fixture {
        dir: std::path::PathBuf,
        cards: CardProductionStack,
        defaults: CardDefaultProductionStack,
        people: PersonProductionStack,
    }

    fn fixture(label: &str) -> Fixture {
        let dir = fresh_temp_dir(label).unwrap();
        let defaults =
            create_card_default_production_stack(deck(), &dir.join("defaults.mmap")).unwrap();
        let cards = create_card_production_stack(
            deck().iter().map(CardDefault::to_card).collect(),
            &dir.join("cards.mmap"),
        )
        .unwrap();
        let people = create_person_production_stack(people(), &dir.join("people.mmap")).unwrap();
        Fixture {
            dir,
            cards,
            defaults,
            people,
        }
    }

    fn ada() -> Uuid {
        person_id("Ada")
    }
    fn bob() -> Uuid {
        person_id("Bob")
    }
    fn n(n: u16) -> Uuid {
        deck_card_id(n)
    }

    #[test]
    fn suit_and_origin_round_trip_every_variant() {
        for suit in Suit::ALL {
            assert_eq!(suit_from_u32(suit_to_u32(suit)), Some(suit));
            assert_eq!(Suit::parse(suit.as_str()), Some(suit));
        }
        assert_eq!(suit_from_u32(6), None);
        assert_eq!(Suit::parse("home"), None, "exact spelling");
        for origin in [Origin::Deck, Origin::Family] {
            assert_eq!(origin_from_u32(origin_to_u32(origin)), Some(origin));
        }
        assert_eq!(origin_from_u32(2), None);
    }

    #[test]
    fn deterministic_ids_are_stable_and_namespaced() {
        assert_eq!(deck_card_id(17), deck_card_id(17));
        assert_ne!(deck_card_id(17), card_default_id(17));
        assert_ne!(deck_card_id(17), deck_card_id(18));
        assert_eq!(person_id(" Ada "), person_id("Ada"), "trimmed");
        assert_ne!(split_card_id("17/Bathrooms"), split_card_id("17/Floors"));
    }

    /// Queries 1–5: held by, unassigned (all and leaf), by suit, by
    /// number, balance (all and leaf-only).
    #[test]
    fn held_by_unassigned_by_suit_by_number_and_balance() {
        let mut f = fixture("fp_queries");
        reassign_card(&mut f.cards, n(1), Some(ada())).unwrap();
        reassign_card(&mut f.cards, n(2), Some(ada())).unwrap();
        reassign_card(&mut f.cards, n(4), Some(bob())).unwrap();
        // Split 1 into two leaves: Ada keeps one, Bob gets the other.
        split_card(
            &mut f.cards,
            n(1),
            vec![spec("1/a", "a", Some(ada())), spec("1/b", "b", Some(bob()))],
            None,
        )
        .unwrap();

        let mut ada_held = cards_held_by(&f.cards, ada());
        ada_held.sort();
        let mut expect = vec![n(1), n(2), split_card_id("1/a")];
        expect.sort();
        assert_eq!(ada_held, expect, "at any level of the tree");

        let mut unassigned = unassigned_cards(&f.cards);
        unassigned.sort();
        let mut expect = vec![n(3), n(5), n(6)];
        expect.sort();
        assert_eq!(unassigned, expect);
        assert_eq!(
            unassigned_leaf_cards(&f.cards).len(),
            3,
            "all three are leaves"
        );
        reassign_card(&mut f.cards, split_card_id("1/b"), None).unwrap();
        assert_eq!(unassigned_cards(&f.cards).len(), 4);
        assert_eq!(unassigned_leaf_cards(&f.cards).len(), 4);

        let mut home = cards_by_suit(&f.cards, Suit::Home);
        home.sort();
        let mut expect = vec![n(1), n(2), n(3), split_card_id("1/a"), split_card_id("1/b")];
        expect.sort();
        assert_eq!(home, expect, "split-off cards copy the suit");
        assert_eq!(cards_by_number(&f.cards, 4), vec![n(4)]);
        assert!(cards_by_number(&f.cards, 99).is_empty());

        assert_eq!(
            balance(&f.cards, &[ada(), bob()], false),
            vec![(ada(), 3), (bob(), 1)]
        );
        assert_eq!(
            balance(&f.cards, &[ada(), bob()], true),
            vec![(ada(), 2), (bob(), 1)],
            "the split parent is not a leaf"
        );
    }

    /// Queries 7–10: ordered children, chain to root, nested tree,
    /// leaves under — on a 4-level tree with a different owner at
    /// every level — and the owner-coverage view.
    #[test]
    fn four_level_tree_with_different_owners_at_each_level() {
        let mut f = fixture("fp_tree");
        let carol = person_id("Carol");
        reassign_card(&mut f.cards, n(2), Some(ada())).unwrap();
        split_card(
            &mut f.cards,
            n(2),
            vec![spec("2/l1", "l1", Some(bob()))],
            None,
        )
        .unwrap();
        split_card(
            &mut f.cards,
            split_card_id("2/l1"),
            vec![spec("2/l1/l2", "l2", Some(carol))],
            None,
        )
        .unwrap();
        split_card(
            &mut f.cards,
            split_card_id("2/l1/l2"),
            vec![
                spec("2/l1/l2/x", "x", None),
                spec("2/l1/l2/y", "y", Some(ada())),
            ],
            None,
        )
        .unwrap();
        let (l1, l2, x, y) = (
            split_card_id("2/l1"),
            split_card_id("2/l1/l2"),
            split_card_id("2/l1/l2/x"),
            split_card_id("2/l1/l2/y"),
        );

        let kids = children_ordered(&f.cards, l2);
        assert_eq!(kids.iter().map(|c| c.id).collect::<Vec<_>>(), vec![x, y]);
        assert_eq!(
            kids.iter().map(|c| c.position).collect::<Vec<_>>(),
            vec![0, 1]
        );
        set_position(&mut f.cards, x, 5).unwrap();
        assert_eq!(
            children_ordered(&f.cards, l2)
                .iter()
                .map(|c| c.id)
                .collect::<Vec<_>>(),
            vec![y, x],
            "position moved in place"
        );

        assert_eq!(chain_to_root(&f.cards, y).unwrap(), vec![y, l2, l1, n(2)]);
        assert_eq!(chain_to_root(&f.cards, n(2)).unwrap(), vec![n(2)]);
        assert_eq!(root_card_id(&f.cards, x).unwrap(), n(2));
        assert_eq!(
            chain_to_root(&f.cards, Uuid::from_u128(999)),
            Err(NotFound(Uuid::from_u128(999)))
        );

        let tree = card_tree(&f.cards, n(2)).unwrap();
        assert_eq!(tree.card.owner_id, Some(ada()));
        assert_eq!(tree.children.len(), 1);
        assert_eq!(tree.children[0].card.owner_id, Some(bob()));
        assert_eq!(tree.children[0].children[0].card.owner_id, Some(carol));
        let leaves = &tree.children[0].children[0].children;
        assert_eq!(
            leaves.iter().map(|t| t.card.id).collect::<Vec<_>>(),
            vec![y, x]
        );
        assert_eq!(tree.flatten().len(), 5);

        assert_eq!(leaf_cards_under(&f.cards, n(2)).unwrap(), vec![y, x]);
        assert_eq!(
            leaf_cards_under(&f.cards, x).unwrap(),
            vec![x],
            "a leaf is its own leaf"
        );
        let coverage = owner_coverage_under(&f.cards, n(2)).unwrap();
        assert_eq!(coverage.get(&Some(ada())), Some(&1));
        assert_eq!(coverage.get(&None), Some(&1));
        assert_eq!(coverage.get(&Some(bob())), None, "holds no leaf");

        assert_eq!(
            balance(&f.cards, &[ada(), bob(), carol], false),
            vec![(ada(), 2), (bob(), 1), (carol, 1)]
        );
        assert_eq!(
            balance(&f.cards, &[ada(), bob(), carol], true),
            vec![(ada(), 1), (bob(), 0), (carol, 0)],
            "leaf-only differs from all-cards"
        );
    }

    /// Query 11: split-off cards are `Family`, numberless, baseline-less,
    /// the parent's suit, positioned after existing children; the parent
    /// update lands last; a second split continues the positions; the
    /// parent stays `Original`.
    #[test]
    fn split_card_creates_custom_children_then_updates_the_parent() {
        let mut f = fixture("fp_split");
        let mut parent = f.cards.get(n(1)).unwrap();
        parent.notes = "split 2026".into();
        parent.owner_id = Some(ada());
        let made = split_card(
            &mut f.cards,
            n(1),
            vec![spec("1/a", "a", Some(bob())), spec("1/b", "b", None)],
            Some(parent),
        )
        .unwrap();
        assert_eq!(made, vec![split_card_id("1/a"), split_card_id("1/b")]);
        let a = f.cards.get(split_card_id("1/a")).unwrap();
        assert_eq!(a.origin, Origin::Family);
        assert_eq!(a.number, None);
        assert_eq!(a.baseline_id, None);
        assert_eq!(a.suit, Suit::Home);
        assert_eq!(a.parent_card_id, Some(n(1)));
        assert_eq!(a.position, 0);
        assert_eq!(a.conception, "", "CPE is not copied from the parent");
        let parent = f.cards.get(n(1)).unwrap();
        assert_eq!(parent.notes, "split 2026");
        assert_eq!(parent.owner_id, Some(ada()));
        assert_eq!(
            state_of(&f.cards, &f.defaults, n(1)).unwrap(),
            CardState::Original
        );
        assert_eq!(
            state_of(&f.cards, &f.defaults, a.id).unwrap(),
            CardState::Custom
        );

        split_card(&mut f.cards, n(1), vec![spec("1/c", "c", None)], None).unwrap();
        assert_eq!(f.cards.get(split_card_id("1/c")).unwrap().position, 2);

        // A parent update whose id is not the parent is refused before
        // any child is written.
        let stray = f.cards.get(n(2)).unwrap();
        let err = split_card(
            &mut f.cards,
            n(1),
            vec![spec("1/d", "d", None)],
            Some(stray),
        )
        .unwrap_err();
        assert!(
            matches!(err, CardError::NotFound(id) if id == n(2)),
            "{err}"
        );
        assert!(f.cards.get(split_card_id("1/d")).is_none());
        // Unknown parent.
        assert!(matches!(
            split_card(&mut f.cards, Uuid::from_u128(7), vec![], None),
            Err(CardError::NotFound(id)) if id == Uuid::from_u128(7)
        ));
        // A rerun with the same child id is a Duplicate, so the caller
        // can tell how far an interrupted split got.
        assert!(matches!(
            split_card(&mut f.cards, n(1), vec![spec("1/a", "a", None)], None),
            Err(CardError::Insert(InsertError::Duplicate(_)))
        ));
    }

    /// Queries 12, 13, 14, 15, 16: state derivation and everything that
    /// changes it.
    #[test]
    fn state_derivation_diff_reset_custom_and_counts() {
        let mut f = fixture("fp_state");
        fn all(f: &Fixture, state: CardState) -> Vec<Uuid> {
            let mut v = cards_by_state(&f.cards, &f.defaults, state).unwrap();
            v.sort();
            v
        }
        // Freshly seeded: all Original.
        assert_eq!(all(&f, CardState::Original).len(), 6);
        assert!(all(&f, CardState::Edited).is_empty());
        assert!(all(&f, CardState::Custom).is_empty());

        // Play state and notes never make a card Edited.
        let mut c = f.cards.get(n(1)).unwrap();
        c.notes = "ours".into();
        c.owner_id = Some(ada());
        c.position = 42;
        replace_card(&mut f.cards, c).unwrap();
        let mut c = f.cards.get(n(2)).unwrap();
        c.parent_card_id = Some(n(1));
        replace_card(&mut f.cards, c).unwrap();
        assert_eq!(all(&f, CardState::Original).len(), 6);

        // Each of the six text fields makes it Edited; changing it back
        // makes it Original again.
        for field in TextField::ALL {
            let before = f.cards.get(n(3)).unwrap();
            let mut edited = before.clone();
            match field {
                TextField::Name => edited.name.push('!'),
                TextField::Suit => edited.suit = Suit::Magic,
                TextField::Conception => edited.conception.push('!'),
                TextField::Planning => edited.planning.push('!'),
                TextField::Execution => edited.execution.push('!'),
                TextField::MinimumStandardOfCare => {
                    edited.minimum_standard_of_care.push("x".into())
                }
            }
            replace_card(&mut f.cards, edited).unwrap();
            assert_eq!(
                state_of(&f.cards, &f.defaults, n(3)).unwrap(),
                CardState::Edited,
                "{field:?}"
            );
            let diff = baseline_diff(
                &f.cards.get(n(3)).unwrap(),
                &f.defaults.get(card_default_id(3)).unwrap(),
            );
            assert_eq!(diff.len(), 1);
            assert_eq!(diff[0].field, field);
            assert_eq!(diff[0].baseline, default(3, Suit::Home).text_field(field));
            replace_card(&mut f.cards, before).unwrap();
            assert_eq!(
                state_of(&f.cards, &f.defaults, n(3)).unwrap(),
                CardState::Original,
                "{field:?}"
            );
        }

        // Reset restores all six and keeps owner, parent, position, notes.
        let mut c = f.cards.get(n(4)).unwrap();
        c.name = "Renamed".into();
        c.suit = Suit::Wild;
        c.conception = "c".into();
        c.planning = "p".into();
        c.execution = "e".into();
        c.minimum_standard_of_care = vec![];
        c.owner_id = Some(bob());
        c.parent_card_id = Some(n(1));
        c.position = 9;
        c.notes = "keep me".into();
        replace_card(&mut f.cards, c).unwrap();
        assert_eq!(
            state_of(&f.cards, &f.defaults, n(4)).unwrap(),
            CardState::Edited
        );
        assert_eq!(cards_by_suit(&f.cards, Suit::Wild).len(), 2);
        let reset = reset_to_baseline(&mut f.cards, &f.defaults, n(4)).unwrap();
        assert_eq!(f.cards.get(n(4)).unwrap(), reset);
        let baseline = default(4, Suit::Out);
        for field in TextField::ALL {
            assert_eq!(reset.text_field(field), baseline.text_field(field));
        }
        assert_eq!(reset.owner_id, Some(bob()));
        assert_eq!(reset.parent_card_id, Some(n(1)));
        assert_eq!(reset.position, 9);
        assert_eq!(reset.notes, "keep me");
        assert_eq!(
            state_of(&f.cards, &f.defaults, n(4)).unwrap(),
            CardState::Original
        );
        assert_eq!(
            cards_by_suit(&f.cards, Suit::Wild),
            vec![n(6)],
            "the index bucket moved back"
        );

        // A custom card, top-level and owned; another under a parent.
        let custom = |id: &str, parent| NewCustomCard {
            id: fair_play_id("custom", id),
            name: id.into(),
            suit: Suit::Magic,
            parent_card_id: parent,
            owner_id: Some(ada()),
            conception: "c".into(),
            planning: "p".into(),
            execution: "e".into(),
            minimum_standard_of_care: vec!["m".into()],
            notes: String::new(),
        };
        create_custom_card(&mut f.cards, custom("Dog walking", None)).unwrap();
        create_custom_card(&mut f.cards, custom("Under 6", Some(n(6)))).unwrap();
        let top = f.cards.get(fair_play_id("custom", "Dog walking")).unwrap();
        assert_eq!(top.origin, Origin::Family);
        assert_eq!(
            top.position, 43,
            "after the top-level cards' max position (42)"
        );
        assert_eq!(
            f.cards
                .get(fair_play_id("custom", "Under 6"))
                .unwrap()
                .position,
            0
        );
        assert_eq!(all(&f, CardState::Custom).len(), 2);
        assert!(matches!(
            reset_to_baseline(&mut f.cards, &f.defaults, top.id),
            Err(CardError::NotADeckCard(id)) if id == top.id
        ));
        assert_eq!(baseline_of(&f.cards, &f.defaults, top.id).unwrap(), None);

        // Counts, overall and per suit.
        let mut c = f.cards.get(n(5)).unwrap();
        c.planning = "changed".into();
        replace_card(&mut f.cards, c).unwrap();
        let counts = state_counts(&f.cards, &f.defaults).unwrap();
        assert_eq!(counts.total[&CardState::Original], 5);
        assert_eq!(counts.total[&CardState::Edited], 1);
        assert_eq!(counts.total[&CardState::Custom], 2);
        assert_eq!(counts.by_suit[&Suit::Out][&CardState::Edited], 1);
        assert_eq!(counts.by_suit[&Suit::Out][&CardState::Original], 1);
        assert_eq!(counts.by_suit[&Suit::Magic][&CardState::Custom], 2);
        assert_eq!(counts.by_suit[&Suit::Home][&CardState::Original], 3);
    }

    /// Persist, drop, reopen through the production store: ownership,
    /// tree structure, sibling order, every text field, origin, baseline
    /// link and baseline text survive, and state derivation agrees.
    #[test]
    fn persist_drop_and_reopen_through_the_production_store() {
        let f = fixture("fp_reopen");
        let Fixture {
            dir,
            cards,
            defaults,
            people,
        } = f;
        let (cards, defaults, people) = (
            GenericProductionStore::new(cards),
            GenericProductionStore::new(defaults),
            GenericProductionStore::new(people),
        );
        cards.with_exclusive(|s| {
            reassign_card(s, n(1), Some(ada())).unwrap();
            let mut c = s.get(n(2)).unwrap();
            c.execution = "our way".into();
            c.notes = "note".into();
            replace_card(s, c).unwrap();
            split_card(
                s,
                n(1),
                vec![spec("1/b", "b", Some(bob())), spec("1/a", "a", None)],
                None,
            )
            .unwrap();
            set_position(s, split_card_id("1/b"), 7).unwrap();
        });
        let before =
            defaults.with_exclusive(|d| cards.with_exclusive(|s| state_counts(s, d).unwrap()));
        drop(cards);
        drop(defaults);
        drop(people);

        let cards = GenericProductionStore::new(
            open_card_production_stack_portable(&dir.join("cards.mmap")).unwrap(),
        );
        let defaults = GenericProductionStore::new(
            open_card_default_production_stack_portable(&dir.join("defaults.mmap")).unwrap(),
        );
        let people = GenericProductionStore::new(
            open_person_production_stack_portable(&dir.join("people.mmap")).unwrap(),
        );
        assert_eq!(people.get::<Person>(bob()).unwrap().name, "Bob");
        assert_eq!(
            people.filter_eq::<Person, NameField>(&"Ada".into()),
            vec![ada()]
        );
        let one = cards.get::<Card>(n(1)).unwrap();
        assert_eq!(one.owner_id, Some(ada()));
        assert_eq!(one.origin, Origin::Deck);
        assert_eq!(one.baseline_id, Some(card_default_id(1)));
        assert_eq!(one.minimum_standard_of_care, vec!["std 1 a", "std 1 b"]);
        let two = cards.get::<Card>(n(2)).unwrap();
        assert_eq!(two.execution, "our way");
        assert_eq!(two.notes, "note");
        let b = cards.get::<Card>(split_card_id("1/b")).unwrap();
        assert_eq!(b.owner_id, Some(bob()));
        assert_eq!(b.parent_card_id, Some(n(1)));
        assert_eq!(b.position, 7, "the slot survived");
        assert_eq!(b.minimum_standard_of_care, vec!["b done"]);
        defaults.with_exclusive(|d| {
            cards.with_exclusive(|s| {
                assert_eq!(
                    children_ordered(s, n(1))
                        .iter()
                        .map(|c| c.id)
                        .collect::<Vec<_>>(),
                    vec![split_card_id("1/a"), split_card_id("1/b")],
                    "sibling order"
                );
                assert_eq!(cards_held_by(s, bob()), vec![split_card_id("1/b")]);
                assert_eq!(
                    chain_to_root(s, split_card_id("1/a")).unwrap(),
                    vec![split_card_id("1/a"), n(1)]
                );
                assert_eq!(
                    d.get(card_default_id(2)).unwrap(),
                    default(2, Suit::Home),
                    "baseline text survived"
                );
                let after = state_counts(s, d).unwrap();
                assert_eq!(after, before);
                assert_eq!(after.total[&CardState::Edited], 1);
                assert_eq!(after.total[&CardState::Custom], 2);
                assert_eq!(state_of(s, d, n(2)).unwrap(), CardState::Edited);
            })
        });
    }
    /// The `origin == Deck` iff `number.is_some()` iff `baseline_id.is_some()`
    /// invariant, and `origin`/`number`/`baseline_id` immutability, are
    /// rejected on every write that goes through the domain — and bypassed
    /// by the raw `Insert`/`Replace` traits, which the stack cannot stop
    /// (the documented gap).
    #[test]
    fn origin_number_baseline_invariant_is_rejected_on_domain_writes_only() {
        let mut f = fixture("fp_invariant");
        let mut bad = f.cards.get(n(1)).unwrap();
        bad.id = Uuid::from_u128(0xbad);
        bad.number = None; // Deck without a number
        assert!(matches!(
            insert_card(&mut f.cards, bad.clone()),
            Err(CardError::OriginMismatch(_))
        ));
        bad.number = Some(1);
        bad.baseline_id = None; // Deck without a baseline
        assert!(matches!(
            insert_card(&mut f.cards, bad.clone()),
            Err(CardError::OriginMismatch(_))
        ));
        bad.origin = Origin::Family; // Family with a number
        assert!(matches!(
            insert_card(&mut f.cards, bad.clone()),
            Err(CardError::OriginMismatch(_))
        ));
        assert!(f.cards.get(bad.id).is_none(), "nothing written");

        let mut flipped = f.cards.get(n(1)).unwrap();
        flipped.origin = Origin::Family;
        flipped.number = None;
        flipped.baseline_id = None;
        assert!(matches!(
            replace_card(&mut f.cards, flipped.clone()),
            Err(CardError::ImmutableChanged(_))
        ));
        let mut renumbered = f.cards.get(n(1)).unwrap();
        renumbered.number = Some(50);
        assert!(matches!(
            replace_card(&mut f.cards, renumbered),
            Err(CardError::ImmutableChanged(_))
        ));
        assert_eq!(f.cards.get(n(1)).unwrap().origin, Origin::Deck);

        // The gap: the raw trait takes it.
        Replace::<Card>::replace(&mut f.cards, flipped).unwrap();
        assert_eq!(f.cards.get(n(1)).unwrap().origin, Origin::Family);
        bad.origin = Origin::Deck;
        bad.number = Some(1);
        bad.baseline_id = None;
        Insert::<Card>::insert(&mut f.cards, bad.clone()).unwrap();
        assert!(bad.validate().is_err());
        // A duplicate `number` is likewise invisible to the stack.
        assert_eq!(
            cards_by_number(&f.cards, 1),
            vec![bad.id],
            "n(1) is now Family; the duplicate shows"
        );
    }

    /// Cycle protection: a card cannot become its own ancestor, at any
    /// distance, and cannot be its own parent; a parent must exist.
    #[test]
    fn a_card_cannot_become_its_own_ancestor() {
        let mut f = fixture("fp_cycle");
        split_card(&mut f.cards, n(1), vec![spec("1/a", "a", None)], None).unwrap();
        split_card(
            &mut f.cards,
            split_card_id("1/a"),
            vec![spec("1/a/b", "b", None)],
            None,
        )
        .unwrap();
        let mut root = f.cards.get(n(1)).unwrap();
        root.parent_card_id = Some(split_card_id("1/a/b"));
        assert!(
            matches!(replace_card(&mut f.cards, root), Err(CardError::Cycle(id)) if id == n(1))
        );
        let mut a = f.cards.get(split_card_id("1/a")).unwrap();
        a.parent_card_id = Some(split_card_id("1/a/b"));
        assert!(matches!(
            replace_card(&mut f.cards, a),
            Err(CardError::Cycle(_))
        ));
        let mut selfie = f.cards.get(n(2)).unwrap();
        selfie.parent_card_id = Some(n(2));
        assert!(matches!(
            replace_card(&mut f.cards, selfie),
            Err(CardError::SelfParent(_))
        ));
        let mut dangling = f.cards.get(n(2)).unwrap();
        dangling.parent_card_id = Some(Uuid::from_u128(404));
        assert!(matches!(
            replace_card(&mut f.cards, dangling),
            Err(CardError::ParentNotFound { .. })
        ));
        assert_eq!(
            f.cards.get(n(1)).unwrap().parent_card_id,
            None,
            "nothing written"
        );
        // Re-parenting that makes no cycle is fine, and moves the index.
        let mut b = f.cards.get(split_card_id("1/a/b")).unwrap();
        b.parent_card_id = Some(n(2));
        replace_card(&mut f.cards, b).unwrap();
        assert_eq!(
            chain_to_root(&f.cards, split_card_id("1/a/b")).unwrap(),
            vec![split_card_id("1/a/b"), n(2)]
        );
        assert!(children_ordered(&f.cards, split_card_id("1/a")).is_empty());
        // And a cycle written through the raw trait still terminates every walk.
        let mut raw = f.cards.get(n(2)).unwrap();
        raw.parent_card_id = Some(split_card_id("1/a/b"));
        Replace::<Card>::replace(&mut f.cards, raw).unwrap();
        assert!(chain_to_root(&f.cards, n(2)).is_ok());
        assert!(card_tree(&f.cards, n(2)).is_ok());
    }

    /// Orphan handling when a parent is deleted (`ADR-0051`: records do
    /// not cascade): the children keep their `parent_card_id`, `Parent`
    /// names an id with no record, `chain_to_root` stops where the trail
    /// goes cold, and the children index still lists the orphans under
    /// the deleted id — live and after a reopen, since it is rebuilt from
    /// the children's own fields. This documents what a raw engine
    /// delete leaves, and why the domain's own `delete_card` refuses a
    /// parent.
    #[test]
    fn deleting_a_parent_leaves_its_children_pointing_at_nothing() {
        use rusty_multimodal_db_engine::generic::query::Delete;
        let mut f = fixture("fp_orphan");
        split_card(
            &mut f.cards,
            n(1),
            vec![spec("1/a", "a", Some(ada()))],
            None,
        )
        .unwrap();
        Delete::<Card>::delete(&mut f.cards, n(1)).unwrap();
        let a = f.cards.get(split_card_id("1/a")).unwrap();
        assert_eq!(a.parent_card_id, Some(n(1)), "the field stays");
        assert_eq!(
            Parent::<Card, ParentCard>::parent(&f.cards, a.id),
            Ok(Some(n(1)))
        );
        assert!(f.cards.get(n(1)).is_none());
        assert_eq!(
            chain_to_root(&f.cards, a.id).unwrap(),
            vec![a.id],
            "stops at the cold trail"
        );
        assert_eq!(
            children_ordered(&f.cards, n(1)).len(),
            1,
            "the orphan is still indexed under the deleted parent"
        );
        assert_eq!(
            cards_held_by(&f.cards, ada()),
            vec![a.id],
            "ownership is untouched"
        );
        // A later write of the orphan is refused until it is re-parented.
        assert!(matches!(
            reassign_card(&mut f.cards, a.id, None),
            Err(CardError::ParentNotFound { .. })
        ));
        let mut fixed = a.clone();
        fixed.parent_card_id = None;
        replace_card(&mut f.cards, fixed).unwrap();
        drop(f.cards);
        let reopened = open_card_production_stack_portable(&f.dir.join("cards.mmap")).unwrap();
        assert!(reopened.get(n(1)).is_none());
        assert_eq!(
            reopened.get(split_card_id("1/a")).unwrap().parent_card_id,
            None
        );
    }

    /// Reassign then reopen — the `Replace` path (`ADR-0049`): the new
    /// owner is in the insert log before the slot write, so the crash
    /// window that ADR names leaves at worst `position` (the slot) at its
    /// old value; here the whole record survives a portable reopen and
    /// the owner index is rebuilt to match.
    #[test]
    fn reassign_then_reopen_keeps_the_new_owner() {
        let f = fixture("fp_reassign");
        let Fixture { dir, cards, .. } = f;
        let cards = GenericProductionStore::new(cards);
        cards.with_exclusive(|s| {
            reassign_card(s, n(1), Some(ada())).unwrap();
            reassign_card(s, n(2), Some(ada())).unwrap();
            reassign_card(s, n(1), Some(bob())).unwrap();
            reassign_card(s, n(2), None).unwrap();
            assert!(matches!(
                reassign_card(s, Uuid::from_u128(9), None),
                Err(CardError::NotFound(_))
            ));
        });
        drop(cards);
        let reopened = open_card_production_stack_portable(&dir.join("cards.mmap")).unwrap();
        assert_eq!(reopened.get(n(1)).unwrap().owner_id, Some(bob()));
        assert_eq!(reopened.get(n(2)).unwrap().owner_id, None);
        assert_eq!(cards_held_by(&reopened, bob()), vec![n(1)]);
        assert!(cards_held_by(&reopened, ada()).is_empty());
        assert_eq!(
            balance(&reopened, &[ada(), bob()], true),
            vec![(ada(), 0), (bob(), 1)]
        );
    }

    #[test]
    fn delete_unsplit_delete_person_and_reorder() {
        let mut f = fixture("fp_delete");
        split_card(
            &mut f.cards,
            n(1),
            vec![spec("1/a", "a", Some(ada())), spec("1/b", "b", None)],
            None,
        )
        .unwrap();
        split_card(
            &mut f.cards,
            split_card_id("1/a"),
            vec![spec("1/a/x", "x", None)],
            None,
        )
        .unwrap();
        let (a, b, x) = (
            split_card_id("1/a"),
            split_card_id("1/b"),
            split_card_id("1/a/x"),
        );

        // A parent is never deleted from under its children.
        assert!(matches!(
            delete_card(&mut f.cards, n(1)),
            Err(CardError::HasChildren(id)) if id == n(1)
        ));
        assert!(matches!(
            delete_card(&mut f.cards, Uuid::from_u128(5)),
            Err(CardError::NotFound(_))
        ));
        // Reorder: the set must match exactly.
        assert!(matches!(
            reorder_children(&mut f.cards, n(1), &[a]),
            Err(CardError::BadOrder(_))
        ));
        assert!(matches!(
            reorder_children(&mut f.cards, n(1), &[a, a]),
            Err(CardError::BadOrder(_))
        ));
        assert!(matches!(
            reorder_children(&mut f.cards, n(1), &[a, b, x]),
            Err(CardError::BadOrder(_))
        ));
        reorder_children(&mut f.cards, n(1), &[b, a]).unwrap();
        assert_eq!(
            children_ordered(&f.cards, n(1))
                .iter()
                .map(|c| c.id)
                .collect::<Vec<_>>(),
            vec![b, a]
        );
        assert_eq!(f.cards.get(b).unwrap().position, 0);
        assert_eq!(f.cards.get(a).unwrap().position, 1);

        // A leaf goes; a person holding a card stays.
        delete_card(&mut f.cards, b).unwrap();
        assert!(f.cards.get(b).is_none());
        assert!(matches!(
            delete_person(&mut f.people, &f.cards, ada()),
            Err(CardError::HoldsCards { person, cards: 1 }) if person == ada()
        ));
        delete_person(&mut f.people, &f.cards, bob()).unwrap();
        assert!(f.people.get(bob()).is_none());
        assert!(matches!(
            delete_person(&mut f.people, &f.cards, bob()),
            Err(CardError::NotFound(_))
        ));

        // Unsplit removes the subtree deepest first and keeps the parent.
        assert_eq!(unsplit_card(&mut f.cards, n(1)).unwrap(), vec![x, a]);
        assert!(f.cards.get(a).is_none() && f.cards.get(x).is_none());
        assert!(f.cards.get(n(1)).is_some());
        assert!(is_leaf(&f.cards, n(1)));
        assert!(unsplit_card(&mut f.cards, n(1)).unwrap().is_empty());
        assert!(cards_held_by(&f.cards, ada()).is_empty());
        delete_person(&mut f.people, &f.cards, ada()).unwrap();
    }
}
