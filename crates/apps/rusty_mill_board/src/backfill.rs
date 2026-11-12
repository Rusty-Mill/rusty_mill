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
        let plain = |s: &str| {
            !s.is_empty()
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        };
        if !plain(repo) || !plain(owner) || !plain(name) {
            return Err(format!("{pair:?}: expected REPO=OWNER/NAME"));
        }
        if out.insert(repo.to_owned(), github.to_owned()).is_some() {
            return Err(format!("{repo:?} is mapped twice"));
        }
    }
    Ok(out)
}

/// The cards to fill: Done, not dropped, without evidence, naming at least one pull request.
/// `github_of` gives the `OWNER/NAME` a card's bare `PR #N` mentions refer to, if any. At
/// most [`MAX_EVIDENCE`] links per card, the last named (the ones that merged, as a rule).
pub fn plan(cards: &[Card], github_of: &dyn Fn(&Card) -> Option<String>) -> Vec<Fill> {
    cards
        .iter()
        .filter(|c| c.done_ms.is_some() && c.closed.is_none() && c.evidence.is_empty())
        .filter_map(|c| {
            let links = links_in(c, github_of(c).as_deref());
            let links = valid(c, links);
            (!links.is_empty()).then_some(Fill {
                id: c.id,
                number: c.number,
                rev: c.rev,
                links,
            })
        })
        .collect()
}

/// Every pull request `card` names, in order, without repeats.
fn links_in(card: &Card, github: Option<&str>) -> Vec<String> {
    let texts =
        std::iter::once(card.body.as_str()).chain(card.comments.iter().map(|c| c.text.as_str()));
    let mut out: Vec<String> = Vec::new();
    for text in texts {
        for link in pull_urls(text).into_iter().chain(pr_mentions(text, github)) {
            if !out.contains(&link) {
                out.push(link);
            }
        }
    }
    out
}

/// The last [`MAX_EVIDENCE`] links the card itself accepts as evidence.
fn valid(card: &Card, links: Vec<String>) -> Vec<String> {
    let ok: Vec<String> = links
        .into_iter()
        .filter(|l| {
            card.add_evidence(std::slice::from_ref(l), card.updated_ms)
                .is_ok()
        })
        .collect();
    let skip = ok.len().saturating_sub(MAX_EVIDENCE);
    ok.into_iter().skip(skip).collect()
}

/// `https://github.com/OWNER/NAME/pull/N` links in `text`, cut to that form (no `/files`,
/// `#issuecomment-...` and so on).
fn pull_urls(text: &str) -> Vec<String> {
    text.match_indices(GITHUB)
        .filter_map(|(at, _)| {
            let mut parts = text[at + GITHUB.len()..].splitn(4, '/');
            let (owner, name, pull) = (parts.next()?, parts.next()?, parts.next()?);
            let number = digits(parts.next()?)?;
            (pull == "pull" && !owner.is_empty() && !name.is_empty())
                .then(|| format!("{GITHUB}{owner}/{name}/pull/{number}"))
        })
        .collect()
}

/// `PR #N`, `PR#N` and `pull request #N` in `text`, as links into `github` (`OWNER/NAME`).
fn pr_mentions(text: &str, github: Option<&str>) -> Vec<String> {
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
        .into_values()
        .collect()
}

/// The number `s` starts with, if any.
fn digits(s: &str) -> Option<u64> {
    let end = s.bytes().take_while(u8::is_ascii_digit).count();
    s[..end].parse().ok()
}

/// Give each planned card its links, if it is still at the planned revision with no evidence.
///
/// # Errors
/// The first storage error. Cards before it are written; running the plan again skips them.
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
        store.replace(&next, Event::of(&next, EventKind::Edited, ACTOR))?;
        out.push(Outcome::Done { id: f.id });
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
        let links = links_in(&c, Some(RM));
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
            links_in(&c, None),
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
    fn repo_maps_parse_or_say_why_not() {
        let m = parse_repos("rusty_mill=Rusty-Mill/rusty_mill, pythrust=o/p").expect("parse");
        assert_eq!(m["rusty_mill"], RM);
        for bad in ["x", "x=y", "x=a/b,x=c/d", "x=a/b c", "=a/b"] {
            assert!(parse_repos(bad).is_err(), "{bad}");
        }
    }
}
