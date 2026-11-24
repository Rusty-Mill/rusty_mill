//! Merge watch (card #1620, option c): find open cards whose work already merged.
//!
//! Pure: [`scan`] takes the cards and the recently merged pull requests (read by an adapter
//! behind [`MergedPulls`]) and lists the open cards a merge points at. It never writes. A line
//! `Closes card #N` in a merged pull request's body (also `Fixes`/`Resolves`; the word `card`
//! is required, since a bare `Closes #N` makes GitHub close issue N) is a [`Why::Marker`]: the
//! only link a later step may close a card on. Evidence on the card and a plain `card #N`
//! mention are weaker ([`Why::Evidence`], [`Why::Mention`]): they are only listed.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::Serialize;
use uuid::Uuid;

use crate::api::Shared;
use crate::backfill::pull_parts;
use crate::board::Board;
use crate::domain::{Actor, BoardError, Capability, Card, Column};

/// A merged pull request, as the forge reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergedPr {
    /// The `OWNER/NAME` it was read from: it can only point at cards of projects in that repository.
    pub repo: String,
    /// `https://github.com/OWNER/NAME/pull/N`.
    pub url: String,
    /// Its body (empty when none).
    pub body: String,
}

/// What one repository read returned.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Recent {
    /// Merged pull requests, newest activity first.
    pub prs: Vec<MergedPr>,
    /// More closed pull requests exist than were read, so older merges may be missing.
    pub truncated: bool,
}

/// Port: recently merged pull requests of one repository.
pub trait MergedPulls {
    /// The most recently merged pull requests of `OWNER/NAME`.
    ///
    /// # Errors
    /// Why the forge could not be read.
    fn recent_merged(&self, slug: &str) -> Result<Recent, String>;
}

/// How current [`MergedOpen`] is, reported beside it so a stale or partial list is not read
/// as complete.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct WatchStatus {
    /// When the list was last refreshed, Unix ms; `None` before the first successful run.
    pub checked_ms: Option<i64>,
    /// The last refresh read only the newest closed pull requests of some repository.
    pub truncated: bool,
    /// Whether the latest run failed, so the list is the last good one (see `checked_ms`). The
    /// detail (repository, API response) is logged, never returned: it may be out of scope.
    pub error: bool,
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
    /// The merged pull requests, strongest link first, at most [`MAX_PRS`]; a cap never drops
    /// the pull request that makes `why`.
    pub prs: Vec<PrLink>,
}

/// A merged pull request and how it points at a card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrLink {
    /// The `OWNER/NAME` it was read from; checked again against the card's project on every read.
    pub repo: String,
    /// `https://github.com/OWNER/NAME/pull/N`.
    pub url: String,
    /// How it points at the card.
    pub why: Why,
}

/// Most pull requests listed per card.
pub const MAX_PRS: usize = 4;

const VERBS: [&str; 9] = [
    "close", "closes", "closed", "fix", "fixes", "fixed", "resolve", "resolves", "resolved",
];

/// Columns of leading whitespace, a tab reaching the next multiple of four.
fn indent(line: &str) -> usize {
    let mut cols = 0;
    for c in line.chars() {
        match c {
            ' ' => cols += 1,
            '\t' => cols = (cols / 4 + 1) * 4,
            _ => break,
        }
    }
    cols
}

/// A code fence line: its character and length (three or more of `` ` `` or `~`).
fn fence(line: &str) -> Option<(char, usize)> {
    let t = line.trim_start();
    let ch = t.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let len = t.chars().take_while(|c| *c == ch).count();
    (len >= 3 && indent(line) < 4).then_some((ch, len))
}

/// `text` without its inline code spans: a run of backticks up to the next run of the same
/// length. An unmatched run is literal.
fn without_inline_code(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let run = |i: usize| chars[i..].iter().take_while(|c| **c == '`').count();
    let (mut out, mut i) = (String::new(), 0);
    while i < chars.len() {
        if chars[i] != '`' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let n = run(i);
        let mut j = i + n;
        let close = loop {
            if j >= chars.len() {
                break None;
            }
            if chars[j] == '`' {
                let m = run(j);
                if m == n {
                    break Some(j + m);
                }
                j += m;
            } else {
                j += 1;
            }
        };
        match close {
            Some(end) => i = end,
            None => {
                out.extend(std::iter::repeat_n('`', n));
                i += n;
            }
        }
    }
    out
}

