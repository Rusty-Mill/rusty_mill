//! The one-off lift of headed body sections into the typed fields (#1582, from the #1568
//! survey): a `## Acceptance` section becomes the card's `acceptance`, and so on. Planning is
//! pure ([`plan`]); [`apply`] and [`undo`] write through the [`Store`] port.
//!
//! Only unambiguous cases move. A field is lifted when exactly one section names it, the card's
//! field is still empty and the text fits the cap; anything else stays in the body and is
//! listed in [`Plan::left`]. Two more sections are tidied: `## Parent milestone` is dropped
//! where it only repeats the card's `parent`, and `## Prerequisites` codes (`M2.2, M1.4`)
//! become `related` links where every code names exactly one card of the project. `related`,
//! not `deps`: a dependency stops a card being taken, and that is a decision for a person.
//! `## Execution gate` is left for a project template.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::backfill::Outcome;
use crate::domain::{
    BoardError, Card, Edit, Event, EventKind, MAX_ACCEPTANCE_BYTES, MAX_SCOPE_BYTES,
    MAX_SUMMARY_BYTES, MAX_WHY_BYTES,
};
use crate::store::Store;

/// The actor recorded on lift events.
pub const ACTOR: &str = "lift";

/// The typed fields, with the section titles (lowercase) that name each and its byte cap.
const FIELDS: [(&str, &[&str], usize); 4] = [
    (
        "acceptance",
        &[
            "acceptance",
            "acceptance criteria",
            "done when",
            "approved-design acceptance and boundaries",
            "completion gate",
        ],
        MAX_ACCEPTANCE_BYTES,
    ),
    (
        "scope",
        &["scope", "bounded scope", "out of scope"],
        MAX_SCOPE_BYTES,
    ),
    ("why", &["why", "problem"], MAX_WHY_BYTES),
    ("summary", &["summary"], MAX_SUMMARY_BYTES),
];

/// The parts of a card a lift changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    /// The description.
    pub body: String,
    /// `acceptance`, `scope`, `why` and `summary`, in that order.
    pub fields: [String; 4],
    /// Cards worth reading alongside this one.
    pub related: Vec<Uuid>,
}

impl State {
    fn of(card: &Card) -> Self {
        Self {
            body: card.body.clone(),
            fields: [
                card.acceptance.clone(),
                card.scope.clone(),
                card.why.clone(),
                card.summary.clone(),
            ],
            related: card.related.clone(),
        }
    }
}

/// One card to change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lift {
    /// The card.
    pub id: Uuid,
    /// Its board number, for people reading the plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<u64>,
    /// The card's revision when planned.
    pub rev: u64,
    /// What it is now: [`undo`] puts this back.
    pub before: State,
    /// What it becomes.
    pub after: State,
}

/// A section left in a body, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Left {
    /// The card's board number.
    pub number: Option<u64>,
    /// The field or tidy-up the section was for.
    pub field: &'static str,
    /// Why it stays.
    pub why: String,
}

/// What a dry run found.
#[derive(Debug, Default, Serialize)]
pub struct Plan {
    /// Cards to change.
    pub lifts: Vec<Lift>,
    /// Sections that stay in their bodies.
    pub left: Vec<Left>,
}

/// A `## ` section: its title and the byte range of header plus text.
struct Section<'a> {
    title: &'a str,
    text: &'a str,
    start: usize,
    end: usize,
}

/// The `## ` sections of `body`, outside code fences.
fn sections(body: &str) -> Vec<Section<'_>> {
    let (mut out, mut fenced, mut at) = (Vec::<Section>::new(), false, 0);
    for line in body.split_inclusive('\n') {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        } else if !fenced && line.starts_with("## ") {
            if let Some(last) = out.last_mut() {
                last.end = at;
            }
            out.push(Section {
                title: line[3..].trim().trim_end_matches(':'),
                text: "",
                start: at,
                end: body.len(),
            });
        }
        at += line.len();
    }
    for s in &mut out {
        let whole = &body[s.start..s.end];
        s.text = whole.split_once('\n').map_or("", |(_, rest)| rest).trim();
    }
    out
}

