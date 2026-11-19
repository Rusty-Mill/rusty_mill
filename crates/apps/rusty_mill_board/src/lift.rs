//! The one-off lift of headed body sections into the typed fields (#1582, from the #1568
//! survey): a `## Acceptance` section becomes the card's `acceptance`, and so on. Planning is
//! pure ([`plan`]); [`apply`] and [`undo`] write through the [`Store`] port.
//!
//! Only unambiguous cases move. A field is lifted when exactly one section names it, the card's
//! field is still empty and the text fits the cap (a wrapped summary becomes one line, and an
//! `## Out of scope` joins `scope` as `Out of scope: ...`, so its meaning survives); anything else stays in the body and is
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

/// A code fence line: its character, its run length, and whether nothing follows the run (only
/// such a line can close a fence, and only when its run is as long as the opening one).
fn fence_line(line: &str) -> Option<(char, usize, bool)> {
    let t = line.trim_start();
    let c = t.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let n = t.chars().take_while(|x| *x == c).count();
    (n >= 3).then(|| (c, n, t[n..].trim().is_empty()))
}

/// A `# ` or `## ` heading line, indented by up to three spaces: its level and title.
fn heading(line: &str) -> Option<(usize, &str)> {
    let text = line.trim_start_matches(' ');
    if line.len() - text.len() > 3 {
        return None;
    }
    text.strip_prefix("## ")
        .map(|t| (2, t))
        .or_else(|| text.strip_prefix("# ").map(|t| (1, t)))
}

/// The `## ` sections of `body`, outside code fences. A `# ` heading ends the section above it.
fn sections(body: &str) -> Vec<Section<'_>> {
    let (mut out, mut fence, mut open, mut at) = (Vec::<Section>::new(), None, false, 0);
    for line in body.split_inclusive('\n') {
        if let Some((c, n, bare)) = fence_line(line) {
            match fence {
                None => fence = Some((c, n)),
                Some((open_c, open_n)) if c == open_c && n >= open_n && bare => fence = None,
                Some(_) => {}
            }
        } else if let (None, Some((level, title))) = (fence, heading(line)) {
            if let Some(last) = out.last_mut().filter(|_| open) {
                last.end = at;
            }
            open = level == 2;
            if open {
                out.push(Section {
                    title: title.trim().trim_end_matches(':'),
                    text: "",
                    start: at,
                    end: body.len(),
                });
            }
        }
        at += line.len();
    }
    for s in &mut out {
        let whole = &body[s.start..s.end];
        s.text = whole.split_once('\n').map_or("", |(_, rest)| rest).trim();
    }
    out
}

/// `body` without the byte ranges of `cut`. A range marked `tidy` is a whole section: the blank
/// lines at its join are squeezed to one. The rest of the body is left byte for byte.
fn rewrite(body: &str, mut cut: Vec<(usize, usize, bool)>) -> String {
    cut.sort_unstable();
    let (mut out, mut at) = (String::new(), 0);
    for (s, e, tidy) in cut {
        out.push_str(&body[at..s]);
        at = e;
        if !tidy {
            continue;
        }
        out.truncate(out.trim_end().len());
        // Skip whole blank lines only: the next line's own indentation is part of it.
        at += body[at..]
            .split_inclusive('\n')
            .take_while(|l| l.trim().is_empty())
            .map(str::len)
            .sum::<usize>();
        if !out.is_empty() && at < body.len() {
            out.push_str("\n\n");
        }
    }
    out.push_str(&body[at..]);
    out
}