/// The prose lines of a pull request body: no fenced or indented code, HTML comments, quotes,
/// or inline code, which may quote a directive without meaning it. The flag says the line was
/// literal: nothing had to be stripped from it, so what is left is what was written.
fn prose(body: &str) -> Vec<(String, bool)> {
    let (mut open, mut comment) = (None::<(char, usize)>, false);
    let mut out = Vec::new();
    for raw in body.lines() {
        if let Some((ch, len)) = open {
            // Only a bare fence of the same character, at least as long, closes it.
            let t = raw.trim();
            if indent(raw) < 4 && t.len() >= len && t.chars().all(|c| c == ch) {
                open = None;
            }
            continue;
        }
        if !comment {
            if let Some(f) = fence(raw) {
                open = Some(f);
                continue;
            }
            if indent(raw) >= 4 || raw.trim_start().starts_with('>') {
                continue;
            }
        }
        let mut text = String::new();
        let mut rest = raw;
        while !rest.is_empty() {
            if comment {
                match rest.split_once("-->") {
                    Some((_, after)) => (comment, rest) = (false, after),
                    None => break,
                }
            } else if let Some((before, after)) = rest.split_once("<!--") {
                text.push_str(before);
                (comment, rest) = (true, after);
            } else {
                text.push_str(rest);
                break;
            }
        }
        let clean = without_inline_code(&text).trim().to_owned();
        let literal = clean == raw.trim();
        out.push((clean, literal));
    }
    out
}

/// `N` of `#N` or `#N.`: digits only (no sign), nothing else.
fn card_number(word: &str) -> Option<u64> {
    let digits = word.strip_prefix('#')?.trim_end_matches('.');
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
        .then(|| digits.parse().ok())?
}

/// The card numbers `body` names in prose, as `(number, closes)`. `closes` only for a line
/// that is exactly `Closes card #N` (also Fixes/Resolves and their other forms, case
/// ignored, one optional list bullet or trailing period); any other `card #N` is a mention.
#[must_use]
pub fn card_refs(body: &str) -> Vec<(u64, bool)> {
    let mut out = Vec::new();
    for (line, literal) in prose(body) {
        let line = line
            .trim_start_matches(['-', '*'])
            .trim()
            .to_ascii_lowercase();
        let words: Vec<&str> = line.split_whitespace().collect();
        // A directive that needed stripping (code or a comment beside it) is not one to write on.
        if literal
            && let [verb, "card", n] = words[..]
            && VERBS.contains(&verb)
            && let Some(n) = card_number(n)
        {
            out.push((n, true));
            continue;
        }
        for pair in words.windows(2) {
            if pair[0] == "card"
                && let Some(n) = card_number(pair[1].trim_end_matches([',', ';', ')']))
            {
                out.push((n, false));
            }
        }
    }
    out
}

/// Whether `link` is pull request `url`, ignoring a query, fragment or `/files`-style tail.
fn same_pull(link: &str, url: &str) -> bool {
    let (link, url) = (link.to_ascii_lowercase(), url.to_ascii_lowercase());
    let bare = link.split(['?', '#']).next().unwrap_or_default();
    let head = bare.split('/').take(7).collect::<Vec<_>>().join("/");
    let (Some(a), Some(b)) = (pull_parts(&head), pull_parts(&url)) else {
        return false;
    };
    a == b
}

