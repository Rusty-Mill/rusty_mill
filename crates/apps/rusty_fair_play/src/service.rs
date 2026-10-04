//! The rules over the three stacks, with no I/O beyond the data directory
//! it opens: every operation the UI needs, as a method that returns the
//! domain's own types plus the derived state. The router (`api`) turns
//! these into JSON; nothing here knows about HTTP.

use rusty_fair_play_domain::seed::{self, SeedData, SeedError, SeedReport};
use rusty_fair_play_domain::{
    baseline_diff, baseline_of, card_state, card_tree, create_custom_card, delete_card,
    delete_person, fair_play_id, open_or_create_card_default_production_stack,
    open_or_create_card_production_stack, open_or_create_person_production_stack, person_id,
    reorder_children, replace_card, reset_to_baseline, set_position, split_card, unsplit_card,
    Card, CardDefault, CardDefaultProductionStack, CardError, CardProductionStack, CardState,
    FieldDiff, NewCustomCard, Person, PersonProductionStack, SplitSpec, Suit, CARD_DEFAULT_FILE,
    CARD_FILE, LOCK_FILE, PERSON_FILE,
};
use rusty_multimodal_db_engine::dir_lock::{DirLock, DirLockError};
use rusty_multimodal_db_engine::durability::DurabilityError;
use rusty_multimodal_db_engine::generic::query::{AllIds, GetById};
use rusty_multimodal_db_engine::generic::{DeleteError, InsertError, ReplaceError};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Written once the deck has been loaded in full. Whether the deck was
/// loaded must not be read off the live cards: a card can be deleted,
/// and a first load can be interrupted part way.
pub const DECK_MARKER: &str = "deck.loaded";
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
            CardError::NotFound(_)
            | CardError::Replace(ReplaceError::NotFound(_))
            | CardError::Delete(DeleteError::NotFound(_)) => Self::NotFound("card"),
            CardError::Insert(InsertError::Duplicate(_)) => {
                Self::Conflict("a card with that id already exists".into())
            }
            CardError::HasChildren(_) | CardError::HoldsCards { .. } => {
                Self::Conflict(e.to_string())
            }
            CardError::Insert(InsertError::Durability(e))
            | CardError::Replace(ReplaceError::Durability(e))
            | CardError::Delete(DeleteError::Durability(e)) => Self::Storage(e.to_string()),
            other => Self::Invalid(other.to_string()),
        }
    }
}

