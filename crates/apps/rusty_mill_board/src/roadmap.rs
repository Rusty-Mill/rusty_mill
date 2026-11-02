//! A project's roadmap, derived from its cards: what is happening now, next, later, what is
//! blocked and what shipped lately. Pure: the caller supplies the cards and the time.

use std::cmp::Reverse;
use std::collections::BTreeSet;

use serde::Serialize;
use uuid::Uuid;

use crate::domain::{Card, Column};
use crate::workflow::{Kind, Workflow};

/// How far back Shipped reaches.
pub const SHIPPED_WINDOW_DAYS: i64 = 30;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// A roadmap lane, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Lane {
    /// Work columns (Building, QA, Review by default).
    Now,
    /// Queues (Ready by default).
    Next,
    /// Backlog.
    Later,
    /// Blocked.
    Blocked,
    /// Done within [`SHIPPED_WINDOW_DAYS`].
    Shipped,
}

impl Lane {
    /// All lanes, in display order.
    pub const ALL: [Lane; 5] = [
        Lane::Now,
        Lane::Next,
        Lane::Later,
        Lane::Blocked,
        Lane::Shipped,
    ];

    /// The lane of a column of kind `kind`.
    fn of(kind: Kind) -> Self {
        match kind {
            Kind::Work => Lane::Now,
            Kind::Queue => Lane::Next,
            Kind::Backlog => Lane::Later,
            Kind::Blocked => Lane::Blocked,
            Kind::Done => Lane::Shipped,
        }
    }
}

/// A card as the roadmap shows it: no body, comments or execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RoadmapCard {
    /// Identity.
    pub id: Uuid,
    /// Board-wide number, if assigned.
    pub number: Option<u64>,
    /// Short title.
    pub title: String,
    /// Current column.
    pub column: Column,
    /// Higher is more urgent.
    pub priority: u8,
    /// Labels.
    pub tags: Vec<String>,
    /// Who holds the work, if anyone.
    pub claim: Option<String>,
    /// Dependencies on the board that are not Done yet.
    pub waits_on: Vec<Uuid>,
    /// When it shipped, Unix ms.
    pub done_ms: Option<i64>,
}

/// One lane and its cards, in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LaneView {
    /// Which lane.
    pub lane: Lane,
    /// Its cards, in order.
    pub cards: Vec<RoadmapCard>,
}

/// A project's roadmap: every lane, in [`Lane::ALL`] order, even when empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Roadmap {
    /// The project.
    pub project: String,
    /// Every lane.
    pub lanes: Vec<LaneView>,
}

/// The roadmap of `project`, each card in the lane of its column's kind in `flow`. `cards` is
/// the whole board, so dependencies in other projects resolve. Done cards appear only with a
/// `done_ms` within the window ending at `now_ms`.
pub fn roadmap(flow: &Workflow, cards: &[Card], project: &str, now_ms: i64) -> Roadmap {
    let open: BTreeSet<Uuid> = cards
        .iter()
        .filter(|c| c.column != Column::DONE)
        .map(|c| c.id)
        .collect();
    let since = now_ms.saturating_sub(SHIPPED_WINDOW_DAYS * DAY_MS);
    let mut mine: Vec<&Card> = cards
        .iter()
        .filter(|c| c.project == project)
        .filter(|c| c.column != Column::DONE || c.done_ms.is_some_and(|d| d >= since))
        .collect();
    mine.sort_by_key(|c| (Reverse(c.priority), c.sort_order, c.created_ms, c.id));
    let lanes = Lane::ALL
        .into_iter()
        .map(|lane| {
            let mut in_lane: Vec<&Card> = mine
                .iter()
                .copied()
                .filter(|c| Lane::of(flow.kind(&c.column).unwrap_or(Kind::Backlog)) == lane)
                .collect();
            if lane == Lane::Shipped {
                in_lane.sort_by_key(|c| (Reverse(c.done_ms), c.id));
            }
            LaneView {
                lane,
                cards: in_lane.into_iter().map(|c| view(c, &open)).collect(),
            }
        })
        .collect();
    Roadmap {
        project: project.to_owned(),
        lanes,
    }
}

