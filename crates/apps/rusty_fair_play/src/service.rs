//! The rules over the three stacks, with no I/O beyond the data directory
//! it opens: every operation the UI needs, as a method that returns the
//! domain's own types plus the derived state. The router (`api`) turns
//! these into JSON; nothing here knows about HTTP.

use rusty_fair_play_domain::seed::{self, SeedData, SeedError, SeedReport};
use rusty_fair_play_domain::{
    baseline_diff, baseline_of, card_state, create_custom_card, deck_card_id, fair_play_id,
    open_or_create_card_default_production_stack, open_or_create_card_production_stack,
    open_or_create_person_production_stack, person_id, replace_card, reset_to_baseline,
    set_position, split_card, Card, CardDefault, CardDefaultProductionStack, CardError,
    CardProductionStack, CardState, FieldDiff, NewCustomCard, Person, PersonProductionStack,
    SplitSpec, Suit, CARD_DEFAULT_FILE, CARD_FILE, PERSON_FILE,
};
use rusty_multimodal_db_engine::dir_lock::{DirLock, DirLockError};
use rusty_multimodal_db_engine::durability::DurabilityError;
use rusty_multimodal_db_engine::generic::query::{AllIds, GetById};
use rusty_multimodal_db_engine::generic::{InsertError, ReplaceError};
use std::path::Path;
use uuid::Uuid;

/// The lock file in the data directory.
pub const LOCK_FILE: &str = "store.lock";
/// Longest text a single field accepts; a card is a card, not a document.
pub const MAX_TEXT_LEN: usize = 10_000;
/// Most standards one card lists.
pub const MAX_STANDARDS: usize = 50;

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("{0} not found")]
    NotFound(&'static str),
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Conflict(String),
    #[error("storage: {0}")]
    Storage(String),
}

impl From<DurabilityError> for ServiceError {
    fn from(e: DurabilityError) -> Self {
        Self::Storage(e.to_string())
    }
}

impl From<DirLockError> for ServiceError {
    fn from(e: DirLockError) -> Self {
        Self::Storage(e.to_string())
    }
}

impl From<CardError> for ServiceError {
    fn from(e: CardError) -> Self {
        match e {
            CardError::NotFound(_) | CardError::Replace(ReplaceError::NotFound(_)) => {
                Self::NotFound("card")
            }
            CardError::Insert(InsertError::Duplicate(_)) => {
                Self::Conflict("a card with that id already exists".into())
            }
            CardError::Insert(InsertError::Durability(e))
            | CardError::Replace(ReplaceError::Durability(e)) => Self::Storage(e.to_string()),
            other => Self::Invalid(other.to_string()),
        }
    }
}

impl From<SeedError> for ServiceError {
    fn from(e: SeedError) -> Self {
        match e {
            SeedError::Card(e) => e.into(),
            SeedError::Store(e) => e.into(),
            SeedError::Io { .. } => Self::Storage(e.to_string()),
            SeedError::Row { .. } | SeedError::File { .. } => Self::Invalid(e.to_string()),
        }
    }
}

pub type Result<T> = std::result::Result<T, ServiceError>;

/// A card with its derived state — what every read returns.
#[derive(Debug, Clone, PartialEq)]
pub struct CardView {
    pub card: Card,
    pub state: CardState,
}

/// Everything the UI needs to boot: one read.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub people: Vec<Person>,
    pub cards: Vec<CardView>,
}

/// A field that is absent keeps its value; `owner_id`/`parent_card_id`
/// distinguish "keep" (`None`) from "clear" (`Some(None)`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CardPatch {
    pub name: Option<String>,
    pub suit: Option<Suit>,
    pub conception: Option<String>,
    pub planning: Option<String>,
    pub execution: Option<String>,
    pub minimum_standard_of_care: Option<Vec<String>>,
    pub notes: Option<String>,
    pub owner_id: Option<Option<Uuid>>,
    pub parent_card_id: Option<Option<Uuid>>,
    pub position: Option<u32>,
}

/// A brand-new family card.
#[derive(Debug, Clone, PartialEq)]
pub struct NewCard {
    pub id: Option<Uuid>,
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

/// One child of a split; CPE text is optional and empty by default.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SplitChild {
    pub id: Option<Uuid>,
    pub name: String,
    pub owner_id: Option<Uuid>,
    pub conception: String,
    pub planning: String,
    pub execution: String,
    pub minimum_standard_of_care: Vec<String>,
    pub notes: String,
}

/// Split a card: the children, and optionally the parent's new owner
/// and notes (`None` keeps each).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SplitRequest {
    pub children: Vec<SplitChild>,
    pub owner_id: Option<Option<Uuid>>,
    pub notes: Option<String>,
}

