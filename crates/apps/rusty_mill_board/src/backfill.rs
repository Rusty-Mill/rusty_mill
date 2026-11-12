//! A one-off backfill of evidence (Nano, 2026-11-12): a card that was Done before evidence
//! existed gets, as evidence, the GitHub pull requests its body and comments already name.
//! Planning is pure ([`plan`]); [`apply`] and [`undo`] write through the [`Store`] port.
//!
//! A pull request is named by its URL (`https://github.com/OWNER/NAME/pull/N`) or, in a
//! project whose repository is mapped to `OWNER/NAME`, as `PR #N` or `pull request #N`. A
//! bare `#N` is left alone: on the board it is as often a card number. Only Done cards that
//! were not dropped and have no evidence yet are filled, so [`undo`] can clear exactly what
//! [`apply`] wrote.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::{BoardError, Card, Event, EventKind, MAX_EVIDENCE};
use crate::store::Store;

/// The actor recorded on backfill events.
pub const ACTOR: &str = "backfill";

const GITHUB: &str = "https://github.com/";

/// Evidence to give one card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fill {
    /// The card.
    pub id: Uuid,
    /// Its board number, for people reading the plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<u64>,
    /// The card's revision when planned.
    pub rev: u64,
    /// The pull request links, in the order the card names them.
    pub links: Vec<String>,
    /// Where the card names each link (the text around it), to review the plan by: one per
    /// link. Nothing checks that a pull request merged, so a person reads these.
    #[serde(default)]
    pub context: Vec<String>,
}

/// A pull request a card names, and the text it was named in.
struct Mention {
    link: String,
    context: String,
}

/// What happened to one planned card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum Outcome {
    /// Written.
    Done {
        /// The card.
        id: Uuid,
    },
    /// Left alone, and why.
    Skipped {
        /// The card.
        id: Uuid,
        /// Why.
        why: String,
    },
}

/// Parse `--backfill-evidence`'s `REPO=OWNER/NAME,...`: which GitHub repository a board
/// repository's bare `PR #N` mentions refer to.
///
/// # Errors
/// A malformed pair or a repeated repository.
pub fn parse_repos(spec: &str) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    for pair in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (repo, github) = pair
            .split_once('=')
            .ok_or_else(|| format!("{pair:?}: expected REPO=OWNER/NAME"))?;
        let (owner, name) = github
            .split_once('/')
            .ok_or_else(|| format!("{pair:?}: expected REPO=OWNER/NAME"))?;
        if !plain(repo) || !plain(owner) || !plain(name) {
            return Err(format!("{pair:?}: expected REPO=OWNER/NAME"));
        }
        if out.insert(repo.to_owned(), github.to_owned()).is_some() {
            return Err(format!("{repo:?} is mapped twice"));
        }
    }
    Ok(out)
}

