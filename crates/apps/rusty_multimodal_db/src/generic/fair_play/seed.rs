//! The Fair Play seed loader (`FPL-FR-006`, ADR-0137): people, the
//! supplied 100-card deck, and optional splits — parsed from CSV text —
//! into a data directory of the three stacks. Parsing is pure (text in,
//! rows or a line-numbered refusal out); only [`seed`] and the `*_file`
//! helpers touch the filesystem, so the CLI (`examples/fair_play_seed.rs`)
//! and a server can share it. Hand-rolled CSV (quoted fields, doubled
//! quotes, newlines inside quotes) rather than a new dependency: the
//! files are three, small and ours.
//!
//! The deck ships inside the library ([`DECK_CSV`](crate::generic::fair_play::seed::DECK_CSV), the supplied file
//! verbatim), so a binary can seed itself with no file on disk.
//!
//! Idempotent by id: every row's id is deterministic (`person_id`,
//! `card_default_id`, `deck_card_id`, `split_card_id`), so a rerun finds
//! what it wrote before and skips it. A live card that already exists is
//! never overwritten — a family's edits, owners and splits survive a
//! rerun — and is counted in the report as `existing`. A baseline that
//! already exists is likewise left alone, even if the text changed: the
//! baseline is what the family's cards were measured against when they
//! were dealt.

use super::{
    card_default_id, children_ordered, deck_card_id, insert_card,
    open_or_create_card_default_production_stack, open_or_create_card_production_stack,
    open_or_create_person_production_stack, person_id, split_card, split_card_id, CardDefault,
    CardError, CardProductionStack, NameField, Person, SplitSpec, Suit, CARD_DEFAULT_FILE,
    CARD_FILE, PERSON_FILE,
};
use crate::durability::DurabilityError;
use crate::generic::query::{FilterEq, GetById};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

/// The supplied deck, verbatim: `data/fair-play-cards.csv`.
pub const DECK_CSV: &str = include_str!("../../../data/fair-play-cards.csv");

/// What one load writes: people, the deck's baselines and live cards,
/// and splits, in that order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SeedData {
    pub people: Vec<Person>,
    /// One baseline plus the row's `notes` (which goes on the live card only).
    pub deck: Vec<(CardDefault, String)>,
    pub splits: Vec<SplitRow>,
    /// Names the splits in a refusal (`<label>:<line>`).
    pub splits_label: String,
}

/// How many rows each input produced — written, or found already there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Tally {
    pub created: usize,
    pub existing: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SeedReport {
    pub people: Tally,
    pub card_defaults: Tally,
    pub cards: Tally,
    pub splits: Tally,
}

/// Every refusal names the input and the 1-based line of the row (the
/// header is line 1), so a malformed row is fixed rather than skipped.
#[derive(Debug)]
pub enum SeedError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// `<label>:<line>: <what is wrong>`.
    Row {
        label: String,
        line: usize,
        message: String,
    },
    /// A whole-input check: the row count, the suit counts.
    File {
        label: String,
        message: String,
    },
    Store(DurabilityError),
    Card(CardError),
}

impl std::fmt::Display for SeedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SeedError::Io { path, source } => write!(f, "{}: {source}", path.display()),
            SeedError::Row {
                label,
                line,
                message,
            } => write!(f, "{label}:{line}: {message}"),
            SeedError::File { label, message } => write!(f, "{label}: {message}"),
            SeedError::Store(e) => write!(f, "store: {e}"),
            SeedError::Card(e) => write!(f, "card: {e}"),
        }
    }
}

impl std::error::Error for SeedError {}

impl From<DurabilityError> for SeedError {
    fn from(e: DurabilityError) -> Self {
        SeedError::Store(e)
    }
}

impl From<CardError> for SeedError {
    fn from(e: CardError) -> Self {
        SeedError::Card(e)
    }
}

// ---------------------------------------------------------------------
// CSV
// ---------------------------------------------------------------------

/// One parsed row: its fields and the line it started on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub line: usize,
    pub fields: Vec<String>,
}