/// The open cards `prs` point at, strongest link first per card, in card-number order. A pull
/// request only points at a card whose project is in the repository it was read from:
/// `github_of(project)` is that project's GitHub `OWNER/NAME`s.
#[must_use]
pub fn scan(
    cards: &[Card],
    prs: &[MergedPr],
    github_of: &dyn Fn(&str) -> Vec<String>,
) -> Vec<MergedOpen> {
    // A number two cards share names neither: like `Board::get_number`, never guess.
    let mut sharing: BTreeMap<u64, Vec<&Card>> = BTreeMap::new();
    for c in cards {
        if let Some(n) = c.number {
            sharing.entry(n).or_default().push(c);
        }
    }
    let by_number: BTreeMap<u64, &Card> = sharing
        .into_iter()
        .filter_map(|(n, v)| match v[..] {
            [c] if c.column != Column::DONE && c.archived_ms.is_none() => Some((n, c)),
            _ => None,
        })
        .collect();
    let mut found: BTreeMap<Uuid, MergedOpen> = BTreeMap::new();
    let mut note = |card: &Card, why: Why, repo: &str, url: &str| {
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
        match e.prs.iter_mut().find(|p| p.url == url) {
            Some(p) => p.why = p.why.min(why),
            None => e.prs.push(PrLink {
                repo: repo.to_owned(),
                url: url.to_owned(),
                why,
            }),
        }
        e.prs.sort_by_key(|p| p.why);
        e.prs.truncate(MAX_PRS);
    };
    let bound: BTreeMap<Uuid, Vec<String>> = by_number
        .values()
        .map(|c| (c.id, github_of(&c.project)))
        .collect();
    for pr in prs {
        let ours = |c: &Card| bound.get(&c.id).is_some_and(|b| b.contains(&pr.repo));
        for (n, closes) in card_refs(&pr.body) {
            if let Some(card) = by_number.get(&n).filter(|c| ours(c)) {
                note(
                    card,
                    if closes { Why::Marker } else { Why::Mention },
                    &pr.repo,
                    &pr.url,
                );
            }
        }
        for card in by_number.values().filter(|c| ours(c)) {
            if card.evidence.iter().any(|l| same_pull(l, &pr.url)) {
                note(card, Why::Evidence, &pr.repo, &pr.url);
            }
        }
    }
    let mut out: Vec<MergedOpen> = found.into_values().collect();
    out.sort_by_key(|m| (m.number, m.id));
    out
}

/// Who merge watch is on the board's history.
pub const ACTOR: &str = "merge-watch";

/// The actor merge watch writes as for one card: `Record` only, limited to the card's project
/// as the snapshot saw it, so a card moved to another project meanwhile is refused.
fn watcher(project: &str) -> Actor {
    Actor {
        name: ACTOR.to_owned(),
        caps: BTreeSet::from([Capability::Record]),
        projects: Some(BTreeSet::from([project.to_owned()])),
    }
}

/// Close each listed card a merged pull request said `Closes card #N` for. Each is checked
/// again at the write ([`Board::close_merged`]: same revision, still the card's repository,
/// not a parent) and tried once: a card that changed since the scan, or that
/// [`Card::closed_by_merge`] refuses (blocked, parked, gated), stays listed for the next run
/// or for a person. A card with only weaker links is never touched.
fn close_marked(board: &Shared, cards: &[Card], found: &[MergedOpen]) {
    let by_id: HashMap<Uuid, &Card> = cards.iter().map(|c| (c.id, c)).collect();
    for o in found.iter().filter(|o| o.why == Why::Marker) {
        let (Some(pr), Some(card)) = (
            o.prs.iter().find(|l| l.why == Why::Marker),
            by_id.get(&o.id),
        ) else {
            continue;
        };
        let actor = watcher(&card.project);
        match board.close_merged(o.id, card.rev, &pr.url, &pr.repo, &actor) {
            Ok(_) => rusty_log::info!(card = ?o.number, pr = %pr.url, "closed by merge"),
            Err(BoardError::Refused(_) | BoardError::Terminal | BoardError::Conflict) => {}
            Err(e) => rusty_log::warn!(card = ?o.number, error = %e, "merge close failed"),
        }
    }
}

