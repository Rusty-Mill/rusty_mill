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
    /// The pull requests, in the order the card first names them.
    pub links: Vec<Link>,
}

/// A pull request a card names, and every place it names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Link {
    /// `https://github.com/OWNER/NAME/pull/N`.
    pub url: String,
    /// The text around each mention, oldest first (at most [`MAX_MENTIONS`]), to review the
    /// plan by. Unless [`keep_merged`] ran, nothing checks that a pull request merged, so a
    /// person reads these: a later "closed unmerged" shows here.
    pub named_in: Vec<String>,
}

/// Most mentions kept per link.
const MAX_MENTIONS: usize = 6;

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

/// A repository, owner or name: letters, digits, `-`, `_` and `.`, never only dots (it
/// goes into a URL path).
pub(crate) fn plain(s: &str) -> bool {
    !s.is_empty()
        && !s.bytes().all(|b| b == b'.')
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
            (!named.is_empty()).then_some(Fill {
                id: c.id,
                number: c.number,
                rev: c.rev,
                links: named,
            })
        })
        .collect()
}

/// Every pull request `card` names, in order, without repeats.
fn links_in(card: &Card, github: Option<&str>) -> Vec<Link> {
    let texts =
        std::iter::once(card.body.as_str()).chain(card.comments.iter().map(|c| c.text.as_str()));
    let mut out: Vec<Link> = Vec::new();
    for text in texts {
        for (at, link) in pull_urls(text).into_iter().chain(pr_mentions(text, github)) {
            let seen = snippet(text, at);
            match out.iter_mut().find(|l| l.url == link) {
                Some(l) if l.named_in.len() < MAX_MENTIONS && !l.named_in.contains(&seen) => {
                    l.named_in.push(seen);
                }
                Some(_) => {}
                None => out.push(Link {
                    url: link,
                    named_in: vec![seen],
                }),
            }
        }
    }
    out
}

/// About 180 characters of `text` around byte `at` (a char boundary), on one line.
fn snippet(text: &str, at: usize) -> String {
    let before: Vec<char> = text[..at].chars().rev().take(100).collect();
    let around: String = before
        .into_iter()
        .rev()
        .chain(text[at..].chars().take(80))
        .collect();
    around.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The last [`MAX_EVIDENCE`] links the card itself accepts as evidence.
fn valid(card: &Card, links: Vec<Link>) -> Vec<Link> {
    let ok: Vec<Link> = links
        .into_iter()
        .filter(|l| {
            card.add_evidence(std::slice::from_ref(&l.url), card.updated_ms)
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

/// Why a merge lookup gave no answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup {
    /// The forge has no such pull request, or the token cannot see it: the link is dropped
    /// and reported.
    Missing,
    /// Anything else (auth, rate limit, transport): the whole check stops, nothing written.
    Failed(String),
}

/// Port: whether a pull request merged.
pub trait Merged {
    /// Whether `OWNER/NAME` pull request `number` merged.
    ///
    /// # Errors
    /// [`Lookup`] when the forge gives no yes or no.
    fn merged(&self, owner: &str, name: &str, number: u64) -> Result<bool, Lookup>;
}

/// A link [`keep_merged`] took out of the plan, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Dropped {
    /// The card.
    pub id: Uuid,
    /// The pull request.
    pub url: String,
    /// `not merged`, or `not found` (also what GitHub answers when the token cannot see the
    /// repository: check the token if a whole repository shows up here).
    pub why: &'static str,
}

/// Keep only the links that merged, asking `forge` once per pull request; a card left with
/// none leaves the plan. Opt-in (`--check-merged`): it turns the plan's `named_in` from the
/// only check into a second one.
///
/// # Errors
/// The first [`Lookup::Failed`], so a bad token or a rate limit never shrinks the plan.
pub fn keep_merged(
    plan: Vec<Fill>,
    forge: &dyn Merged,
) -> Result<(Vec<Fill>, Vec<Dropped>), String> {
    let mut seen: BTreeMap<String, Result<(), &'static str>> = BTreeMap::new();
    let mut dropped = Vec::new();
    let mut kept = Vec::new();
    for mut fill in plan {
        let mut links = Vec::new();
        for link in fill.links {
            let answer = match seen.get(&link.url) {
                Some(v) => *v,
                None => {
                    let v = verdict(forge, &link.url)?;
                    seen.insert(link.url.clone(), v);
                    v
                }
            };
            match answer {
                Ok(()) => links.push(link),
                Err(why) => dropped.push(Dropped {
                    id: fill.id,
                    url: link.url,
                    why,
                }),
            }
        }
        if !links.is_empty() {
            fill.links = links;
            kept.push(fill);
        }
    }
    Ok((kept, dropped))
}

/// How many links each `OWNER/NAME` had not found: a whole repository here usually means
/// the token cannot see it.
#[must_use]
pub fn not_found_per_repo(dropped: &[Dropped]) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for d in dropped.iter().filter(|d| d.why == "not found") {
        if let Some((owner, name, _)) = pull_parts(&d.url) {
            *out.entry(format!("{owner}/{name}")).or_insert(0) += 1;
        }
    }
    out
}

/// `Ok(())` to keep `url`, `Err(why)` to drop it.
fn verdict(forge: &dyn Merged, url: &str) -> Result<Result<(), &'static str>, String> {
    let (owner, name, number) = pull_parts(url).ok_or_else(|| format!("{url}: not a pull URL"))?;
    match forge.merged(owner, name, number) {
        Ok(true) => Ok(Ok(())),
        Ok(false) => Ok(Err("not merged")),
        Err(Lookup::Missing) => Ok(Err("not found")),
        Err(Lookup::Failed(e)) => Err(format!("checking {url}: {e}")),
    }
}