/// `body` with each byte range of `cut` replaced by its text, blank-line runs squeezed.
fn rewrite(body: &str, mut cut: Vec<(usize, usize, String)>) -> String {
    cut.sort_unstable();
    let mut out = String::new();
    let mut at = 0;
    for (s, e, with) in cut {
        out.push_str(&body[at..s]);
        out.push_str(&with);
        at = e;
    }
    out.push_str(&body[at..]);
    while out.contains("\n\n\n") {
        out = out.replace("\n\n\n", "\n\n");
    }
    out.trim().to_owned()
}

/// A milestone-style code: `M2.2`.
fn is_code(s: &str) -> bool {
    let mut parts = s.split('.');
    let head = parts.next().unwrap_or("");
    let (letters, digits) = head.split_at(head.find(|c: char| c.is_ascii_digit()).unwrap_or(0));
    !letters.is_empty()
        && letters.bytes().all(|b| b.is_ascii_uppercase())
        && !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && parts.all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// Plan the lifts for `cards`.
#[must_use]
pub fn plan(cards: &[Card]) -> Plan {
    let by_id: BTreeMap<Uuid, &Card> = cards.iter().map(|c| (c.id, c)).collect();
    let mut by_code: BTreeMap<(&str, &str), Vec<Uuid>> = BTreeMap::new();
    for c in cards {
        if let Some(code) = c.title.split_whitespace().next().filter(|t| is_code(t)) {
            by_code.entry((&c.project, code)).or_default().push(c.id);
        }
    }
    let mut out = Plan::default();
    for card in cards {
        let found = lift_card(card, &by_id, &by_code, &mut out.left);
        out.lifts.extend(found);
    }
    out
}

fn lift_card(
    card: &Card,
    by_id: &BTreeMap<Uuid, &Card>,
    by_code: &BTreeMap<(&str, &str), Vec<Uuid>>,
    left: &mut Vec<Left>,
) -> Option<Lift> {
    let secs = sections(&card.body);
    let before = State::of(card);
    let mut after = before.clone();
    let mut cut = Vec::new();
    let mut keep = |field: &'static str, why: &str| {
        left.push(Left {
            number: card.number,
            field,
            why: why.to_owned(),
        });
    };
    for (i, (field, titles, cap)) in FIELDS.iter().enumerate() {
        let named: Vec<_> = secs
            .iter()
            .filter(|s| titles.contains(&s.title.to_lowercase().as_str()))
            .collect();
        let [one] = named[..] else {
            if named.len() > 1 {
                keep(field, "several sections name it");
            }
            continue;
        };
        if !before.fields[i].is_empty() {
            keep(field, "the field already has text");
        } else if one.text.is_empty() {
            keep(field, "the section is empty");
        } else if one.text.len() > *cap {
            keep(field, "the section is over the cap");
        } else {
            after.fields[i] = one.text.to_owned();
            cut.push((one.start, one.end, String::new()));
        }
    }
    for s in secs
        .iter()
        .filter(|s| s.title.eq_ignore_ascii_case("parent milestone"))
    {
        let number = card
            .parent
            .and_then(|p| by_id.get(&p))
            .and_then(|p| p.number);
        if number.is_some_and(|n| s.text == format!("#{n}")) {
            cut.push((s.start, s.end, String::new()));
        }
    }
    for s in secs
        .iter()
        .filter(|s| s.title.eq_ignore_ascii_case("prerequisites"))
    {
        let first = s.text.lines().next().unwrap_or("");
        let codes: Vec<&str> = first.split(',').map(str::trim).collect();
        if !codes.iter().all(|c| is_code(c)) {
            continue;
        }
        let found: Vec<Option<Uuid>> = codes
            .iter()
            .map(
                |c| match by_code.get(&(card.project.as_str(), *c)).map(Vec::as_slice) {
                    Some([id]) if *id != card.id => Some(*id),
                    _ => None,
                },
            )
            .collect();
        if found.iter().any(Option::is_none) {
            keep("prerequisites", "a code names no card or several");
            continue;
        }
        after.related.extend(found.into_iter().flatten());
        after.related.sort_unstable();
        after.related.dedup();
        // The codes line goes; any text after it stays under its header.
        let rest = s.text.split_once('\n').map_or("", |(_, r)| r.trim());
        let kept = if rest.is_empty() {
            String::new()
        } else {
            format!("## Prerequisites\n{rest}\n\n")
        };
        cut.push((s.start, s.end, kept));
    }
    if cut.is_empty() {
        return None;
    }
    after.body = rewrite(&card.body, cut);
    Some(Lift {
        id: card.id,
        number: card.number,
        rev: card.rev,
        before,
        after,
    })
}