/// One watch run: read the recently merged pull requests of every registered GitHub
/// repository, [`scan`] the board's cards and keep the result for `board_metrics`. With
/// `close`, also closes the cards a merged pull request named with a `Closes card #N` line
/// ([`close_marked`]); without it, it only reads.
/// A repository that cannot be read is skipped and flagged ([`WatchStatus::error`]); when none
/// can be read the last list stays, flagged the same way.
///
/// # Errors
/// Every repository (or the board) failed.
pub fn watch(board: &Shared, pulls: &dyn MergedPulls, close: bool) -> Result<usize, String> {
    let projects = board.projects();
    let slugs: Vec<&str> = projects.github_slugs().collect();
    let (mut merged, mut truncated, mut failed) = (Vec::new(), false, Vec::new());
    for slug in &slugs {
        match pulls.recent_merged(slug) {
            Ok(recent) => {
                truncated |= recent.truncated;
                merged.extend(recent.prs);
            }
            // One unreadable repository does not stop the others; the status flags it.
            Err(e) => failed.push(format!("{slug}: {e}")),
        }
    }
    if !slugs.is_empty() && failed.len() == slugs.len() {
        board.note_merge_watch_failed();
        return Err(failed.join("; "));
    }
    let cards = match board.list(None) {
        Ok(cards) => cards,
        Err(e) => {
            board.note_merge_watch_failed();
            return Err(e.to_string());
        }
    };
    for f in &failed {
        rusty_log::warn!(error = %f, "merge watch could not read a repository");
    }
    let github_of = |project: &str| -> Vec<String> {
        projects
            .repos_of(project)
            .filter_map(|r| projects.github_of(r).map(str::to_owned))
            .collect()
    };
    let found = scan(&cards, &merged, &github_of);
    if close {
        close_marked(board, &cards, &found);
    }
    let n = found.len();
    board.set_merged_open(found, truncated, !failed.is_empty());
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{Board, NewCard};
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
            repo: "o/r".to_owned(),
            url: format!("https://github.com/o/r/pull/{n}"),
            body: body.to_owned(),
        }
    }

    #[test]
    fn refs_need_the_word_card_and_tell_closes_from_mention() {
        assert_eq!(card_refs("Closes card #12."), [(12, true)]);
        assert_eq!(card_refs("- fixes card #3"), [(3, true)]);
        assert_eq!(
            card_refs("see card #7, and CARD #8"),
            [(7, false), (8, false)]
        );
        assert!(card_refs("Closes #12").is_empty());
        assert!(card_refs("Closes card #N").is_empty());
        for body in [
            "Closes card #12abc",
            "Closes card #12-other",
            "see card #12x",
        ] {
            assert!(card_refs(body).is_empty(), "{body:?}");
        }
    }

    #[test]
    fn only_a_standalone_directive_in_prose_closes() {
        // Not the whole line, or not prose: a mention at most, never a marker.
        for body in [
            "This never closes card #5 by itself",
            "Closes card #5 and card #6",
            "fixes: card #3",
            "> Closes card #5",
            "`Closes card #5`",
            "```\nCloses card #5\n```",
            "<!-- Closes card #5 -->",
            "<!--\nCloses card #5\n-->",
            "Example:\n\n    Closes card #5",
            "Example:\n\n\tCloses card #5",
            "  \tCloses card #5",
            "````\n```\nx\n```\nCloses card #5\n````",
            "```\n~~~\nCloses card #5\n~~~\n```",
            "```\n    ```\nCloses card #5\n```",
            "``Closes card #5``",
            "Closes card #+5",
            "Closes card #5 <!-- x -->",
            "<!-- x --> Closes card #5",
            "`a` Closes card #5",
            "Closes card #5 `x`",
            "Closes card #5 <!-- start\nend -->",
        ] {
            assert!(
                card_refs(body).iter().all(|(_, closes)| !closes),
                "{body:?}"
            );
        }
        assert_eq!(
            card_refs("Closes card #5\n```\nx\n```\nfixes card #6"),
            [(5, true), (6, true)]
        );
        assert_eq!(card_refs("<!-- x --> Closes card #9"), [(9, false)]);
    }

    #[test]
    fn the_cap_never_drops_the_marker_pull_request() {
        let cards = [card(1, &[])];
        let mut prs: Vec<MergedPr> = (1..=6).map(|n| pr(n, "see card #1")).collect();
        prs.push(pr(7, "Closes card #1"));
        let got = scan(&cards, &prs, &|_| vec!["o/r".to_owned()]);
        assert_eq!(got[0].prs.len(), MAX_PRS);
        assert_eq!(got[0].why, Why::Marker);
        assert_eq!(got[0].prs[0].why, Why::Marker);
        assert!(got[0].prs[0].url.ends_with("/pull/7"));
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
        let got = scan(&cards, &prs, &|_| vec!["o/r".to_owned()]);
        assert_eq!(got.len(), 2);
        assert_eq!((got[0].number, got[0].why), (Some(1), Why::Marker));
        assert_eq!(got[0].prs.len(), 2);
        assert_eq!(got[0].prs[0].why, Why::Marker);
        assert_eq!(got[0].prs[0].why, Why::Marker);
        assert_eq!((got[1].number, got[1].why), (Some(2), Why::Evidence));
    }

    #[test]
    fn done_and_unknown_cards_are_not_listed() {
        let mut done = card(1, &[]);
        done.column = Column::DONE;
        let got = scan(&[done], &[pr(1, "Closes card #1\ncard #99")], &|_| {
            vec!["o/r".to_owned()]
        });
        assert!(got.is_empty());
    }

    struct Fixed(Result<Recent, String>);

    impl MergedPulls for Fixed {
        fn recent_merged(&self, _: &str) -> Result<Recent, String> {
            self.0.clone()
        }
    }

    fn recent(prs: Vec<MergedPr>, truncated: bool) -> Fixed {
        Fixed(Ok(Recent { prs, truncated }))
    }

    const REGISTRY: &str = r#"{"version": 2,
        "repos": {"mill": {"github": "o/r"}, "other": {"github": "o/other"}},
        "projects": {"p": {"repos": {"mill": ["src"]}}, "q": {"repos": {"other": ["src"]}}}}"#;

    fn board() -> Shared {
        Board::new(
            Box::new(MemStore::default()) as Box<dyn Store>,
            crate::board::Roles::default(),
        )
        .with_projects(Projects::parse(REGISTRY).expect("registry"))
    }

    #[test]
    fn a_pull_request_only_points_at_cards_of_its_own_repository() {
        let b = board();
        let who = b.roles().actor("nano", "human").expect("role");
        let mine = b.add_in(&who, "p", "mine", "", vec![]).expect("card");
        let theirs = b.add_in(&who, "q", "theirs", "", vec![]).expect("card");
        let body = format!(
            "Closes card #{}\nsee card #{}",
            mine.number.expect("n"),
            theirs.number.expect("n")
        );
        // Merged in o/r: card of project q (repository o/other) must not appear.
        assert_eq!(watch(&b, &recent(vec![pr(1, &body)], false), false), Ok(1));
        let m = b.metrics(|_| true).expect("metrics");
        assert_eq!(m.merged_open.len(), 1);
        assert_eq!(m.merged_open[0].id, mine.id);
    }

    #[test]
    fn a_cached_match_is_rechecked_and_a_stale_or_partial_list_says_so() {
        let b = board();
        let who = b.roles().actor("nano", "human").expect("role");
        let card = b.add_in(&who, "p", "work", "", vec![]).expect("card");
        let body = format!("Closes card #{}", card.number.expect("n"));
        assert_eq!(watch(&b, &recent(vec![pr(1, &body)], true), false), Ok(1));
        let m = b.metrics(|_| true).expect("metrics");
        assert_eq!(
            (m.merged_open.len(), m.merged_open[0].why),
            (1, Why::Marker)
        );
        assert!(m.merged_watch.truncated && !m.merged_watch.error);
        assert!(m.merged_watch.checked_ms.is_some());
        // A failed refresh keeps the last list, flagged.
        assert!(watch(&b, &Fixed(Err("rate limit".into())), false).is_err());
        let m = b.metrics(|_| true).expect("metrics");
        assert_eq!(m.merged_open.len(), 1);
        // The detail is for the log: the status says only that the refresh failed.
        assert!(m.merged_watch.error);
        assert!(!format!("{:?}", m.merged_watch).contains("rate limit"));
        // Out of the caller's scope: not shown.
        assert!(
            b.metrics(|_| false)
                .expect("metrics")
                .merged_open
                .is_empty()
        );
        // The card is Done before the next refresh: no longer listed.
        b.close(card.id, crate::domain::CloseReason::Superseded, None, &who)
            .expect("closed");
        assert!(b.metrics(|_| true).expect("metrics").merged_open.is_empty());
    }

    #[test]
    fn a_card_reassigned_to_another_repository_drops_the_old_links() {
        let b = board();
        let who = b.roles().actor("nano", "human").expect("role");
        let card = b.add_in(&who, "p", "work", "", vec![]).expect("card");
        let body = format!("Closes card #{}", card.number.expect("n"));
        assert_eq!(watch(&b, &recent(vec![pr(1, &body)], false), false), Ok(1));
        assert_eq!(b.metrics(|_| true).expect("metrics").merged_open.len(), 1);
        // Project q lives in repository o/other: the link read from o/r no longer applies.
        b.assign(card.id, "q", &who).expect("assigned");
        assert!(b.metrics(|_| true).expect("metrics").merged_open.is_empty());
    }

    #[test]
    fn text_after_a_closed_fence_or_unmatched_ticks_is_still_prose() {
        assert_eq!(card_refs("```\nx\n```\nCloses card #5"), [(5, true)]);
        assert_eq!(card_refs("a ` tick\nCloses card #5"), [(5, true)]);
    }

    #[test]
    fn numbers_two_cards_share_and_case_differences_are_handled() {
        let a = card(1, &["https://github.com/O/R/pull/9"]);
        let mut b = card(2, &[]);
        b.title = "other".into();
        let dup = card(2, &[]);
        let prs = [pr(9, "Closes card #2")];
        let got = scan(&[a, b, dup], &prs, &|_| vec!["o/r".to_owned()]);
        // Card 2 is ambiguous: neither card is listed; card 1's evidence matches despite case.
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].number, got[0].why), (Some(1), Why::Evidence));
    }

    #[test]
    fn an_unreadable_repository_is_skipped_and_flagged() {
        struct Half;
        impl MergedPulls for Half {
            fn recent_merged(&self, slug: &str) -> Result<Recent, String> {
                if slug == "o/other" {
                    return Err("404".into());
                }
                Ok(Recent {
                    prs: vec![pr(1, "Closes card #1")],
                    truncated: false,
                })
            }
        }
        let b = board();
        let who = b.roles().actor("nano", "human").expect("role");
        b.add_in(&who, "p", "work", "", vec![]).expect("card");
        assert_eq!(watch(&b, &Half, false), Ok(1));
        let m = b.metrics(|_| true).expect("metrics");
        assert_eq!(m.merged_open.len(), 1);
        assert!(m.merged_watch.error);
        assert!(m.merged_watch.checked_ms.is_some());
    }

    fn merged(b: &Shared, body: &str) -> Result<usize, String> {
        watch(b, &recent(vec![pr(7, body)], false), true)
    }

    fn two_cards() -> (Shared, Actor, Card, u64) {
        let b = board();
        let who = b.roles().actor("nano", "human").expect("role");
        let card = b.add_in(&who, "p", "work", "", vec![]).expect("card");
        let n = card.number.expect("n");
        (b, who, card, n)
    }

    #[test]
    fn a_marker_closes_the_card_with_evidence_a_comment_and_no_claim() {
        let (b, who, card, n) = two_cards();
        b.move_card(card.id, &Column::READY, &who).expect("ready");
        b.move_card(card.id, &Column::BUILDING, &who)
            .expect("building");
        b.claim(card.id, &who).expect("claimed");
        assert_eq!(merged(&b, &format!("Closes card #{n}")), Ok(1));
        let done = b.get(card.id).expect("card");
        assert_eq!(done.column, Column::DONE);
        assert!(done.shipped() && done.claim.is_none());
        assert_eq!(done.evidence, ["https://github.com/o/r/pull/7"]);
        let last = done.comments.last().expect("comment");
        assert_eq!(last.by, ACTOR);
        assert!(last.text.contains("pull/7") && last.text.contains("building"));
        let events = b.events(None, 100).expect("events");
        assert!(
            events
                .iter()
                .any(|e| e.actor == ACTOR && e.column == Column::DONE)
        );
        // Closed once: the next run finds nothing open.
        assert_eq!(merged(&b, &format!("Closes card #{n}")), Ok(0));
    }

    #[test]
    fn weaker_links_and_earlier_prs_never_close() {
        let (b, who, card, n) = two_cards();
        let by_evidence = b
            .add_evidence(card.id, &["https://github.com/o/r/pull/7".into()], &who)
            .expect("evidence");
        assert_eq!(by_evidence.column, Column::BACKLOG);
        // Evidence match, then a plain mention, then no marker at all: card stays open.
        for body in ["", &format!("see card #{n}"), &format!("Closes #{n}")] {
            merged(&b, body).expect("watch");
            assert_ne!(b.get(card.id).expect("card").column, Column::DONE);
        }
    }

    #[test]
    fn without_the_flag_a_marker_only_lists() {
        let (b, _, card, n) = two_cards();
        let body = format!("Closes card #{n}");
        assert_eq!(watch(&b, &recent(vec![pr(7, &body)], false), false), Ok(1));
        assert_eq!(b.get(card.id).expect("card").column, Column::BACKLOG);
    }

    #[test]
    fn blocked_parked_and_parent_cards_are_listed_not_closed() {
        let (b, who, blocked, n_blocked) = two_cards();
        b.move_card(blocked.id, &Column::BLOCKED, &who)
            .expect("blocked");
        let parked = b.add_in(&who, "p", "parked", "", vec![]).expect("card");
        b.move_card(parked.id, &Column::PARKED, &who)
            .expect("parked");
        let parent = b.add_in(&who, "p", "parent", "", vec![]).expect("card");
        let mut child = NewCard::new("child", "", vec![]);
        child.parent = Some(parent.id);
        b.add_card(&who, "p", child).expect("child");
        let body = format!(
            "Closes card #{n_blocked}\nCloses card #{}\nCloses card #{}",
            parked.number.expect("n"),
            parent.number.expect("n")
        );
        assert_eq!(merged(&b, &body), Ok(3));
        for id in [blocked.id, parked.id, parent.id] {
            assert_ne!(b.get(id).expect("card").column, Column::DONE);
        }
    }

    const PR: &str = "https://github.com/o/r/pull/7";

    #[test]
    fn the_write_rechecks_the_card_and_never_retries_a_stale_match() {
        let (b, who, card, _) = two_cards();
        let watcher = watcher("p");
        // Edited since the scan: the revision fence refuses, the card is untouched.
        let stale = card.rev;
        b.comment(card.id, &who, "edited meanwhile")
            .expect("comment");
        assert_eq!(
            b.close_merged(card.id, stale, PR, "o/r", &watcher)
                .map(|c| c.id),
            Err(BoardError::Conflict)
        );
        let fresh = b.get(card.id).expect("card");
        assert_eq!(fresh.column, Column::BACKLOG);
        // The repository must be one of the card's project's now.
        assert!(matches!(
            b.close_merged(card.id, fresh.rev, PR, "o/other", &watcher),
            Err(BoardError::Refused(_))
        ));
        // The watcher is limited to the card's project: a card moved elsewhere is out of scope.
        assert!(matches!(
            b.close_merged(card.id, fresh.rev, PR, "o/r", &super::watcher("q")),
            Err(BoardError::OutOfScope(_))
        ));
        // A parent closes with its children.
        let mut child = NewCard::new("child", "", vec![]);
        child.parent = Some(card.id);
        b.add_card(&who, "p", child).expect("child");
        assert!(matches!(
            b.close_merged(card.id, fresh.rev, PR, "o/r", &watcher),
            Err(BoardError::Refused(_))
        ));
        assert_eq!(b.get(card.id).expect("card").column, Column::BACKLOG);
        // And when everything holds, it commits once.
        let (b, _, card, _) = two_cards();
        let done = b
            .close_merged(card.id, card.rev, PR, "o/r", &watcher)
            .expect("closed");
        assert_eq!(done.column, Column::DONE);
    }
}
