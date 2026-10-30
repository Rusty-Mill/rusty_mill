//! Board metrics, derived from cards: work in progress, flow per week, lead time and the cards
//! idle longest. Pure: the caller supplies the cards (already narrowed to what it may see) and
//! the time.

use std::collections::BTreeMap;

use serde::Serialize;
use uuid::Uuid;

use crate::domain::{Card, Column, PRIORITIES};

/// Weeks of flow history, the current week last.
pub const WEEKS: usize = 8;
/// How far back lead time and per-project shipped counts reach.
pub const RECENT_DAYS: i64 = 30;
/// How many idle cards are listed.
pub const IDLE_LIMIT: usize = 5;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;
const WEEK_MS: i64 = 7 * DAY_MS;

/// A count under some key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Tally<K> {
    /// What was counted.
    pub key: K,
    /// How many.
    pub count: usize,
}

/// One project's open cards and those it shipped within [`RECENT_DAYS`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectTally {
    /// The project.
    pub project: String,
    /// Cards not Done.
    pub open: usize,
    /// Cards shipped within [`RECENT_DAYS`].
    pub shipped: usize,
}

/// Seven days of flow, ending at `start_ms + 7 days` (the last one ends now).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Week {
    /// Start of the week, Unix ms.
    pub start_ms: i64,
    /// Cards created in it.
    pub added: usize,
    /// Cards shipped in it.
    pub done: usize,
}

/// Created-to-shipped time of the cards shipped within [`RECENT_DAYS`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LeadTime {
    /// Cards shipped in the window.
    pub shipped: usize,
    /// Median, ms; `None` when nothing shipped.
    pub median_ms: Option<i64>,
    /// 85th percentile (nearest rank), ms; `None` when nothing shipped.
    pub p85_ms: Option<i64>,
}

/// An open card and how long since it last changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IdleCard {
    /// Identity.
    pub id: Uuid,
    /// Board-wide number, if assigned.
    pub number: Option<u64>,
    /// Short title.
    pub title: String,
    /// The project.
    pub project: String,
    /// Current column.
    pub column: Column,
    /// Who holds the work, if anyone.
    pub claim: Option<String>,
    /// Time since its last change, ms.
    pub idle_ms: i64,
}

/// Metrics over a set of cards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Metrics {
    /// Cards not Done.
    pub open: usize,
    /// Open cards per column, every open column in board order.
    pub by_column: Vec<Tally<Column>>,
    /// Per project, most open first.
    pub by_project: Vec<ProjectTally>,
    /// Open cards per priority, most urgent first, every priority.
    pub by_priority: Vec<Tally<u8>>,
    /// Claimed cards in Building, QA or Review per claimant, most first.
    pub by_claim: Vec<Tally<String>>,
    /// The last [`WEEKS`] weeks, oldest first.
    pub weeks: Vec<Week>,
    /// Lead time of recent work.
    pub lead_time: LeadTime,
    /// Up to [`IDLE_LIMIT`] open cards past Backlog, idle longest first.
    pub idle: Vec<IdleCard>,
}

/// The metrics of `cards` as of `now_ms`.
pub fn metrics(cards: &[Card], now_ms: i64) -> Metrics {
    let open: Vec<&Card> = cards.iter().filter(|c| c.column != Column::Done).collect();
    let since = now_ms.saturating_sub(RECENT_DAYS * DAY_MS);
    let recent: Vec<&Card> = cards
        .iter()
        .filter(|c| c.done_ms.is_some_and(|d| d >= since))
        .collect();
    Metrics {
        open: open.len(),
        by_column: by_column(&open),
        by_project: by_project(&open, &recent),
        by_priority: PRIORITIES
            .into_iter()
            .rev()
            .map(|p| Tally {
                key: p,
                count: open.iter().filter(|c| c.priority == p).count(),
            })
            .collect(),
        by_claim: by_claim(&open),
        weeks: weeks(cards, now_ms),
        lead_time: lead_time(&recent),
        idle: idle(&open, now_ms),
    }
}

fn by_column(open: &[&Card]) -> Vec<Tally<Column>> {
    Column::ALL
        .into_iter()
        .filter(|&col| col != Column::Done)
        .map(|col| Tally {
            key: col,
            count: open.iter().filter(|c| c.column == col).count(),
        })
        .collect()
}

fn by_project(open: &[&Card], recent: &[&Card]) -> Vec<ProjectTally> {
    let mut per: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    open.iter()
        .for_each(|c| per.entry(&c.project).or_default().0 += 1);
    recent
        .iter()
        .for_each(|c| per.entry(&c.project).or_default().1 += 1);
    let mut out: Vec<ProjectTally> = per
        .into_iter()
        .map(|(project, (open, shipped))| ProjectTally {
            project: project.to_owned(),
            open,
            shipped,
        })
        .collect();
    out.sort_by(|a, b| b.open.cmp(&a.open).then_with(|| a.project.cmp(&b.project)));
    out
}