/// RFC 4180: comma-separated, a field may be quoted, a quote inside a
/// quoted field is doubled, a quoted field may span lines. `\r\n` and
/// `\n` both end a record. An unterminated quote is an error at the line
/// it opened on.
pub fn parse_csv(text: &str) -> Result<Vec<Row>, (usize, String)> {
    let mut rows = Vec::new();
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut line = 1;
    let mut row_line = 1;
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    let mut pending = false; // something on the current row not yet pushed
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => quoted = false,
                '\n' => {
                    line += 1;
                    field.push(c);
                }
                _ => field.push(c),
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => {
                quoted = true;
                pending = true;
            }
            ',' => {
                fields.push(std::mem::take(&mut field));
                pending = true;
            }
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' => {
                if pending || !field.is_empty() {
                    fields.push(std::mem::take(&mut field));
                    rows.push(Row {
                        line: row_line,
                        fields: std::mem::take(&mut fields),
                    });
                }
                pending = false;
                line += 1;
                row_line = line;
            }
            _ => {
                field.push(c);
                pending = true;
            }
        }
    }
    if quoted {
        return Err((row_line, "unterminated quoted field".into()));
    }
    if pending || !field.is_empty() {
        fields.push(field);
        rows.push(Row {
            line: row_line,
            fields,
        });
    }
    Ok(rows)
}

fn rows(text: &str, label: &str, header: &[&str]) -> Result<Vec<Row>, SeedError> {
    let mut rows = parse_csv(text).map_err(|(line, message)| SeedError::Row {
        label: label.to_string(),
        line,
        message,
    })?;
    let Some(first) = rows.first() else {
        return Err(SeedError::File {
            label: label.to_string(),
            message: "empty input; expected a header row".into(),
        });
    };
    if first.fields != header {
        return Err(row_error(
            label,
            first.line,
            format!(
                "header must be `{}`, got `{}`",
                header.join(","),
                first.fields.join(",")
            ),
        ));
    }
    rows.remove(0);
    for row in &rows {
        if row.fields.len() != header.len() {
            return Err(row_error(
                label,
                row.line,
                format!("expected {} fields, got {}", header.len(), row.fields.len()),
            ));
        }
    }
    Ok(rows)
}

fn row_error(label: &str, line: usize, message: impl Into<String>) -> SeedError {
    SeedError::Row {
        label: label.to_string(),
        line,
        message: message.into(),
    }
}