/// The byte range of the first non-blank line under `section`'s heading, newline included.
fn codes_line(body: &str, section: &Section<'_>) -> (usize, usize) {
    let mut at = section.start;
    for (n, line) in body[section.start..section.end]
        .split_inclusive('\n')
        .enumerate()
    {
        if n > 0 && !line.trim().is_empty() {
            return (at, at + line.len());
        }
        at += line.len();
    }
    (section.start, section.end)
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
        // Scope alone may take its in-scope and out-of-scope sections together.
        let one_of_each = *field == "scope"
            && named.iter().all(|a| {
                named
                    .iter()
                    .filter(|b| b.title.eq_ignore_ascii_case(a.title))
                    .count()
                    == 1
            });
        if named.is_empty() {
            continue;
        }
        if named.len() > 1 && !one_of_each {
            keep(field, "several sections name it");
            continue;
        }
        // In-scope text first, then the out-of-scope text under its label, whatever the order
        // in the body: nothing follows the label, so it cannot be read as in scope.
        let is_out = |s: &Section| s.title.eq_ignore_ascii_case("out of scope");
        let mut parts: Vec<String> = named
            .iter()
            .filter(|s| !is_out(s))
            .map(|s| s.text.to_owned())
            .collect();
        parts.extend(
            named
                .iter()
                .filter(|s| is_out(s))
                .map(|s| format!("Out of scope: {}", s.text)),
        );
        let text = parts.join("\n\n");
        if !before.fields[i].is_empty() {
            keep(field, "the field already has text");
        } else if named.iter().any(|s| s.text.is_empty()) {
            keep(field, "the section is empty");
        } else if *field == "summary" && text.lines().any(|l| l.trim().is_empty()) {
            keep(field, "the summary has several paragraphs");
        } else if text.len() > *cap {
            keep(field, "the section is over the cap");
        } else {
            // The summary box is one line, so a wrapped summary becomes one paragraph.
            after.fields[i] = if *field == "summary" {
                text.split_whitespace().collect::<Vec<_>>().join(" ")
            } else {
                text
            };
            cut.extend(named.iter().map(|s| (s.start, s.end, true)));
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
            cut.push((s.start, s.end, true));
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
        // Only the codes line goes; the heading and every byte after it stay. A section of
        // nothing else goes whole.
        let (lo, hi) = codes_line(&card.body, s);
        if s.text.lines().count() == 1 {
            cut.push((s.start, s.end, true));
        } else {
            cut.push((lo, hi, false));
        }
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

/// Save `plan` as the record file at `path`, which must not exist yet, and sync it and its directory: what
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
        .map_err(|e| e.to_string())?;
    // The file's own sync does not make its directory entry durable: without this a crash could
    // keep the card writes that follow and lose the record that undoes them.
    crate::retag::sync_parent_dir(path)
        .map_err(|e| format!("the record's directory could not be synced: {e}"))
}

/// Whether `card` holds only the first step of a write the store keeps in two (rusty_tick saves
/// the body, then the typed fields and links, and reports `PartlySaved` if the second fails):
/// the lifted body with everything else as before. Both [`apply`] and [`undo`] take it as theirs,
/// so the record that undoes a lift also finishes or reverses a half-written one.
fn partial(card: &Card, l: &Lift) -> bool {
    let held = State::of(card);
    held.body == l.after.body
        && held.body != l.before.body
        && held.fields == l.before.fields
        && held.related == l.before.related
}

/// Write each lift whose card is still at the planned revision. A card changed since is
/// skipped and reported, so running a plan again is safe.
///
/// # Errors
/// The first storage error other than a conflict. Cards before it are written.
pub fn apply(store: &dyn Store, lifts: &[Lift], now_ms: i64) -> Result<Vec<Outcome>, BoardError> {
    write(store, lifts, now_ms, |card, l| {
        (card.rev == l.rev || partial(card, l)).then_some(&l.after)
    })
}

/// Put back each lifted card that still holds what the lift wrote.
///
/// # Errors
/// The first storage error other than a conflict. Cards before it are restored.
pub fn undo(store: &dyn Store, lifts: &[Lift], now_ms: i64) -> Result<Vec<Outcome>, BoardError> {
    write(store, lifts, now_ms, |card, l| {
        (State::of(card) == l.after || partial(card, l)).then_some(&l.before)
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
            "## Objective\nDo it.\n\n## Execution gate\nboiler\n"
        );
        assert!(p.left.is_empty());
    }

    #[test]
    fn ambiguous_full_over_cap_and_fenced_sections_stay_put() {
        let long = "x".repeat(MAX_SUMMARY_BYTES + 1);
        let mut set = card("t", "## Why\nnew\n");
        set.why = "kept".into();
        let cases = [
            (card("t", "## Why\na\n\n## Problem\nb\n"), "several"),
            (card("t", "## Summary\none\n\ntwo\n"), "paragraphs"),
            (set, "already"),
            (card("t", &format!("## Summary\n{long}\n")), "cap"),
            (card("t", "## Why\n"), "empty"),
        ];
        for (c, word) in cases {
            let p = lifted(&c);
            assert!(p.lifts.is_empty(), "{word}: {p:?}");
            assert!(p.left[0].why.contains(word), "{word}: {p:?}");
        }
        // A shorter or labelled fence line inside a longer fence does not close it.
        for body in [
            "````\n```\n## Why\nin code\n```js\n````\n",
            "~~~~\n~~~\n## Why\nx\n~~~~\n",
        ] {
            assert!(lifted(&card("t", body)).lifts.is_empty(), "{body:?}");
        }
        for body in [
            "## Summary\none\r\n\r\ntwo\r\n",
            "## Summary\none\n \ntwo\n",
        ] {
            assert!(lifted(&card("t", body)).lifts.is_empty(), "{body:?}");
        }
        let swapped = lifted(&card("t", "## Out of scope\nthat\n\n## Scope\nthis\n"));
        assert_eq!(
            swapped.lifts[0].after.fields[1],
            "this\n\nOut of scope: that"
        );
        for fence in ["```", "~~~"] {
            let c = card("t", &format!("{fence}\n## Why\nin code\n{fence}\n"));
            assert!(lifted(&c).lifts.is_empty(), "{fence}");
        }
    }

    #[test]
    fn out_of_scope_keeps_its_meaning_and_a_wrapped_summary_becomes_one_line() {
        let c = card(
            "t",
            "## Scope\nthis\n\n## Out of scope\nthat\n\n## Summary\nwrapped\nacross lines\n",
        );
        let p = lifted(&c);
        let [l] = &p.lifts[..] else { panic!("{p:?}") };
        assert_eq!(l.after.fields[1], "this\n\nOut of scope: that");
        assert_eq!(l.after.fields[3], "wrapped across lines");
        let only = lifted(&card("t", "## Out of scope\nthat\n"));
        assert_eq!(only.lifts[0].after.fields[1], "Out of scope: that");
    }

    #[test]
    fn only_the_joins_are_tidied_and_a_top_level_heading_ends_a_section() {
        let body = "intro\n\n\n\n```\na\n\n\n\nb\n```\n\n## Why\nBecause.\n\n# Notes\nkept\n";
        let p = lifted(&card("t", body));
        let [l] = &p.lifts[..] else { panic!("{p:?}") };
        assert_eq!(l.after.fields[2], "Because.");
        assert_eq!(
            l.after.body,
            "intro\n\n\n\n```\na\n\n\n\nb\n```\n\n# Notes\nkept\n"
        );
    }

    #[test]
    fn indented_headings_end_sections_and_prerequisites_keep_their_other_bytes() {
        let c = card(
            "t",
            "## Why\nreason\n\n  ## Execution gate\nleave this here\n",
        );
        let p = lifted(&c);
        let [l] = &p.lifts[..] else { panic!("{p:?}") };
        assert_eq!(l.after.fields[2], "reason");
        assert_eq!(l.after.body, "  ## Execution gate\nleave this here\n");
        let target = card("M1.1 First", "");
        let c = card(
            "M2.1 Second",
            "## Prerequisites\nM1.1\n\n    literal command\n",
        );
        let p = plan(&[target, c]);
        let [l] = &p.lifts[..] else { panic!("{p:?}") };
        assert_eq!(l.after.body, "## Prerequisites\n\n    literal command\n");
    }

    /// A store that keeps only a write's body, as rusty_tick does when its second write fails.
    struct Halves(MemStore);

    impl Store for Halves {
        fn get(&self, id: Uuid) -> Result<Option<Card>, BoardError> {
            self.0.get(id)
        }
        fn list(&self) -> Result<Vec<Card>, BoardError> {
            self.0.list()
        }
        fn insert(&self, card: &Card, event: Event) -> Result<u64, BoardError> {
            self.0.insert(card, event)
        }
        fn replace(&self, card: &Card, event: Event) -> Result<u64, BoardError> {
            let held = self.0.get(card.id)?.expect("card");
            let half = Card {
                body: card.body.clone(),
                ..held.next(card.updated_ms)
            };
            self.0.replace(&half, event)?;
            Err(BoardError::PartlySaved {
                rev: half.rev,
                why: "second write failed".into(),
            })
        }
        fn events_since(
            &self,
            after: Option<Uuid>,
            limit: usize,
        ) -> Result<Vec<Event>, BoardError> {
            self.0.events_since(after, limit)
        }
        fn last_event_id(&self) -> Result<Option<Uuid>, BoardError> {
            self.0.last_event_id()
        }
        fn reserve_number(&self, card: Uuid, floor: u64) -> Result<u64, BoardError> {
            self.0.reserve_number(card, floor)
        }
    }

    #[test]
    fn a_half_written_lift_is_finished_by_apply_and_reversed_by_undo() {
        for finish in [true, false] {
            let store = Halves(MemStore::default());
            let c = card("t", "## Why\nBecause.\n\n## Objective\nx\n");
            store
                .insert(&c, Event::of(&c, EventKind::Created, "t"))
                .expect("insert");
            let p = lifted(&c);
            assert!(matches!(
                apply(&store, &p.lifts, 5),
                Err(BoardError::PartlySaved { .. })
            ));
            let held = store.get(c.id).expect("get").expect("card");
            assert_eq!(
                (held.body.as_str(), held.why.as_str()),
                ("## Objective\nx\n", "")
            );
            let out = if finish {
                apply(&store.0, &p.lifts, 6)
            } else {
                undo(&store.0, &p.lifts, 6)
            }
            .expect("recover");
            assert!(matches!(out[0], Outcome::Done { .. }), "{out:?}");
            let end = store.get(c.id).expect("get").expect("card");
            let want = if finish {
                ("Because.", "## Objective\nx\n")
            } else {
                ("", c.body.as_str())
            };
            assert_eq!((end.why.as_str(), end.body.as_str()), want);
        }
    }

    #[test]
    fn the_record_is_new_and_never_overwritten() {
        let path = std::env::temp_dir().join(format!("lift_record_{}.json", Uuid::new_v4()));
        save_record(&path, &[]).expect("save");
        assert!(save_record(&path, &[]).is_err());
        std::fs::remove_file(&path).expect("clean");
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
        assert_eq!(l.after.body, "## Objective\nx\n");
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
            "## Prerequisites\n\nPlanning does not change status.\n"
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
            ("Because.", "## Objective\nx\n")
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
