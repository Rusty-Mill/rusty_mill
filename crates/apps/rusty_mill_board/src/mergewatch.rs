//! Merge watch (card #1620, option c): find open cards whose work already merged.
//!
//! Pure: [`scan`] takes the cards and the recently merged pull requests (read by an adapter
//! behind [`MergedPulls`]) and lists the open cards a merge points at. It never writes. A line
//! `Closes card #N` in a merged pull request's body (also `Fixes`/`Resolves`; the word `card`
//! is required, since a bare `Closes #N` makes GitHub close issue N) is a [`Why::Marker`]: the
//! only link a later step may close a card on. Evidence on the card and a plain `card #N`
//! mention are weaker ([`Why::Evidence`], [`Why::Mention`]): they are only listed.

use std::collections::BTreeMap;

use serde::Serialize;
use uuid::Uuid;

use crate::api::Shared;
use crate::backfill::pull_parts;
use crate::domain::{Card, Column};

/// A merged pull request, as the forge reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergedPr {
    /// `https://github.com/OWNER/NAME/pull/N`.
    pub url: String,
    /// Its body (empty when none).
    pub body: String,
}

/// Port: recently merged pull requests of one repository.
pub trait MergedPulls {
    /// The most recently merged pull requests of `OWNER/NAME`, newest activity first.
    ///
    /// # Errors
    /// Why the forge could not be read.
    fn recent_merged(&self, slug: &str) -> Result<Vec<MergedPr>, String>;
}

/// How a merged pull request points at a card, strongest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Why {
    /// Its body says `Closes card #N`.
    Marker,
    /// The card lists it as evidence.
    Evidence,
    /// Its body mentions `card #N`.
    Mention,
}

/// An open card a merged pull request points at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MergedOpen {
    /// The card.
    pub id: Uuid,
    /// Its board number, if assigned.
    pub number: Option<u64>,
    /// Its project.
    pub project: String,
    /// Its title.
    pub title: String,
    /// Its column.
    pub column: Column,
    /// The strongest link found.
    pub why: Why,
    /// The merged pull requests, at most [`MAX_PRS`].
    pub prs: Vec<String>,
}

/// Most pull requests listed per card.
pub const MAX_PRS: usize = 4;

const VERBS: [&str; 9] = [
    "close", "closes", "closed", "fix", "fixes", "fixed", "resolve", "resolves", "resolved",
];

/// The card numbers `body` names, as `(number, closes)`: `closes` for `Closes card #N`.
#[must_use]
pub fn card_refs(body: &str) -> Vec<(u64, bool)> {
    let words: Vec<String> = body
        .split_whitespace()
        .map(|w| w.to_ascii_lowercase())
        .collect();
    let mut out = Vec::new();
    for (i, w) in words.iter().enumerate() {
        let Some(number) = (w == "card")
            .then(|| words.get(i + 1))
            .flatten()
            .and_then(|n| n.strip_prefix('#'))
            .map(|n| n.trim_end_matches(|c: char| !c.is_ascii_digit()))
            .and_then(|n| n.parse::<u64>().ok())
        else {
            continue;
        };
        let closes = i
            .checked_sub(1)
            .and_then(|p| words.get(p))
            .is_some_and(|v| VERBS.contains(&v.trim_matches(|c: char| !c.is_alphabetic())));
        out.push((number, closes));
    }
    out
}

/// Whether `link` is pull request `url`, ignoring a query, fragment or `/files`-style tail.
fn same_pull(link: &str, url: &str) -> bool {
    let bare = link.split(['?', '#']).next().unwrap_or_default();
    let head = bare.split('/').take(7).collect::<Vec<_>>().join("/");
    let (Some(a), Some(b)) = (pull_parts(&head), pull_parts(url)) else {
        return false;
    };
    a == b
}

/// The open cards `prs` point at, strongest link first per card, in card-number order.
#[must_use]
pub fn scan(cards: &[Card], prs: &[MergedPr]) -> Vec<MergedOpen> {
    let by_number: BTreeMap<u64, &Card> = cards
        .iter()
        .filter(|c| c.column != Column::DONE && c.archived_ms.is_none())
        .filter_map(|c| c.number.map(|n| (n, c)))
        .collect();
    let mut found: BTreeMap<Uuid, MergedOpen> = BTreeMap::new();
    let mut note = |card: &Card, why: Why, url: &str| {
        let e = found.entry(card.id).or_insert_with(|| MergedOpen {
            id: card.id,
            number: card.number,
            project: card.project.clone(),
            title: card.title.clone(),
            column: card.column.clone(),
            why,
            prs: Vec::new(),
        });
        e.why = e.why.min(why);
        if e.prs.len() < MAX_PRS && !e.prs.iter().any(|p| p == url) {
            e.prs.push(url.to_owned());
        }
    };
    for pr in prs {
        for (n, closes) in card_refs(&pr.body) {
            if let Some(card) = by_number.get(&n) {
                note(
                    card,
                    if closes { Why::Marker } else { Why::Mention },
                    &pr.url,
                );
            }
        }
        for card in by_number.values() {
            if card.evidence.iter().any(|l| same_pull(l, &pr.url)) {
                note(card, Why::Evidence, &pr.url);
            }
        }
    }
    let mut out: Vec<MergedOpen> = found.into_values().collect();
    out.sort_by_key(|m| (m.number, m.id));
    out
}