fn view(card: &Card, open: &BTreeSet<Uuid>) -> RoadmapCard {
    RoadmapCard {
        id: card.id,
        number: card.number,
        title: card.title.clone(),
        column: card.column.clone(),
        priority: card.priority,
        tags: card.tags.clone(),
        claim: card.claim.clone(),
        waits_on: card
            .deps
            .iter()
            .copied()
            .filter(|d| open.contains(d))
            .collect(),
        done_ms: card.done_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 100 * DAY_MS;
    const WINDOW: i64 = SHIPPED_WINDOW_DAYS * DAY_MS;

    fn card(title: &str, column: Column) -> Card {
        let mut c = Card::new(title, "", vec![], 0).expect("valid");
        c.column = column;
        c.project = "p".into();
        c
    }

    fn done(title: &str, done_ms: Option<i64>) -> Card {
        Card {
            done_ms,
            ..card(title, Column::DONE)
        }
    }

    fn titles(r: &Roadmap, lane: Lane) -> Vec<&str> {
        r.lanes
            .iter()
            .find(|l| l.lane == lane)
            .expect("every lane present")
            .cards
            .iter()
            .map(|c| c.title.as_str())
            .collect()
    }

    #[test]
    fn columns_map_to_lanes() {
        let cards: Vec<Card> = Workflow::default()
            .columns()
            .iter()
            .map(|col| match col.kind {
                Kind::Done => done("done", Some(NOW)),
                _ => card(&col.label, col.id.clone()),
            })
            .collect();
        let r = roadmap(&Workflow::default(), &cards, "p", NOW);
        let mut now = titles(&r, Lane::Now);
        now.sort_unstable();
        assert_eq!(now, ["Building", "QA", "Review"]);
        assert_eq!(titles(&r, Lane::Next), ["Ready"]);
        assert_eq!(titles(&r, Lane::Later), ["Backlog"]);
        assert_eq!(titles(&r, Lane::Blocked), ["Blocked"]);
        assert_eq!(titles(&r, Lane::Shipped), ["done"]);
    }

    #[test]
    fn empty_project_has_every_lane_in_order() {
        let r = roadmap(&Workflow::default(), &[], "p", NOW);
        let lanes: Vec<Lane> = r.lanes.iter().map(|l| l.lane).collect();
        assert_eq!(lanes, Lane::ALL);
        assert!(r.lanes.iter().all(|l| l.cards.is_empty()));
    }

    #[test]
    fn lanes_order_by_priority_then_manual_order_then_age() {
        let mut low = card("low", Column::READY);
        low.created_ms = 1;
        let mut high_late = card("high_late", Column::READY);
        high_late.priority = 5;
        high_late.sort_order = 2;
        let mut high_early = card("high_early", Column::READY);
        high_early.priority = 5;
        high_early.sort_order = 1;
        let mut oldest = card("oldest", Column::READY);
        let r = roadmap(
            &Workflow::default(),
            &[low.clone(), high_late, high_early, oldest.clone()],
            "p",
            NOW,
        );
        assert_eq!(
            titles(&r, Lane::Next),
            ["high_early", "high_late", "oldest", "low"]
        );
        // Equal on everything else: the id decides, so the order is stable.
        oldest.created_ms = 1;
        let r = roadmap(
            &Workflow::default(),
            &[low.clone(), oldest.clone()],
            "p",
            NOW,
        );
        let expected = if low.id < oldest.id {
            ["low", "oldest"]
        } else {
            ["oldest", "low"]
        };
        assert_eq!(titles(&r, Lane::Next), expected);
    }

    #[test]
    fn shipped_is_windowed_on_done_ms_and_newest_first() {
        let cards = [
            done("edge", Some(NOW - WINDOW)),
            done("too_old", Some(NOW - WINDOW - 1)),
            done("fresh", Some(NOW - 1)),
        ];
        assert_eq!(
            titles(
                &roadmap(&Workflow::default(), &cards, "p", NOW),
                Lane::Shipped
            ),
            ["fresh", "edge"]
        );
    }

    #[test]
    fn a_done_card_touched_today_keeps_its_ship_date() {
        let old = Card {
            updated_ms: NOW,
            ..done("old", Some(NOW - WINDOW - 1))
        };
        assert!(
            titles(
                &roadmap(&Workflow::default(), &[old], "p", NOW),
                Lane::Shipped
            )
            .is_empty()
        );
    }

    #[test]
    fn closed_without_shipping_is_not_shown() {
        assert!(
            titles(
                &roadmap(&Workflow::default(), &[done("wontdo", None)], "p", NOW),
                Lane::Shipped
            )
            .is_empty()
        );
    }

    #[test]
    fn other_projects_are_left_out() {
        let mut other = card("other", Column::READY);
        other.project = "q".into();
        assert!(
            titles(
                &roadmap(&Workflow::default(), &[other], "p", NOW),
                Lane::Next
            )
            .is_empty()
        );
    }

    #[test]
    fn waits_on_lists_open_deps_across_projects() {
        let mut elsewhere = card("elsewhere", Column::BUILDING);
        elsewhere.project = "q".into();
        let shipped = done("shipped", Some(NOW));
        let mut c = card("c", Column::READY);
        c.deps = vec![elsewhere.id, shipped.id, Uuid::new_v4()];
        let r = roadmap(
            &Workflow::default(),
            &[elsewhere.clone(), shipped, c],
            "p",
            NOW,
        );
        let next = &r.lanes[1].cards[0];
        assert_eq!(
            next.waits_on,
            [elsewhere.id],
            "done and missing deps do not block"
        );
    }
}