impl From<SeedError> for ServiceError {
    fn from(e: SeedError) -> Self {
        match e {
            SeedError::Card(e) => e.into(),
            SeedError::Store(e) => e.into(),
            SeedError::Io { .. } | SeedError::Locked(_) => Self::Storage(e.to_string()),
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
    /// Covers the card and everything under it: it moves when a
    /// descendant is edited, added, removed or reordered, so it guards the
    /// subtree operations (`unsplit`, `reorder`) that the card's own tag
    /// cannot.
    pub tree_etag: String,
}

impl CardView {
    /// The card's etag: SHA-256 over its serialized form, shortened. Two
    /// reads of the same stored card agree; any field change changes it.
    pub fn etag(&self) -> String {
        card_etag(&self.card)
    }
}

/// See [`CardView::etag`].
pub fn card_etag(card: &Card) -> String {
    let text = rusty_json::to_string(card).unwrap_or_default();
    let digest = Sha256::digest(text.as_bytes());
    digest[..8].iter().map(|b| format!("{b:02x}")).collect()
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
    dir: PathBuf,
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
            dir: dir.to_path_buf(),
            _lock: lock,
            people: open_or_create_person_production_stack(&dir.join(PERSON_FILE))?,
            defaults: open_or_create_card_default_production_stack(&dir.join(CARD_DEFAULT_FILE))?,
            cards: open_or_create_card_production_stack(&dir.join(CARD_FILE))?,
        })
    }

    fn view(&self, card: Card) -> Result<CardView> {
        let baseline = baseline_of(&self.cards, &self.defaults, card.id)?;
        let state = card_state(&card, baseline.as_ref())?;
        let tree_etag = self.tree_etag(card.id);
        Ok(CardView {
            card,
            state,
            tree_etag,
        })
    }

    /// [`CardView::tree_etag`]: SHA-256 over the card tags of the card and
    /// its whole subtree in tree order.
    fn tree_etag(&self, id: Uuid) -> String {
        let Ok(tree) = card_tree(&self.cards, id) else {
            return String::new();
        };
        let mut hash = Sha256::new();
        for card in tree.flatten() {
            hash.update(card_etag(card).as_bytes());
        }
        hash.finalize()[..8]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
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

    /// Put `parent`'s children in `order` — one request, one lock, one
    /// slot write per child — and return them in that order.
    pub fn reorder_children(&mut self, parent: Uuid, order: &[Uuid]) -> Result<Vec<CardView>> {
        reorder_children(&mut self.cards, parent, order)?;
        order.iter().map(|id| self.card_view(*id)).collect()
    }

    /// Delete a leaf card; a split parent is refused (`Conflict`).
    pub fn delete_card(&mut self, id: Uuid) -> Result<()> {
        Ok(delete_card(&mut self.cards, id)?)
    }

    /// Merge a split back: every card under `id` goes, deepest first;
    /// `id` stays. Returns the parent and the deleted ids.
    pub fn unsplit(&mut self, id: Uuid) -> Result<(CardView, Vec<Uuid>)> {
        let deleted = unsplit_card(&mut self.cards, id)?;
        Ok((self.card_view(id)?, deleted))
    }

    /// Delete a person who holds no card; one who does is refused
    /// (`Conflict`, naming the count).
    pub fn delete_person(&mut self, id: Uuid) -> Result<()> {
        Ok(delete_person(&mut self.people, &self.cards, id)?)
    }

    /// Load the deck that ships in the library, restoring any deck card
    /// that is missing and never touching one that exists, then record
    /// that the load finished. An explicit call, so it also brings back a
    /// card that was deleted; start-up uses [`Service::ensure_deck`].
    pub fn seed_deck(&mut self) -> Result<SeedReport> {
        let data = SeedData {
            deck: seed::deck()?,
            ..SeedData::default()
        };
        let report = seed::seed_into(&mut self.people, &mut self.defaults, &mut self.cards, &data)?;
        self.mark_deck_loaded()?;
        Ok(report)
    }

    /// Load the deck once: a no-op when a load already finished (so a
    /// deleted deck card stays deleted), and a completing re-run when a
    /// first load was interrupted before it wrote its marker.
    pub fn ensure_deck(&mut self) -> Result<Option<SeedReport>> {
        if self.has_deck() {
            return Ok(None);
        }
        self.seed_deck().map(Some)
    }

    /// Whether a deck load has finished.
    pub fn has_deck(&self) -> bool {
        self.dir.join(DECK_MARKER).is_file()
    }

    /// Write the marker by rename, so it is either absent or complete.
    fn mark_deck_loaded(&self) -> Result<()> {
        let io = |e: std::io::Error| ServiceError::Storage(format!("deck marker: {e}"));
        let tmp = self.dir.join(format!("{DECK_MARKER}.tmp"));
        let file = std::fs::File::create(&tmp).map_err(io)?;
        file.sync_all().map_err(io)?;
        std::fs::rename(&tmp, self.dir.join(DECK_MARKER)).map_err(io)
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
    use rusty_fair_play_domain::deck_card_id;
    use rusty_fair_play_domain::seed::SeedData;

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
    fn a_deleted_deck_card_stays_deleted_across_restarts() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Service::open(dir.path()).unwrap();
        assert!(!s.has_deck());
        assert_eq!(s.ensure_deck().unwrap().unwrap().cards.created, 100);
        assert!(s.has_deck());
        s.delete_card(deck_card_id(1)).unwrap();
        assert!(s.ensure_deck().unwrap().is_none(), "already loaded");
        drop(s);

        let mut s = Service::open(dir.path()).unwrap();
        assert!(s.has_deck(), "the marker is on disk");
        assert!(s.ensure_deck().unwrap().is_none());
        assert_eq!(s.snapshot().unwrap().cards.len(), 99);
        assert!(
            s.card(deck_card_id(1)).is_err(),
            "card 1 was not resurrected"
        );

        // An explicit POST /seed does bring it back.
        assert_eq!(s.seed_deck().unwrap().cards.created, 1);
        assert_eq!(s.snapshot().unwrap().cards.len(), 100);
    }

    #[test]
    fn an_interrupted_first_load_is_finished_on_the_next_start() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Service::open(dir.path()).unwrap();
        // Ten cards in, no marker: what a kill part way through leaves.
        let partial = SeedData {
            deck: seed::deck().unwrap().into_iter().take(10).collect(),
            ..SeedData::default()
        };
        seed::seed_into(&mut s.people, &mut s.defaults, &mut s.cards, &partial).unwrap();
        assert!(s.card(deck_card_id(1)).is_ok());
        assert!(!s.has_deck(), "ten cards are not a loaded deck");
        drop(s);

        let mut s = Service::open(dir.path()).unwrap();
        let report = s.ensure_deck().unwrap().expect("the load resumes");
        assert_eq!((report.cards.created, report.cards.existing), (90, 10));
        assert_eq!(s.snapshot().unwrap().cards.len(), 100);
        assert!(s.has_deck());
    }

    #[test]
    fn the_tree_etag_moves_with_any_descendant() {
        let (_d, mut s) = service();
        let cleaning = deck_card_id(2);
        let made = s
            .split(
                cleaning,
                SplitRequest {
                    children: vec![
                        SplitChild {
                            name: "Bathrooms".into(),
                            ..SplitChild::default()
                        },
                        SplitChild {
                            name: "Floors".into(),
                            ..SplitChild::default()
                        },
                    ],
                    ..SplitRequest::default()
                },
            )
            .unwrap();
        let root = s.card(cleaning).unwrap();
        let (a, b) = (made.children[0].card.id, made.children[1].card.id);
        let patch = CardPatch {
            notes: Some("x".into()),
            ..CardPatch::default()
        };
        s.patch_card(a, patch).unwrap();
        let edited = s.card(cleaning).unwrap();
        assert_eq!(edited.etag(), root.etag(), "the card itself did not change");
        assert_ne!(edited.tree_etag, root.tree_etag, "but its subtree did");

        s.reorder_children(cleaning, &[b, a]).unwrap();
        let reordered = s.card(cleaning).unwrap();
        assert_eq!(reordered.etag(), root.etag());
        assert_ne!(reordered.tree_etag, edited.tree_etag, "a reorder moves it");
        assert_eq!(
            s.card(a).unwrap().tree_etag,
            s.card(a).unwrap().tree_etag,
            "a leaf's tags are stable between reads"
        );
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

        // Etags change with the card and only with the card.
        let before = s.card(cleaning).unwrap().etag();
        assert_eq!(before, s.card(cleaning).unwrap().etag());
        s.patch_card(
            cleaning,
            CardPatch {
                notes: Some("again".into()),
                ..CardPatch::default()
            },
        )
        .unwrap();
        assert_ne!(before, s.card(cleaning).unwrap().etag());

        // Reorder, delete, unsplit, delete person.
        let (floors, bathrooms) = (split.children[1].card.id, split.children[0].card.id);
        let ordered = s.reorder_children(cleaning, &[floors, bathrooms]).unwrap();
        assert_eq!(
            ordered.iter().map(|v| v.card.position).collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert!(matches!(
            s.reorder_children(cleaning, &[floors]),
            Err(ServiceError::Invalid(_))
        ));
        assert!(matches!(
            s.delete_card(cleaning),
            Err(ServiceError::Conflict(_))
        ));
        assert!(matches!(
            s.delete_person(bob.id),
            Err(ServiceError::Conflict(_))
        ));
        s.delete_card(floors).unwrap();
        let (parent, deleted) = s.unsplit(cleaning).unwrap();
        assert_eq!(deleted, vec![bathrooms]);
        assert_eq!(parent.card.id, cleaning);
        assert!(matches!(
            s.card(bathrooms),
            Err(ServiceError::NotFound("card"))
        ));
        s.delete_card(custom.card.id).unwrap();
        s.delete_person(bob.id).unwrap();
        assert!(matches!(
            s.person(bob.id),
            Err(ServiceError::NotFound("person"))
        ));
        assert_eq!(s.snapshot().unwrap().cards.len(), 100);
    }
}