pub struct Split {
    pub parent: CardView,
    pub children: Vec<CardView>,
}

pub struct Service {
    _lock: DirLock,
    people: PersonProductionStack,
    defaults: CardDefaultProductionStack,
    cards: CardProductionStack,
}

impl Service {
    /// Open or create the three stacks under `dir`, holding its lock for
    /// the service's lifetime.
    pub fn open(dir: &Path) -> Result<Self> {
        let lock = DirLock::acquire(dir, LOCK_FILE)?;
        Ok(Self {
            _lock: lock,
            people: open_or_create_person_production_stack(&dir.join(PERSON_FILE))?,
            defaults: open_or_create_card_default_production_stack(&dir.join(CARD_DEFAULT_FILE))?,
            cards: open_or_create_card_production_stack(&dir.join(CARD_FILE))?,
        })
    }

    fn view(&self, card: Card) -> Result<CardView> {
        let baseline = baseline_of(&self.cards, &self.defaults, card.id)?;
        let state = card_state(&card, baseline.as_ref())?;
        Ok(CardView { card, state })
    }

    fn card_view(&self, id: Uuid) -> Result<CardView> {
        let card = self.cards.get(id).ok_or(ServiceError::NotFound("card"))?;
        self.view(card)
    }

    // ---- reads ---------------------------------------------------------

    pub fn snapshot(&self) -> Result<Snapshot> {
        let mut people: Vec<Person> = self
            .people
            .all_ids()
            .into_iter()
            .filter_map(|id| self.people.get(id))
            .collect();
        people.sort_by_key(|p| (p.player, p.name.clone()));
        let mut cards = Vec::new();
        for id in self.cards.all_ids() {
            if let Some(card) = self.cards.get(id) {
                cards.push(self.view(card)?);
            }
        }
        cards.sort_by_key(|v| {
            (
                v.card.number.unwrap_or(u16::MAX),
                v.card.position,
                v.card.id,
            )
        });
        Ok(Snapshot { people, cards })
    }

    pub fn card(&self, id: Uuid) -> Result<CardView> {
        self.card_view(id)
    }

    pub fn person(&self, id: Uuid) -> Result<Person> {
        self.people.get(id).ok_or(ServiceError::NotFound("person"))
    }

    /// A card's baseline and the fields it differs on; `(None, [])` for a
    /// family card.
    pub fn baseline(&self, id: Uuid) -> Result<(Option<CardDefault>, Vec<FieldDiff>)> {
        let card = self.cards.get(id).ok_or(ServiceError::NotFound("card"))?;
        let baseline = baseline_of(&self.cards, &self.defaults, id)?;
        let diff = baseline
            .as_ref()
            .map(|b| baseline_diff(&card, b))
            .unwrap_or_default();
        Ok((baseline, diff))
    }

    // ---- people --------------------------------------------------------

    pub fn create_person(&mut self, name: &str) -> Result<Person> {
        let name = clean_name(name)?;
        let id = person_id(&name);
        if self.people.get(id).is_some() {
            return Err(ServiceError::Conflict(format!("{name} already exists")));
        }
        let player = self
            .people
            .all_ids()
            .into_iter()
            .filter_map(|id| self.people.get(id))
            .map(|p| p.player)
            .max()
            .map_or(1, |n| n.saturating_add(1));
        let person = Person { id, name, player };
        self.people
            .insert(person.clone())
            .map_err(|e| ServiceError::Storage(e.to_string()))?;
        Ok(person)
    }

    pub fn rename_person(&mut self, id: Uuid, name: &str) -> Result<Person> {
        let mut person = self.person(id)?;
        person.name = clean_name(name)?;
        self.people
            .replace(person.clone())
            .map_err(|e| ServiceError::Storage(e.to_string()))?;
        Ok(person)
    }

    fn check_owner(&self, owner: Option<Uuid>) -> Result<()> {
        match owner {
            Some(id) if self.people.get(id).is_none() => {
                Err(ServiceError::Invalid("unknown owner".into()))
            }
            _ => Ok(()),
        }
    }

    // ---- cards ---------------------------------------------------------