fn by_claim(open: &[&Card]) -> Vec<Tally<String>> {
    let mut per: BTreeMap<&str, usize> = BTreeMap::new();
    open.iter()
        .filter(|c| matches!(c.column, Column::Building | Column::Qa | Column::Review))
        .filter_map(|c| c.claim.as_deref())
        .for_each(|who| *per.entry(who).or_default() += 1);
    let mut out: Vec<Tally<String>> = per
        .into_iter()
        .map(|(who, count)| Tally {
            key: who.to_owned(),
            count,
        })
        .collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.key.cmp(&b.key)));
    out
}

/// Which of the last [`WEEKS`] weeks `at_ms` falls in, oldest `0`; a future time counts as now.
fn week_of(at_ms: i64, now_ms: i64) -> Option<usize> {
    let back = usize::try_from(now_ms.saturating_sub(at_ms).max(0) / WEEK_MS).ok()?;
    (back < WEEKS).then(|| WEEKS - 1 - back)
}

fn weeks(cards: &[Card], now_ms: i64) -> Vec<Week> {
    let mut out: Vec<Week> = (0..WEEKS)
        .map(|i| Week {
            start_ms: now_ms - (WEEKS - i) as i64 * WEEK_MS,
            added: 0,
            done: 0,
        })
        .collect();
    for c in cards {
        if let Some(w) = week_of(c.created_ms, now_ms) {
            out[w].added += 1;
        }
        if let Some(w) = c.done_ms.and_then(|d| week_of(d, now_ms)) {
            out[w].done += 1;
        }
    }
    out
}

fn lead_time(recent: &[&Card]) -> LeadTime {
    let mut spans: Vec<i64> = recent
        .iter()
        .filter_map(|c| c.done_ms.map(|d| d.saturating_sub(c.created_ms).max(0)))
        .collect();
    spans.sort_unstable();
    LeadTime {
        shipped: spans.len(),
        median_ms: percentile(&spans, 50),
        p85_ms: percentile(&spans, 85),
    }
}

/// Nearest-rank percentile of sorted `values`.
fn percentile(values: &[i64], pct: usize) -> Option<i64> {
    let rank = (values.len() * pct).div_ceil(100).max(1);
    values.get(rank - 1).copied()
}