/// A card carrying `state`, one revision on.
fn with_state(card: &Card, state: &State, now_ms: i64) -> Result<Card, BoardError> {
    let next = Card {
        body: state.body.clone(),
        related: state.related.clone(),
        ..card.next(now_ms)
    };
    let [a, s, w, m] = &state.fields;
    next.with_fields(&Edit {
        acceptance: Some(a),
        scope: Some(s),
        why: Some(w),
        summary: Some(m),
        ..Edit::default()
    })
}

/// Save `plan` as the record file at `path`, which must not exist yet, and sync it: what
/// [`undo`] reads, written before [`apply`] changes any card.
///
/// # Errors
/// If the file exists or cannot be written.
pub fn save_record(path: &std::path::Path, lifts: &[Lift]) -> Result<(), String> {
    let json = serde_json::to_string_pretty(lifts).map_err(|e| e.to_string())?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    std::io::Write::write_all(&mut file, json.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|e| e.to_string())
}

/// Write each lift whose card is still at the planned revision. A card changed since is
/// skipped and reported, so running a plan again is safe.
///
/// # Errors
/// The first storage error other than a conflict. Cards before it are written.
pub fn apply(store: &dyn Store, lifts: &[Lift], now_ms: i64) -> Result<Vec<Outcome>, BoardError> {
    write(store, lifts, now_ms, |card, l| {
        (card.rev == l.rev).then_some(&l.after)
    })
}

/// Put back each lifted card that still holds what the lift wrote.
///
/// # Errors
/// The first storage error other than a conflict. Cards before it are restored.
pub fn undo(store: &dyn Store, lifts: &[Lift], now_ms: i64) -> Result<Vec<Outcome>, BoardError> {
    write(store, lifts, now_ms, |card, l| {
        (State::of(card) == l.after).then_some(&l.before)
    })
}