/// One watch run: read the recently merged pull requests of every registered GitHub
/// repository, [`scan`] the board's cards and keep the result for `board_metrics`. Reads only;
/// a repository that cannot be read leaves the last result in place.
///
/// # Errors
/// The first repository or board failure.
pub fn watch(board: &Shared, pulls: &dyn MergedPulls) -> Result<usize, String> {
    let projects = board.projects();
    let mut merged = Vec::new();
    for slug in projects.github_slugs() {
        merged.extend(
            pulls
                .recent_merged(slug)
                .map_err(|e| format!("{slug}: {e}"))?,
        );
    }
    let cards = board.list(None).map_err(|e| e.to_string())?;
    let found = scan(&cards, &merged);
    let n = found.len();
    board.set_merged_open(found);
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Board;
    use crate::projects::Projects;
    use crate::store::{MemStore, Store};

    fn card(number: u64, evidence: &[&str]) -> Card {
        let mut c = Card::new("t", "", vec![], 0)
            .expect("valid")
            .numbered(number);
        c.evidence = evidence.iter().map(|s| (*s).to_owned()).collect();
        c
    }

    fn pr(n: u64, body: &str) -> MergedPr {
        MergedPr {
            url: format!("https://github.com/o/r/pull/{n}"),
            body: body.to_owned(),
        }
    }

    #[test]
    fn refs_need_the_word_card_and_tell_closes_from_mention() {
        assert_eq!(card_refs("Closes card #12."), [(12, true)]);
        assert_eq!(card_refs("fixes: card #3"), [(3, true)]);
        assert_eq!(
            card_refs("see card #7, and CARD #8"),
            [(7, false), (8, false)]
        );
        assert!(card_refs("Closes #12").is_empty());
        assert!(card_refs("Closes card #N").is_empty());
    }

    #[test]
    fn the_strongest_link_wins_and_prs_collect() {
        let cards = [
            card(1, &[]),
            card(2, &["https://github.com/o/r/pull/9/files"]),
        ];
        let prs = [
            pr(8, "mentions card #1"),
            pr(9, "nothing"),
            pr(10, "Closes card #1"),
        ];
        let got = scan(&cards, &prs);
        assert_eq!(got.len(), 2);
        assert_eq!((got[0].number, got[0].why), (Some(1), Why::Marker));
        assert_eq!(got[0].prs.len(), 2);
        assert_eq!((got[1].number, got[1].why), (Some(2), Why::Evidence));
    }

    #[test]
    fn done_and_unknown_cards_are_not_listed() {
        let mut done = card(1, &[]);
        done.column = Column::DONE;
        let got = scan(&[done], &[pr(1, "Closes card #1 and card #99")]);
        assert!(got.is_empty());
    }

    struct Fixed(Result<Vec<MergedPr>, String>);

    impl MergedPulls for Fixed {
        fn recent_merged(&self, _: &str) -> Result<Vec<MergedPr>, String> {
            self.0.clone()
        }
    }

    #[test]
    fn watch_feeds_metrics_and_a_failed_read_keeps_the_last_result() {
        let registry = r#"{"version": 2, "repos": {"mill": {"github": "o/r"}},
            "projects": {"p": {"repos": {"mill": ["src"]}}}}"#;
        let board: Shared = Board::new(
            Box::new(MemStore::default()) as Box<dyn Store>,
            crate::board::Roles::default(),
        )
        .with_projects(Projects::parse(registry).expect("registry"));
        let who = board.roles().actor("nano", "human").expect("role");
        let card = board.add_in(&who, "p", "work", "", vec![]).expect("card");
        let n = card.number.expect("numbered");
        let body = format!("Closes card #{n}");
        assert_eq!(watch(&board, &Fixed(Ok(vec![pr(1, &body)]))), Ok(1));
        let m = board.metrics(|_| true).expect("metrics");
        assert_eq!(
            (m.merged_open.len(), m.merged_open[0].why),
            (1, Why::Marker)
        );
        assert!(watch(&board, &Fixed(Err("rate limit".into()))).is_err());
        assert_eq!(
            board.metrics(|_| true).expect("metrics").merged_open.len(),
            1
        );
        assert!(
            board
                .metrics(|_| false)
                .expect("metrics")
                .merged_open
                .is_empty()
        );
    }
}