fn idle(open: &[&Card], now_ms: i64) -> Vec<IdleCard> {
    let mut past_backlog: Vec<&Card> = open
        .iter()
        .copied()
        .filter(|c| c.column != Column::Backlog)
        .collect();
    past_backlog.sort_by_key(|c| (c.updated_ms, c.id));
    past_backlog
        .into_iter()
        .take(IDLE_LIMIT)
        .map(|c| IdleCard {
            id: c.id,
            number: c.number,
            title: c.title.clone(),
            project: c.project.clone(),
            column: c.column,
            claim: c.claim.clone(),
            idle_ms: now_ms.saturating_sub(c.updated_ms).max(0),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 100 * DAY_MS;

    fn card(title: &str, column: Column) -> Card {
        let mut c = Card::new(title, "", vec![], 0).expect("valid");
        c.column = column;
        c.project = "p".into();
        c.created_ms = NOW;
        c.updated_ms = NOW;
        c
    }

    fn shipped(title: &str, created_ms: i64, done_ms: i64) -> Card {
        Card {
            created_ms,
            done_ms: Some(done_ms),
            ..card(title, Column::Done)
        }
    }

    #[test]
    fn empty_board_has_every_bucket_and_no_lead_time() {
        let m = metrics(&[], NOW);
        assert_eq!(m.open, 0);
        let cols: Vec<Column> = m.by_column.iter().map(|t| t.key).collect();
        assert_eq!(
            cols,
            [
                Column::Backlog,
                Column::Ready,
                Column::Building,
                Column::Qa,
                Column::Review,
                Column::Blocked
            ]
        );
        let prios: Vec<u8> = m.by_priority.iter().map(|t| t.key).collect();
        assert_eq!(prios, [5, 3, 1, 0]);
        assert_eq!(m.weeks.len(), WEEKS);
        assert_eq!(m.weeks[WEEKS - 1].start_ms, NOW - WEEK_MS);
        assert_eq!(
            m.lead_time,
            LeadTime {
                shipped: 0,
                median_ms: None,
                p85_ms: None
            }
        );
        assert!(m.by_project.is_empty() && m.by_claim.is_empty() && m.idle.is_empty());
    }

    #[test]
    fn open_counts_leave_done_out() {
        let mut high = card("high", Column::Ready);
        high.priority = 5;
        let cards = [high, card("b", Column::Backlog), shipped("s", NOW, NOW)];
        let m = metrics(&cards, NOW);
        assert_eq!(m.open, 2);
        assert_eq!(m.by_column[0].count, 1, "backlog");
        assert_eq!(m.by_column[1].count, 1, "ready");
        assert_eq!(m.by_priority[0], Tally { key: 5, count: 1 });
        assert_eq!(m.by_priority[3], Tally { key: 0, count: 1 });
    }

    #[test]
    fn projects_count_open_and_recently_shipped_most_open_first() {
        let mut q1 = card("q1", Column::Ready);
        q1.project = "q".into();
        let mut q2 = card("q2", Column::Building);
        q2.project = "q".into();
        let old = shipped("old", 0, NOW - RECENT_DAYS * DAY_MS - 1);
        let cards = [q1, q2, shipped("new", NOW - DAY_MS, NOW), old];
        let m = metrics(&cards, NOW);
        assert_eq!(
            m.by_project,
            [
                ProjectTally {
                    project: "q".into(),
                    open: 2,
                    shipped: 0
                },
                ProjectTally {
                    project: "p".into(),
                    open: 0,
                    shipped: 1
                },
            ]
        );
    }

    #[test]
    fn claims_count_only_work_in_progress() {
        let claimed = |col: Column, who: &str| Card {
            claim: Some(who.into()),
            ..card("c", col)
        };
        let cards = [
            claimed(Column::Building, "b"),
            claimed(Column::Review, "a"),
            claimed(Column::Qa, "a"),
            claimed(Column::Ready, "z"),
            card("free", Column::Building),
        ];
        let m = metrics(&cards, NOW);
        assert_eq!(
            m.by_claim,
            [
                Tally {
                    key: "a".to_owned(),
                    count: 2
                },
                Tally {
                    key: "b".to_owned(),
                    count: 1
                }
            ]
        );
    }

    #[test]
    fn weeks_bucket_added_and_done_oldest_first() {
        let cards = [
            shipped("this week", NOW - DAY_MS, NOW),
            shipped("edge", NOW - WEEK_MS, NOW - WEEK_MS),
            shipped("oldest kept", NOW - 8 * WEEK_MS + 1, NOW - 8 * WEEK_MS + 1),
            shipped("too old", NOW - 8 * WEEK_MS, NOW - 8 * WEEK_MS),
            Card {
                created_ms: NOW + DAY_MS,
                ..card("clock skew", Column::Backlog)
            },
        ];
        let m = metrics(&cards, NOW);
        let added: Vec<usize> = m.weeks.iter().map(|w| w.added).collect();
        let done: Vec<usize> = m.weeks.iter().map(|w| w.done).collect();
        assert_eq!(added, [1, 0, 0, 0, 0, 0, 1, 2]);
        assert_eq!(done, [1, 0, 0, 0, 0, 0, 1, 1]);
    }

    #[test]
    fn lead_time_uses_recent_ships_with_nearest_rank() {
        let cards: Vec<Card> = (1..=10)
            .map(|d| shipped("s", NOW - d * DAY_MS, NOW))
            .chain([
                shipped("closed long ago", 0, NOW - RECENT_DAYS * DAY_MS - 1),
                Card {
                    done_ms: None,
                    ..card("won't do", Column::Done)
                },
            ])
            .collect();
        let lt = metrics(&cards, NOW).lead_time;
        assert_eq!(lt.shipped, 10);
        assert_eq!(lt.median_ms, Some(5 * DAY_MS));
        assert_eq!(lt.p85_ms, Some(9 * DAY_MS));
    }

    #[test]
    fn single_ship_is_its_own_median_and_p85() {
        let lt = metrics(&[shipped("s", NOW - DAY_MS, NOW)], NOW).lead_time;
        assert_eq!((lt.median_ms, lt.p85_ms), (Some(DAY_MS), Some(DAY_MS)));
    }

    #[test]
    fn idle_lists_open_cards_past_backlog_stalest_first_capped() {
        let mut cards: Vec<Card> = (0..7)
            .map(|i| Card {
                updated_ms: NOW - i * DAY_MS,
                ..card(&format!("c{i}"), Column::Ready)
            })
            .collect();
        cards.push(Card {
            updated_ms: 0,
            ..card("backlog", Column::Backlog)
        });
        cards.push(Card {
            updated_ms: 0,
            ..shipped("done", 0, 0)
        });
        let idle = metrics(&cards, NOW).idle;
        let titles: Vec<&str> = idle.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, ["c6", "c5", "c4", "c3", "c2"]);
        assert_eq!(idle[0].idle_ms, 6 * DAY_MS);
    }
}