fn write<'a>(
    store: &dyn Store,
    lifts: &'a [Lift],
    now_ms: i64,
    target: impl Fn(&Card, &'a Lift) -> Option<&'a State>,
) -> Result<Vec<Outcome>, BoardError> {
    let skip = |id, why: &str| Outcome::Skipped {
        id,
        why: why.to_owned(),
    };
    let mut out = Vec::new();
    for l in lifts {
        let Some(card) = store.get(l.id)? else {
            out.push(skip(l.id, "no longer on the board"));
            continue;
        };
        let Some(state) = target(&card, l) else {
            out.push(skip(l.id, "changed since the plan"));
            continue;
        };
        let next = with_state(&card, state, now_ms)?;
        match store.replace(&next, Event::of(&next, EventKind::Edited, ACTOR)) {
            Ok(_) => out.push(Outcome::Done { id: l.id }),
            Err(BoardError::Conflict) => out.push(skip(l.id, "changed while writing")),
            Err(e) => return Err(e),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemStore;

    fn card(title: &str, body: &str) -> Card {
        Card::new(title, body, Vec::new(), 1).expect("card")
    }

    fn lifted(c: &Card) -> Plan {
        plan(std::slice::from_ref(c))
    }

    #[test]
    fn headed_sections_move_into_their_fields_and_leave_the_rest_of_the_body() {
        let c = card(
            "t",
            "## Objective\nDo it.\n\n## Acceptance criteria\nIt works.\n\n## Why\nBecause.\n\n## Execution gate\nboiler\n",
        );
        let p = lifted(&c);
        let [l] = &p.lifts[..] else { panic!("{p:?}") };
        assert_eq!(l.after.fields[0], "It works.");
        assert_eq!(l.after.fields[2], "Because.");
        assert_eq!(
            l.after.body,
            "## Objective\nDo it.\n\n## Execution gate\nboiler"
        );
        assert!(p.left.is_empty());
    }

    #[test]
    fn ambiguous_full_over_cap_and_fenced_sections_stay_put() {
        let long = "x".repeat(MAX_SUMMARY_BYTES + 1);
        let mut set = card("t", "## Why\nnew\n");
        set.why = "kept".into();
        let cases = [
            (card("t", "## Scope\na\n\n## Out of scope\nb\n"), "several"),
            (set, "already"),
            (card("t", &format!("## Summary\n{long}\n")), "cap"),
            (card("t", "## Why\n"), "empty"),
        ];
        for (c, word) in cases {
            let p = lifted(&c);
            assert!(p.lifts.is_empty(), "{word}: {p:?}");
            assert!(p.left[0].why.contains(word), "{word}: {p:?}");
        }
        let fenced = card("t", "```\n## Why\nin code\n```\n");
        assert!(lifted(&fenced).lifts.is_empty());
    }

    #[test]
    fn parent_milestone_goes_only_where_it_repeats_the_parent() {
        let mut parent = card("M1", "");
        parent.number = Some(872);
        let mut child = card("c", "## Parent milestone\n#872\n\n## Objective\nx\n");
        child.parent = Some(parent.id);
        let mut odd = card("o", "## Parent milestone\n#1\n");
        odd.parent = Some(parent.id);
        let p = plan(&[parent, child, odd]);
        let [l] = &p.lifts[..] else { panic!("{p:?}") };
        assert_eq!(l.after.body, "## Objective\nx");
    }

    #[test]
    fn prerequisites_become_related_only_when_every_code_names_one_card() {
        let (a, b) = (card("M2.2 First", ""), card("M1.4 Second", ""));
        let text = "## Prerequisites\nM2.2, M1.4\n\nPlanning does not change status.\n";
        let c = card("M3.1 Third", text);
        let nope = card("M3.2 Fourth", "## Prerequisites\nM9.9\n");
        let p = plan(&[a.clone(), b.clone(), c, nope]);
        let [l] = &p.lifts[..] else { panic!("{p:?}") };
        let mut want = vec![a.id, b.id];
        want.sort_unstable();
        assert_eq!(l.after.related, want);
        assert_eq!(
            l.after.body,
            "## Prerequisites\nPlanning does not change status."
        );
        assert_eq!(p.left.len(), 1);
    }

    #[test]
    fn apply_skips_changed_cards_and_undo_restores_only_what_it_wrote() {
        let store = MemStore::default();
        let c = card("t", "## Why\nBecause.\n\n## Objective\nx\n");
        store
            .insert(&c, Event::of(&c, EventKind::Created, "t"))
            .expect("insert");
        let p = lifted(&c);
        assert!(matches!(
            apply(&store, &p.lifts, 5).expect("a")[0],
            Outcome::Done { .. }
        ));
        let now = store.get(c.id).expect("get").expect("card");
        assert_eq!(
            (now.why.as_str(), now.body.as_str()),
            ("Because.", "## Objective\nx")
        );
        assert!(matches!(
            apply(&store, &p.lifts, 6).expect("again")[0],
            Outcome::Skipped { .. }
        ));
        let edited = Card {
            why: "Edited.".into(),
            ..now.next(7)
        };
        store
            .replace(&edited, Event::of(&edited, EventKind::Edited, "t"))
            .expect("edit");
        assert!(matches!(
            undo(&store, &p.lifts, 8).expect("u")[0],
            Outcome::Skipped { .. }
        ));
        let back = Card {
            why: "Because.".into(),
            ..edited.next(9)
        };
        store
            .replace(&back, Event::of(&back, EventKind::Edited, "t"))
            .expect("restore");
        assert!(matches!(
            undo(&store, &p.lifts, 10).expect("u")[0],
            Outcome::Done { .. }
        ));
        let end = store.get(c.id).expect("get").expect("card");
        assert_eq!((end.body.as_str(), end.why.as_str()), (c.body.as_str(), ""));
    }
}