    pub fn patch_card(&mut self, id: Uuid, patch: CardPatch) -> Result<CardView> {
        let mut card = self.cards.get(id).ok_or(ServiceError::NotFound("card"))?;
        if let Some(name) = patch.name {
            card.name = clean_name(&name)?;
        }
        if let Some(suit) = patch.suit {
            card.suit = suit;
        }
        if let Some(text) = patch.conception {
            card.conception = clean_text(text)?;
        }
        if let Some(text) = patch.planning {
            card.planning = clean_text(text)?;
        }
        if let Some(text) = patch.execution {
            card.execution = clean_text(text)?;
        }
        if let Some(list) = patch.minimum_standard_of_care {
            card.minimum_standard_of_care = clean_standards(list)?;
        }
        if let Some(text) = patch.notes {
            card.notes = clean_text(text)?;
        }
        if let Some(owner) = patch.owner_id {
            self.check_owner(owner)?;
            card.owner_id = owner;
        }
        if let Some(parent) = patch.parent_card_id {
            card.parent_card_id = parent;
        }
        if let Some(position) = patch.position {
            card.position = position;
        }
        replace_card(&mut self.cards, card)?;
        self.card_view(id)
    }

    pub fn create_card(&mut self, new: NewCard) -> Result<CardView> {
        self.check_owner(new.owner_id)?;
        let name = clean_name(&new.name)?;
        let id = new
            .id
            .unwrap_or_else(|| fair_play_id("custom", &format!("{name}:{}", now_nanos())));
        create_custom_card(
            &mut self.cards,
            NewCustomCard {
                id,
                name,
                suit: new.suit,
                parent_card_id: new.parent_card_id,
                owner_id: new.owner_id,
                conception: clean_text(new.conception)?,
                planning: clean_text(new.planning)?,
                execution: clean_text(new.execution)?,
                minimum_standard_of_care: clean_standards(new.minimum_standard_of_care)?,
                notes: clean_text(new.notes)?,
            },
        )?;
        self.card_view(id)
    }

    pub fn split(&mut self, id: Uuid, request: SplitRequest) -> Result<Split> {
        if request.children.is_empty() {
            return Err(ServiceError::Invalid(
                "a split needs at least one child".into(),
            ));
        }
        let mut parent = self.cards.get(id).ok_or(ServiceError::NotFound("card"))?;
        let mut specs = Vec::with_capacity(request.children.len());
        for child in request.children {
            self.check_owner(child.owner_id)?;
            let name = clean_name(&child.name)?;
            let child_id = child
                .id
                .unwrap_or_else(|| fair_play_id("split", &format!("{id}/{name}:{}", now_nanos())));
            specs.push(SplitSpec {
                id: child_id,
                name,
                owner_id: child.owner_id,
                conception: clean_text(child.conception)?,
                planning: clean_text(child.planning)?,
                execution: clean_text(child.execution)?,
                minimum_standard_of_care: clean_standards(child.minimum_standard_of_care)?,
                notes: clean_text(child.notes)?,
            });
        }
        let update = match (request.owner_id, request.notes) {
            (None, None) => None,
            (owner, notes) => {
                if let Some(owner) = owner {
                    self.check_owner(owner)?;
                    parent.owner_id = owner;
                }
                if let Some(notes) = notes {
                    parent.notes = clean_text(notes)?;
                }
                Some(parent)
            }
        };
        let created = split_card(&mut self.cards, id, specs, update)?;
        let children = created
            .into_iter()
            .map(|c| self.card_view(c))
            .collect::<Result<Vec<_>>>()?;
        Ok(Split {
            parent: self.card_view(id)?,
            children,
        })
    }

    pub fn reset(&mut self, id: Uuid) -> Result<CardView> {
        reset_to_baseline(&mut self.cards, &self.defaults, id)?;
        self.card_view(id)
    }

    pub fn set_position(&mut self, id: Uuid, position: u32) -> Result<()> {
        set_position(&mut self.cards, id, position).map_err(|_| ServiceError::NotFound("card"))
    }

    /// Load the deck that ships in the library; idempotent.
    pub fn seed_deck(&mut self) -> Result<SeedReport> {
        let data = SeedData {
            deck: seed::deck()?,
            ..SeedData::default()
        };
        Ok(seed::seed_into(
            &mut self.people,
            &mut self.defaults,
            &mut self.cards,
            &data,
        )?)
    }

    /// Whether the deck has been loaded: deck card 1 exists.
    pub fn has_deck(&self) -> bool {
        self.cards.get(deck_card_id(1)).is_some()
    }
}

fn now_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn clean_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ServiceError::Invalid("name must not be blank".into()));
    }
    if name.chars().count() > 200 {
        return Err(ServiceError::Invalid("name is too long".into()));
    }
    Ok(name.to_string())
}

fn clean_text(text: String) -> Result<String> {
    if text.chars().count() > MAX_TEXT_LEN {
        return Err(ServiceError::Invalid(format!(
            "text is longer than {MAX_TEXT_LEN} characters"
        )));
    }
    Ok(text.trim().to_string())
}