fn read(path: &Path) -> Result<String, SeedError> {
    std::fs::read_to_string(path).map_err(|source| SeedError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn label(path: &Path) -> String {
    path.display().to_string()
}

// ---------------------------------------------------------------------
// People
// ---------------------------------------------------------------------

pub const PEOPLE_HEADER: [&str; 1] = ["name"];

/// `name` per row; `player` is the row's 1-based order.
pub fn parse_people_text(text: &str, label: &str) -> Result<Vec<Person>, SeedError> {
    let mut seen = HashSet::new();
    rows(text, label, &PEOPLE_HEADER)?
        .into_iter()
        .enumerate()
        .map(|(i, row)| {
            let name = row.fields[0].trim();
            if name.is_empty() {
                return Err(row_error(label, row.line, "empty name"));
            }
            if !seen.insert(name.to_string()) {
                return Err(row_error(
                    label,
                    row.line,
                    format!("duplicate name `{name}`"),
                ));
            }
            Ok(Person {
                id: person_id(name),
                name: name.to_string(),
                player: u32::try_from(i + 1).unwrap_or(u32::MAX),
            })
        })
        .collect()
}

pub fn parse_people(path: &Path) -> Result<Vec<Person>, SeedError> {
    parse_people_text(&read(path)?, &label(path))
}

// ---------------------------------------------------------------------
// Cards
// ---------------------------------------------------------------------

pub const CARDS_HEADER: [&str; 8] = [
    "number",
    "name",
    "suit",
    "conception",
    "planning",
    "execution",
    "minimum_standard_of_care",
    "notes",
];

/// What the supplied deck must be: 100 rows, these suit counts.
pub const DECK_SIZE: usize = 100;
pub const SUIT_COUNTS: [(Suit, usize); 6] = [
    (Suit::Home, 22),
    (Suit::Out, 22),
    (Suit::Caregiving, 22),
    (Suit::Magic, 22),
    (Suit::Wild, 10),
    (Suit::UnicornSpace, 2),
];

/// `|`-separated standards; an empty cell is no standards.
pub fn parse_standards(cell: &str) -> Vec<String> {
    cell.split('|')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

/// One deck row as a baseline plus the row's `notes`. The whole deck is
/// checked: 100 rows, the suit counts, unique numbers in 1..=100.
pub fn parse_cards_text(text: &str, label: &str) -> Result<Vec<(CardDefault, String)>, SeedError> {
    let rows = rows(text, label, &CARDS_HEADER)?;
    let mut numbers = BTreeMap::new();
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let f = &row.fields;
        let number: u16 = f[0]
            .trim()
            .parse()
            .ok()
            .filter(|n| (1..=100).contains(n))
            .ok_or_else(|| {
                row_error(
                    label,
                    row.line,
                    format!("number must be 1..=100, got `{}`", f[0]),
                )
            })?;
        if let Some(first) = numbers.insert(number, row.line) {
            return Err(row_error(
                label,
                row.line,
                format!("number {number} already used on line {first}"),
            ));
        }
        let suit = Suit::parse(f[2].trim())
            .ok_or_else(|| row_error(label, row.line, format!("unknown suit `{}`", f[2])))?;
        if f[1].trim().is_empty() {
            return Err(row_error(label, row.line, "empty name"));
        }
        out.push((
            CardDefault {
                id: card_default_id(number),
                number,
                name: f[1].trim().to_string(),
                suit,
                conception: f[3].trim().to_string(),
                planning: f[4].trim().to_string(),
                execution: f[5].trim().to_string(),
                minimum_standard_of_care: parse_standards(&f[6]),
            },
            f[7].trim().to_string(),
        ));
    }
    if out.len() != DECK_SIZE {
        return Err(SeedError::File {
            label: label.to_string(),
            message: format!("expected {DECK_SIZE} cards, got {}", out.len()),
        });
    }
    for (suit, expected) in SUIT_COUNTS {
        let got = out.iter().filter(|(c, _)| c.suit == suit).count();
        if got != expected {
            return Err(SeedError::File {
                label: label.to_string(),
                message: format!("expected {expected} {} cards, got {got}", suit.as_str()),
            });
        }
    }
    Ok(out)
}

pub fn parse_cards(path: &Path) -> Result<Vec<(CardDefault, String)>, SeedError> {
    parse_cards_text(&read(path)?, &label(path))
}

/// The deck that ships with the library.
pub fn deck() -> Result<Vec<(CardDefault, String)>, SeedError> {
    parse_cards_text(DECK_CSV, "fair-play-cards.csv")
}

// ---------------------------------------------------------------------
// Splits
// ---------------------------------------------------------------------

pub const SPLITS_HEADER: [&str; 4] = [
    "parent_path",
    "name",
    "owner_name",
    "minimum_standard_of_care",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitRow {
    pub line: usize,
    /// `17` or `17/Bathrooms`: a deck number, then child names.
    pub parent_path: String,
    pub name: String,
    pub owner_name: Option<String>,
    pub minimum_standard_of_care: Vec<String>,
}

pub fn parse_splits_text(text: &str, label: &str) -> Result<Vec<SplitRow>, SeedError> {
    rows(text, label, &SPLITS_HEADER)?
        .into_iter()
        .map(|row| {
            let f = &row.fields;
            let parent_path = f[0].trim().to_string();
            let Some(first) = parent_path.split('/').next() else {
                return Err(row_error(label, row.line, "empty parent_path"));
            };
            if first.parse::<u16>().is_err() {
                return Err(row_error(
                    label,
                    row.line,
                    format!("parent_path must start with a deck number, got `{parent_path}`"),
                ));
            }
            if parent_path.split('/').any(|seg| seg.trim().is_empty()) {
                return Err(row_error(
                    label,
                    row.line,
                    format!("parent_path has an empty segment: `{parent_path}`"),
                ));
            }
            let name = f[1].trim().to_string();
            if name.is_empty() || name.contains('/') {
                return Err(row_error(
                    label,
                    row.line,
                    "name must be non-empty and contain no `/`",
                ));
            }
            let owner = f[2].trim();
            Ok(SplitRow {
                line: row.line,
                parent_path,
                name,
                owner_name: (!owner.is_empty()).then(|| owner.to_string()),
                minimum_standard_of_care: parse_standards(&f[3]),
            })
        })
        .collect()
}

pub fn parse_splits(path: &Path) -> Result<Vec<SplitRow>, SeedError> {
    parse_splits_text(&read(path)?, &label(path))
}

/// `17/Bathrooms/Tub` → the card: the deck card, then one child by name
/// per segment. Ambiguous or missing names are errors on the row.
fn resolve_parent(
    cards: &CardProductionStack,
    label: &str,
    row: &SplitRow,
) -> Result<uuid::Uuid, SeedError> {
    let mut segments = row.parent_path.split('/');
    let number: u16 = segments
        .next()
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| row_error(label, row.line, "bad deck number"))?;
    let mut current = deck_card_id(number);
    if cards.get(current).is_none() {
        return Err(row_error(
            label,
            row.line,
            format!("deck card {number} is not loaded"),
        ));
    }
    for name in segments {
        let matches: Vec<_> = children_ordered(cards, current)
            .into_iter()
            .filter(|c| c.name == name)
            .collect();
        current = match matches.as_slice() {
            [one] => one.id,
            [] => {
                return Err(row_error(
                    label,
                    row.line,
                    format!("`{}` has no child named `{name}`", row.parent_path),
                ))
            }
            _ => {
                return Err(row_error(
                    label,
                    row.line,
                    format!(
                        "`{}` has more than one child named `{name}`",
                        row.parent_path
                    ),
                ))
            }
        };
    }
    Ok(current)
}

// ---------------------------------------------------------------------
// The load
// ---------------------------------------------------------------------

/// Load `data` into the stacks under `store_dir`, creating them if
/// absent. Writes go people, then baselines, then live cards, then
/// splits in order; nothing is written for an input that failed to
/// parse, since parsing happens before this is called.
pub fn seed(store_dir: &Path, data: &SeedData) -> Result<SeedReport, SeedError> {
    std::fs::create_dir_all(store_dir).map_err(|source| SeedError::Io {
        path: store_dir.to_path_buf(),
        source,
    })?;
    let mut people = open_or_create_person_production_stack(&store_dir.join(PERSON_FILE))?;
    let mut defaults =
        open_or_create_card_default_production_stack(&store_dir.join(CARD_DEFAULT_FILE))?;
    let mut cards = open_or_create_card_production_stack(&store_dir.join(CARD_FILE))?;
    seed_into(&mut people, &mut defaults, &mut cards, data)
}

/// [`seed`] on stacks the caller already holds open — what a running
/// server uses, since it owns the stacks for its lifetime.
pub fn seed_into(
    people: &mut super::PersonProductionStack,
    defaults: &mut super::CardDefaultProductionStack,
    cards: &mut CardProductionStack,
    data: &SeedData,
) -> Result<SeedReport, SeedError> {
    let mut report = SeedReport::default();

    for person in &data.people {
        if people.get(person.id).is_some() {
            report.people.existing += 1;
            continue;
        }
        people
            .insert(person.clone())
            .map_err(|e| SeedError::Card(CardError::Insert(e)))?;
        report.people.created += 1;
    }

    for (baseline, notes) in &data.deck {
        if defaults.get(baseline.id).is_some() {
            report.card_defaults.existing += 1;
        } else {
            defaults
                .insert(baseline.clone())
                .map_err(|e| SeedError::Card(CardError::Insert(e)))?;
            report.card_defaults.created += 1;
        }
        let mut card = baseline.to_card();
        card.notes = notes.clone();
        if cards.get(card.id).is_some() {
            report.cards.existing += 1;
            continue;
        }
        insert_card(cards, card)?;
        report.cards.created += 1;
    }

    for row in &data.splits {
        let id = split_card_id(&format!("{}/{}", row.parent_path, row.name));
        if cards.get(id).is_some() {
            report.splits.existing += 1;
            continue;
        }
        let parent = resolve_parent(cards, &data.splits_label, row)?;
        let owner_id = match &row.owner_name {
            None => None,
            Some(name) => match FilterEq::<Person, NameField>::filter_eq(people, name).as_slice() {
                [one] => Some(*one),
                _ => {
                    return Err(row_error(
                        &data.splits_label,
                        row.line,
                        format!("unknown owner `{name}`"),
                    ))
                }
            },
        };
        split_card(
            cards,
            parent,
            vec![SplitSpec {
                id,
                name: row.name.clone(),
                owner_id,
                minimum_standard_of_care: row.minimum_standard_of_care.clone(),
                ..SplitSpec::default()
            }],
            None,
        )?;
        report.splits.created += 1;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_deck_parses_and_matches_the_counts() {
        let deck = deck().unwrap();
        assert_eq!(deck.len(), DECK_SIZE);
        assert_eq!(deck[16].0.name, "Meals (Weekday Dinner)");
        assert_eq!(deck[16].0.number, 17);
    }

    #[test]
    fn csv_parser_handles_quotes_doubled_quotes_and_crlf() {
        let rows = parse_csv("a,b\r\n1,\"x, \"\"y\"\"\"\n\"multi\nline\",\n").unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1].fields, vec!["1", "x, \"y\""]);
        assert_eq!(rows[2].fields, vec!["multi\nline", ""]);
        assert_eq!(rows[2].line, 3);
        assert_eq!(parse_csv("a,\"open\n").unwrap_err().0, 1);
    }
}