/// `(OWNER, NAME, N)` of a link [`pull_urls`] or [`pr_mentions`] built.
fn pull_parts(url: &str) -> Option<(&str, &str, u64)> {
    let mut parts = url.strip_prefix(GITHUB)?.split('/');
    let (owner, name) = (parts.next()?, parts.next()?);
    let number = parts.next().filter(|p| *p == "pull").and(parts.next())?;
    let n = digits(number).filter(|_| number.bytes().all(|b| b.is_ascii_digit()))?;
    (parts.next().is_none() && plain(owner) && plain(name)).then_some((owner, name, n))
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
        let urls: Vec<String> = f.links.iter().map(|l| l.url.clone()).collect();
        let next = card.add_evidence(&urls, now_ms)?;
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
/// The first storage error other than a conflict. Cards before it are cleared.
pub fn undo(store: &dyn Store, plan: &[Fill], now_ms: i64) -> Result<Vec<Outcome>, BoardError> {
    let mut out = Vec::new();
    for f in plan {
        let Some(card) = store.get(f.id)? else {
            out.push(skipped(f.id, "no longer on the board"));
            continue;
        };
        if !card.evidence.iter().eq(f.links.iter().map(|l| &l.url)) {
            out.push(skipped(f.id, "its evidence is not what the backfill wrote"));
            continue;
        }
        let next = Card {
            evidence: Vec::new(),
            ..card.next(now_ms)
        };
        match store.replace(&next, Event::of(&next, EventKind::Edited, ACTOR)) {
            Ok(_) => out.push(Outcome::Done { id: f.id }),
            Err(BoardError::Conflict) => out.push(skipped(f.id, "changed while writing")),
            Err(e) => return Err(e),
        }
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
            |github| -> Vec<String> { links_in(&c, github).into_iter().map(|l| l.url).collect() };
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
        assert!(p[0].links[0].url.ends_with("/pull/3"), "{:?}", p[0].links);
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
        let links: Vec<String> = links_in(&c, Some(RM)).into_iter().map(|l| l.url).collect();
        assert_eq!(links, ["https://github.com/o/r/pull/4"]);
    }

    #[test]
    fn the_plan_shows_every_place_a_pull_request_is_named() {
        let c = done(
            "Superseded by PR #5, which was closed unmerged.",
            &[
                "see PR #6 for the fix",
                "PR #5 was closed unmerged; PR #6 landed",
                "PR #6",
            ],
        );
        let p = plan(&[c], &mapped);
        assert_eq!(p[0].links.len(), 2);
        let five = &p[0].links[0];
        assert!(five.url.ends_with("/pull/5"));
        assert_eq!(five.named_in.len(), 2, "{five:?}");
        assert!(five.named_in[0].contains("Superseded by PR #5"));
        assert!(
            five.named_in[1].contains("PR #5 was closed unmerged"),
            "a later mention shows"
        );
        assert_eq!(p[0].links[1].named_in.len(), 3, "{:?}", p[0].links[1]);
    }

    /// A store whose writes lose a race.
    struct Racy(MemStore);

    impl Store for Racy {
        fn get(&self, id: Uuid) -> Result<Option<Card>, BoardError> {
            self.0.get(id)
        }
        fn list(&self) -> Result<Vec<Card>, BoardError> {
            self.0.list()
        }
        fn insert(&self, card: &Card, event: Event) -> Result<u64, BoardError> {
            self.0.insert(card, event)
        }
        fn replace(&self, _: &Card, _: Event) -> Result<u64, BoardError> {
            Err(BoardError::Conflict)
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
    fn a_lost_race_is_skipped_by_apply_and_undo_not_fatal() {
        let store = Racy(MemStore::default());
        let c = done("PR #1", &[]);
        store
            .insert(&c, Event::of(&c, EventKind::Created, "t"))
            .expect("insert");
        let p = plan(std::slice::from_ref(&c), &mapped);
        let out = apply(&store, &p, 10).expect("apply goes on");
        assert!(
            matches!(&out[..], [Outcome::Skipped { why, .. }] if why == "changed while writing"),
            "{out:?}"
        );
        // A card that already carries the planned links, whose clearing loses the race.
        let filled = c
            .add_evidence(&[p[0].links[0].url.clone()], 11)
            .expect("fill");
        store
            .0
            .replace(&filled, Event::of(&filled, EventKind::Edited, "t"))
            .expect("write");
        let out = undo(&store, &p, 12).expect("undo goes on");
        assert!(
            matches!(&out[..], [Outcome::Skipped { why, .. }] if why == "changed while writing"),
            "{out:?}"
        );
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
        for bad in [
            "x",
            "x=y",
            "x=a/b,x=c/d",
            "x=a/b c",
            "=a/b",
            "x=../b",
            "x=a/..",
        ] {
            assert!(parse_repos(bad).is_err(), "{bad}");
        }
    }

    /// A forge that answers from a table: merged, unmerged, missing, or failing.
    struct Table(BTreeMap<u64, Result<bool, Lookup>>, std::cell::Cell<usize>);

    impl Merged for Table {
        fn merged(&self, owner: &str, name: &str, number: u64) -> Result<bool, Lookup> {
            assert_eq!((owner, name), ("Rusty-Mill", "rusty_mill"));
            self.1.set(self.1.get() + 1);
            self.0.get(&number).cloned().unwrap_or(Err(Lookup::Missing))
        }
    }

    fn table(rows: &[(u64, Result<bool, Lookup>)]) -> Table {
        Table(rows.iter().cloned().collect(), std::cell::Cell::new(0))
    }

    #[test]
    fn keep_merged_keeps_merged_links_and_reports_the_rest() {
        let a = done("PR #1 then PR #2, and PR #3", &[]);
        let b = done("only PR #2", &[]);
        let c = done("PR #1 again", &[]);
        let p = plan(&[a.clone(), b.clone(), c], &mapped);
        let forge = table(&[(1, Ok(true)), (2, Ok(false))]);
        let (kept, dropped) = keep_merged(p, &forge).expect("check");
        assert_eq!(forge.1.get(), 3, "each pull request is asked once");
        assert_eq!(kept.len(), 2, "b names nothing merged: {kept:?}");
        assert_eq!(kept[0].id, a.id);
        let urls: Vec<&str> = kept[0].links.iter().map(|l| l.url.as_str()).collect();
        assert_eq!(urls, [format!("{GITHUB}{RM}/pull/1")]);
        let why: Vec<(Uuid, &str)> = dropped.iter().map(|d| (d.id, d.why)).collect();
        assert_eq!(
            why,
            [
                (a.id, "not merged"),
                (a.id, "not found"),
                (b.id, "not merged")
            ]
        );
        let missing = not_found_per_repo(&dropped);
        assert_eq!(
            missing.into_iter().collect::<Vec<_>>(),
            [(RM.to_owned(), 1)]
        );
    }

    #[test]
    fn a_failed_lookup_stops_the_check() {
        let p = plan(&[done("PR #1 PR #2", &[])], &mapped);
        let forge = table(&[(1, Ok(true)), (2, Err(Lookup::Failed("403".into())))]);
        let err = keep_merged(p, &forge).expect_err("stops");
        assert!(err.contains("/pull/2") && err.contains("403"), "{err}");
    }

    #[test]
    fn pull_parts_reads_only_the_links_the_plan_builds() {
        assert_eq!(
            pull_parts(&format!("{GITHUB}{RM}/pull/42")),
            Some(("Rusty-Mill", "rusty_mill", 42))
        );
        for bad in [
            "https://github.com/o/r/issues/1",
            "https://github.com/o/r/pull/0",
            "https://github.com/o/r/pull/1/files",
            "https://github.com/o/r/pull/1x",
            "https://github.com/../r/pull/1",
            "https://example.com/o/r/pull/1",
        ] {
            assert_eq!(pull_parts(bad), None, "{bad}");
        }
    }
}