/// A repository, owner or name: letters, digits, `-`, `_` and `.`.
fn plain(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

/// The cards to fill: Done, not dropped, without evidence, naming at least one pull request.
/// `github_of` gives the `OWNER/NAME` a card's bare `PR #N` mentions refer to, if any. At
/// most [`MAX_EVIDENCE`] links per card, the last named (the ones that merged, as a rule).
pub fn plan(cards: &[Card], github_of: &dyn Fn(&Card) -> Option<String>) -> Vec<Fill> {
    cards
        .iter()
        .filter(|c| c.done_ms.is_some() && c.closed.is_none() && c.evidence.is_empty())
        .filter_map(|c| {
            let named = valid(c, links_in(c, github_of(c).as_deref()));
            (!named.is_empty()).then(|| Fill {
                id: c.id,
                number: c.number,
                rev: c.rev,
                links: named.iter().map(|m| m.link.clone()).collect(),
                context: named.into_iter().map(|m| m.context).collect(),
            })
        })
        .collect()
}

/// Every pull request `card` names, in order, without repeats.
fn links_in(card: &Card, github: Option<&str>) -> Vec<Mention> {
    let texts =
        std::iter::once(card.body.as_str()).chain(card.comments.iter().map(|c| c.text.as_str()));
    let mut out: Vec<Mention> = Vec::new();
    for text in texts {
        for (at, link) in pull_urls(text).into_iter().chain(pr_mentions(text, github)) {
            if !out.iter().any(|m| m.link == link) {
                out.push(Mention {
                    link,
                    context: snippet(text, at),
                });
            }
        }
    }
    out
}

/// About 140 characters of `text` around byte `at` (a char boundary), on one line.
fn snippet(text: &str, at: usize) -> String {
    let before: Vec<char> = text[..at].chars().rev().take(60).collect();
    let around: String = before
        .into_iter()
        .rev()
        .chain(text[at..].chars().take(80))
        .collect();
    around.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The last [`MAX_EVIDENCE`] links the card itself accepts as evidence.
fn valid(card: &Card, links: Vec<Mention>) -> Vec<Mention> {
    let ok: Vec<Mention> = links
        .into_iter()
        .filter(|m| {
            card.add_evidence(std::slice::from_ref(&m.link), card.updated_ms)
                .is_ok()
        })
        .collect();
    let skip = ok.len().saturating_sub(MAX_EVIDENCE);
    ok.into_iter().skip(skip).collect()
}

/// `https://github.com/OWNER/NAME/pull/N` links in `text`, cut to that form (no `/files`,
/// `#issuecomment-...` and so on), with where each starts.
fn pull_urls(text: &str) -> Vec<(usize, String)> {
    text.match_indices(GITHUB)
        .filter_map(|(at, _)| {
            let mut parts = text[at + GITHUB.len()..].splitn(4, '/');
            let (owner, name, pull) = (parts.next()?, parts.next()?, parts.next()?);
            let number = digits(parts.next()?)?;
            (pull == "pull" && plain(owner) && plain(name))
                .then(|| (at, format!("{GITHUB}{owner}/{name}/pull/{number}")))
        })
        .collect()
}

/// `PR #N`, `PR#N` and `pull request #N` in `text`, as links into `github` (`OWNER/NAME`).
fn pr_mentions(text: &str, github: Option<&str>) -> Vec<(usize, String)> {
    let Some(github) = github else {
        return Vec::new();
    };
    let lower = text.to_ascii_lowercase();
    ["pr #", "pr#", "pull request #"]
        .iter()
        .flat_map(|marker| {
            lower.match_indices(marker).filter_map(|(at, m)| {
                let word_start = at == 0 || !lower.as_bytes()[at - 1].is_ascii_alphanumeric();
                let number = digits(&lower[at + m.len()..])?;
                word_start.then(|| (at, format!("{GITHUB}{github}/pull/{number}")))
            })
        })
        .collect::<BTreeMap<usize, String>>()
        .into_iter()
        .collect()
}

/// The pull request number `s` starts with, if any (not 0).
fn digits(s: &str) -> Option<u64> {
    let end = s.bytes().take_while(u8::is_ascii_digit).count();
    s[..end].parse().ok().filter(|n| *n > 0)
}

/// Save `plan` as the record file at `path`, which must not exist yet, and sync it: what
/// [`undo`] reads, written before [`apply`] changes any card.
///
/// # Errors
/// If the file exists or cannot be written.
pub fn save_record(path: &std::path::Path, plan: &[Fill]) -> Result<(), String> {
    let json = serde_json::to_string_pretty(plan).map_err(|e| e.to_string())?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    std::io::Write::write_all(&mut file, json.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|e| e.to_string())
}

/// Give each planned card its links, if it is still at the planned revision with no evidence.
///
/// A card someone changed between the check and the write is skipped and reported, as is one
/// that already has evidence, so running a plan again is safe.
///
/// # Errors
/// The first storage error other than a conflict. Cards before it are written.
pub fn apply(store: &dyn Store, plan: &[Fill], now_ms: i64) -> Result<Vec<Outcome>, BoardError> {
    let mut out = Vec::new();
    for f in plan {
        let Some(card) = store.get(f.id)? else {
            out.push(skipped(f.id, "no longer on the board"));
            continue;
        };
        if card.rev != f.rev || !card.evidence.is_empty() {
            out.push(skipped(f.id, "changed since the plan"));
            continue;
        }
        let next = card.add_evidence(&f.links, now_ms)?;
        match store.replace(&next, Event::of(&next, EventKind::Edited, ACTOR)) {
            Ok(_) => out.push(Outcome::Done { id: f.id }),
            Err(BoardError::Conflict) => out.push(skipped(f.id, "changed while writing")),
            Err(e) => return Err(e),
        }
    }
    Ok(out)
}

/// Clear the evidence of each planned card that still has exactly the planned links.
///
/// # Errors
/// The first storage error. Cards before it are cleared; running it again finishes the rest.
pub fn undo(store: &dyn Store, plan: &[Fill], now_ms: i64) -> Result<Vec<Outcome>, BoardError> {
    let mut out = Vec::new();
    for f in plan {
        let Some(card) = store.get(f.id)? else {
            out.push(skipped(f.id, "no longer on the board"));
            continue;
        };
        if card.evidence != f.links {
            out.push(skipped(f.id, "its evidence is not what the backfill wrote"));
            continue;
        }
        let next = Card {
            evidence: Vec::new(),
            ..card.next(now_ms)
        };
        store.replace(&next, Event::of(&next, EventKind::Edited, ACTOR))?;
        out.push(Outcome::Done { id: f.id });
    }
    Ok(out)
}

fn skipped(id: Uuid, why: &str) -> Outcome {
    Outcome::Skipped {
        id,
        why: why.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{CloseReason, Closure, Column, Comment};
    use crate::store::MemStore;

    fn done(body: &str, comments: &[&str]) -> Card {
        let mut c = Card::new("t", body, Vec::new(), 1).expect("card");
        c.column = Column::DONE;
        c.done_ms = Some(2);
        c.comments = comments
            .iter()
            .map(|t| Comment {
                by: "dev".into(),
                at_ms: 3,
                text: (*t).to_owned(),
            })
            .collect();
        c
    }

    const RM: &str = "Rusty-Mill/rusty_mill";

    fn mapped(_: &Card) -> Option<String> {
        Some(RM.to_owned())
    }

    #[test]
    fn pull_requests_are_found_by_url_and_by_mention_in_a_mapped_repo() {
        let c = done(
            "See https://github.com/o/r/pull/7/files and https://github.com/o/r/issues/8.",
            &[
                "Merged as PR #1521; also pr#12 and Pull Request #13.",
                "card #99 and APR #5",
            ],
        );
        let only =
            |github| -> Vec<String> { links_in(&c, github).into_iter().map(|m| m.link).collect() };
        let links = only(Some(RM));
        assert_eq!(
            links,
            [
                "https://github.com/o/r/pull/7".to_owned(),
                format!("{GITHUB}{RM}/pull/1521"),
                format!("{GITHUB}{RM}/pull/12"),
                format!("{GITHUB}{RM}/pull/13"),
            ]
        );
        assert_eq!(
            only(None),
            ["https://github.com/o/r/pull/7"],
            "unmapped: URLs only"
        );
    }

    #[test]
    fn only_done_undropped_cards_without_evidence_are_planned_with_the_last_eight() {
        let many: Vec<String> = (1..=10).map(|n| format!("PR #{n}")).collect();
        let full = done(&many.join(" "), &[]);
        let open = Card::new("o", "PR #1", Vec::new(), 1).expect("card");
        let dropped = Card {
            closed: Some(Closure {
                reason: CloseReason::WontDo,
                at_ms: 2,
            }),
            done_ms: None,
            ..done("PR #2", &[])
        };
        let has = Card {
            evidence: vec!["https://x.test/1".into()],
            ..done("PR #3", &[])
        };
        let none = done("nothing here", &[]);
        let p = plan(&[full.clone(), open, dropped, has, none], &mapped);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].id, full.id);
        assert_eq!(p[0].links.len(), MAX_EVIDENCE);
        assert!(p[0].links[0].ends_with("/pull/3"), "{:?}", p[0].links);
    }

    #[test]
    fn apply_writes_planned_cards_once_and_undo_clears_only_what_it_wrote() {
        let store = MemStore::default();
        let a = done("PR #1", &[]);
        let b = done("PR #2", &[]);
        for c in [&a, &b] {
            store
                .insert(c, Event::of(c, EventKind::Created, "t"))
                .expect("insert");
        }
        let p = plan(&store.list().expect("list"), &mapped);
        assert_eq!(p.len(), 2);
        let out = apply(&store, &p, 10).expect("apply");
        assert!(
            out.iter().all(|o| matches!(o, Outcome::Done { .. })),
            "{out:?}"
        );
        let again = apply(&store, &p, 11).expect("again");
        assert!(again.iter().all(|o| matches!(o, Outcome::Skipped { .. })));
        let filled = store.get(a.id).expect("get").expect("card");
        assert!(filled.shipped());

        // Someone changes b's evidence by hand: undo leaves it.
        let b_now = store.get(b.id).expect("get").expect("card");
        let edited = b_now
            .add_evidence(&["https://x.test/manual".into()], 12)
            .expect("edit");
        store
            .replace(&edited, Event::of(&edited, EventKind::Edited, "nano"))
            .expect("replace");
        let undone = undo(&store, &p, 13).expect("undo");
        for (f, o) in p.iter().zip(&undone) {
            let cleared = matches!(o, Outcome::Done { .. });
            assert_eq!(cleared, f.id == a.id, "{o:?}");
        }
        assert!(
            store
                .get(a.id)
                .expect("get")
                .expect("card")
                .evidence
                .is_empty()
        );
        assert_eq!(
            store.get(b.id).expect("get").expect("card").evidence.len(),
            2
        );
    }

    #[test]
    fn malformed_pull_urls_and_pr_zero_are_not_evidence() {
        let c = done(
            "https://github.com/o/r/pull/ https://github.com/o/r/pull/x https://github.com/o /r/pull/1 \
             https://github.com/o/r/pull/0 https://github.com/o%20x/r/pull/2 PR #0 \
             https://github.com/o/r/issues/3 https://github.com/o/r/pull/4#issuecomment-9",
            &[],
        );
        let links: Vec<String> = links_in(&c, Some(RM)).into_iter().map(|m| m.link).collect();
        assert_eq!(links, ["https://github.com/o/r/pull/4"]);
    }

    #[test]
    fn the_plan_shows_where_each_pull_request_is_named() {
        let c = done(
            "Superseded by PR #5, which was closed unmerged.",
            &["see PR #6 for the fix"],
        );
        let p = plan(&[c], &mapped);
        assert_eq!(p[0].links.len(), 2);
        assert_eq!(p[0].context.len(), 2);
        assert!(
            p[0].context[0].contains("Superseded by PR #5"),
            "{:?}",
            p[0].context
        );
        assert!(p[0].context[1].contains("see PR #6 for the fix"));
    }

    #[test]
    fn apply_and_undo_skip_missing_and_changed_cards() {
        let store = MemStore::default();
        let c = done("PR #1", &[]);
        store
            .insert(&c, Event::of(&c, EventKind::Created, "t"))
            .expect("insert");
        let p = plan(std::slice::from_ref(&c), &mapped);
        let gone = Fill {
            id: Uuid::new_v4(),
            ..p[0].clone()
        };
        let stale = Fill {
            rev: 99,
            ..p[0].clone()
        };
        let out = apply(&store, &[gone.clone(), stale], 10).expect("apply");
        assert!(
            out.iter().all(|o| matches!(o, Outcome::Skipped { .. })),
            "{out:?}"
        );
        assert!(
            store
                .get(c.id)
                .expect("get")
                .expect("card")
                .evidence
                .is_empty()
        );
        let out = undo(&store, &[gone, p[0].clone()], 11).expect("undo");
        assert!(
            out.iter().all(|o| matches!(o, Outcome::Skipped { .. })),
            "{out:?}"
        );
    }

    #[test]
    fn a_record_is_never_overwritten() {
        let dir = std::env::temp_dir().join(format!("mb-backfill-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("record.json");
        let p = plan(&[done("PR #1", &[])], &mapped);
        save_record(&path, &p).expect("first");
        let saved: Vec<Fill> =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("parse");
        assert_eq!(saved, p);
        assert!(
            save_record(&path, &p).is_err(),
            "an existing record is refused"
        );
        std::fs::remove_dir_all(&dir).expect("clean up");
    }

    #[test]
    fn repo_maps_parse_or_say_why_not() {
        let m = parse_repos("rusty_mill=Rusty-Mill/rusty_mill, pythrust=o/p").expect("parse");
        assert_eq!(m["rusty_mill"], RM);
        for bad in ["x", "x=y", "x=a/b,x=c/d", "x=a/b c", "=a/b"] {
            assert!(parse_repos(bad).is_err(), "{bad}");
        }
    }
}