fn clean_standards(list: Vec<String>) -> Result<Vec<String>> {
    if list.len() > MAX_STANDARDS {
        return Err(ServiceError::Invalid(format!(
            "at most {MAX_STANDARDS} standards"
        )));
    }
    list.into_iter()
        .map(clean_text)
        .filter(|s| !matches!(s, Ok(s) if s.is_empty()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service() -> (tempfile::TempDir, Service) {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Service::open(dir.path()).unwrap();
        s.seed_deck().unwrap();
        (dir, s)
    }

    #[test]
    fn open_seeds_once_and_locks_the_directory() {
        let (dir, mut s) = service();
        assert!(s.has_deck());
        let again = s.seed_deck().unwrap();
        assert_eq!((again.cards.created, again.cards.existing), (0, 100));
        assert!(
            Service::open(dir.path()).is_err(),
            "a second service on the same directory is refused"
        );
        assert_eq!(s.snapshot().unwrap().cards.len(), 100);
    }

    #[test]
    fn deal_split_edit_reset_round_trip() {
        let (_d, mut s) = service();
        let ada = s.create_person("Ada").unwrap();
        let bob = s.create_person(" Bob ").unwrap();
        assert_eq!((ada.player, bob.player, bob.name.as_str()), (1, 2, "Bob"));
        assert!(matches!(
            s.create_person("Ada"),
            Err(ServiceError::Conflict(_))
        ));
        assert!(matches!(
            s.create_person("  "),
            Err(ServiceError::Invalid(_))
        ));

        let cleaning = deck_card_id(2);
        let dealt = s
            .patch_card(
                cleaning,
                CardPatch {
                    owner_id: Some(Some(ada.id)),
                    ..CardPatch::default()
                },
            )
            .unwrap();
        assert_eq!(dealt.card.owner_id, Some(ada.id));
        assert_eq!(dealt.state, CardState::Original, "ownership is play state");
        assert!(matches!(
            s.patch_card(
                cleaning,
                CardPatch {
                    owner_id: Some(Some(Uuid::from_u128(9))),
                    ..CardPatch::default()
                }
            ),
            Err(ServiceError::Invalid(_))
        ));

        let split = s
            .split(
                cleaning,
                SplitRequest {
                    children: vec![
                        SplitChild {
                            name: "Bathrooms".into(),
                            owner_id: Some(ada.id),
                            ..SplitChild::default()
                        },
                        SplitChild {
                            name: "Floors".into(),
                            owner_id: Some(bob.id),
                            ..SplitChild::default()
                        },
                    ],
                    owner_id: Some(None),
                    notes: Some("split".into()),
                },
            )
            .unwrap();
        assert_eq!(split.children.len(), 2);
        assert_eq!(split.children[1].state, CardState::Custom);
        assert_eq!(split.children[1].card.suit, Suit::Home);
        assert_eq!(split.parent.card.owner_id, None);
        assert_eq!(split.parent.card.notes, "split");

        let edited = s
            .patch_card(
                cleaning,
                CardPatch {
                    execution: Some("  our way  ".into()),
                    ..CardPatch::default()
                },
            )
            .unwrap();
        assert_eq!(
            (edited.state, edited.card.execution.as_str()),
            (CardState::Edited, "our way")
        );
        let (baseline, diff) = s.baseline(cleaning).unwrap();
        assert!(baseline.is_some());
        assert_eq!(diff.len(), 1);
        let reset = s.reset(cleaning).unwrap();
        assert_eq!(reset.state, CardState::Original);
        assert_eq!(reset.card.notes, "split", "notes survive a reset");
        assert!(matches!(
            s.reset(split.children[0].card.id),
            Err(ServiceError::Invalid(_))
        ));

        let custom = s
            .create_card(NewCard {
                id: None,
                name: "Dog walking".into(),
                suit: Suit::Out,
                parent_card_id: None,
                owner_id: Some(bob.id),
                conception: "c".into(),
                planning: "p".into(),
                execution: "e".into(),
                minimum_standard_of_care: vec!["twice a day".into(), "  ".into()],
                notes: String::new(),
            })
            .unwrap();
        assert_eq!(custom.state, CardState::Custom);
        assert_eq!(custom.card.minimum_standard_of_care, vec!["twice a day"]);
        assert_eq!(s.snapshot().unwrap().cards.len(), 103);
        assert!(matches!(
            s.card(Uuid::from_u128(1)),
            Err(ServiceError::NotFound("card"))
        ));
    }
}
