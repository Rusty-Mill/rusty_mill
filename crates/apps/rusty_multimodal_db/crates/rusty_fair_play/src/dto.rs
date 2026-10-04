//! The JSON shapes on the wire: camelCase, ids as UUID strings, the suit
//! by its deck spelling (`Home`, `Out`, `Caregiving`, `Magic`, `Wild`,
//! `Unicorn Space`), `origin` as `deck`/`family`, `state` as
//! `original`/`edited`/`custom`. Inputs reject unknown fields, so a typo
//! in a field name is an error rather than a silently ignored change.

use crate::service::{CardView, Snapshot};
use rusty_multimodal_db::generic::fair_play::{
    Card, CardDefault, CardState, FieldDiff, Origin, Person, Suit,
};
use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

// ---- responses ---------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonDto {
    pub id: Uuid,
    pub name: String,
    pub player: u32,
}

impl From<Person> for PersonDto {
    fn from(p: Person) -> Self {
        Self {
            id: p.id,
            name: p.name,
            player: p.player,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardDto {
    pub id: Uuid,
    pub number: Option<u16>,
    pub name: String,
    pub suit: &'static str,
    pub parent_card_id: Option<Uuid>,
    pub position: u32,
    pub owner_id: Option<Uuid>,
    pub conception: String,
    pub planning: String,
    pub execution: String,
    pub minimum_standard_of_care: Vec<String>,
    pub notes: String,
    pub origin: &'static str,
    pub baseline_id: Option<Uuid>,
    pub state: &'static str,
}

pub fn state_name(state: CardState) -> &'static str {
    match state {
        CardState::Original => "original",
        CardState::Edited => "edited",
        CardState::Custom => "custom",
    }
}

impl From<CardView> for CardDto {
    fn from(v: CardView) -> Self {
        let c: Card = v.card;
        Self {
            id: c.id,
            number: c.number,
            name: c.name,
            suit: c.suit.as_str(),
            parent_card_id: c.parent_card_id,
            position: c.position,
            owner_id: c.owner_id,
            conception: c.conception,
            planning: c.planning,
            execution: c.execution,
            minimum_standard_of_care: c.minimum_standard_of_care,
            notes: c.notes,
            origin: match c.origin {
                Origin::Deck => "deck",
                Origin::Family => "family",
            },
            baseline_id: c.baseline_id,
            state: state_name(v.state),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BaselineDto {
    pub id: Uuid,
    pub number: u16,
    pub name: String,
    pub suit: &'static str,
    pub conception: String,
    pub planning: String,
    pub execution: String,
    pub minimum_standard_of_care: Vec<String>,
}

impl From<CardDefault> for BaselineDto {
    fn from(d: CardDefault) -> Self {
        Self {
            id: d.id,
            number: d.number,
            name: d.name,
            suit: d.suit.as_str(),
            conception: d.conception,
            planning: d.planning,
            execution: d.execution,
            minimum_standard_of_care: d.minimum_standard_of_care,
        }
    }
}

#[derive(Serialize)]
pub struct DiffDto {
    pub field: &'static str,
    pub card: String,
    pub baseline: String,
}

impl From<FieldDiff> for DiffDto {
    fn from(d: FieldDiff) -> Self {
        Self {
            field: d.field.name(),
            card: d.card,
            baseline: d.baseline,
        }
    }
}

#[derive(Serialize)]
pub struct BaselineResponse {
    pub baseline: Option<BaselineDto>,
    pub diff: Vec<DiffDto>,
}

#[derive(Serialize)]
pub struct SnapshotDto {
    pub people: Vec<PersonDto>,
    pub cards: Vec<CardDto>,
}

impl From<Snapshot> for SnapshotDto {
    fn from(s: Snapshot) -> Self {
        Self {
            people: s.people.into_iter().map(PersonDto::from).collect(),
            cards: s.cards.into_iter().map(CardDto::from).collect(),
        }
    }
}

#[derive(Serialize)]
pub struct SplitDto {
    pub parent: CardDto,
    pub children: Vec<CardDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TallyDto {
    pub created: usize,
    pub existing: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeedDto {
    pub card_defaults: TallyDto,
    pub cards: TallyDto,
}

// ---- requests ----------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreatePerson {
    pub name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchPerson {
    pub name: String,
}

/// A nullable field in a patch: absent keeps, `null` clears, a value sets.
/// `Option<Option<T>>` with serde's default does not tell absent from
/// `null`, so `deserialize_with` wraps the parsed value one level.
fn nullable<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct PatchCard {
    pub name: Option<String>,
    pub suit: Option<String>,
    pub conception: Option<String>,
    pub planning: Option<String>,
    pub execution: Option<String>,
    pub minimum_standard_of_care: Option<Vec<String>>,
    pub notes: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub owner_id: Option<Option<Uuid>>,
    #[serde(deserialize_with = "nullable")]
    pub parent_card_id: Option<Option<Uuid>>,
    pub position: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateCard {
    #[serde(default)]
    pub id: Option<Uuid>,
    pub name: String,
    pub suit: String,
    #[serde(default)]
    pub parent_card_id: Option<Uuid>,
    #[serde(default)]
    pub owner_id: Option<Uuid>,
    #[serde(default)]
    pub conception: String,
    #[serde(default)]
    pub planning: String,
    #[serde(default)]
    pub execution: String,
    #[serde(default)]
    pub minimum_standard_of_care: Vec<String>,
    #[serde(default)]
    pub notes: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SplitChildInput {
    #[serde(default)]
    pub id: Option<Uuid>,
    pub name: String,
    #[serde(default)]
    pub owner_id: Option<Uuid>,
    #[serde(default)]
    pub conception: String,
    #[serde(default)]
    pub planning: String,
    #[serde(default)]
    pub execution: String,
    #[serde(default)]
    pub minimum_standard_of_care: Vec<String>,
    #[serde(default)]
    pub notes: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct SplitInput {
    pub children: Vec<SplitChildInput>,
    #[serde(deserialize_with = "nullable")]
    pub owner_id: Option<Option<Uuid>>,
    pub notes: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetPosition {
    pub position: u32,
}

/// The deck spelling, or `None`.
pub fn parse_suit(text: &str) -> Option<Suit> {
    Suit::parse(text)
}
