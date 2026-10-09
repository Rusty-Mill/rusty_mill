//! Progress across play sessions, and a short training plan (pure, no I/O).
//!
//! Matches are ordered by when they were played (the replay's own date) and grouped into
//! **play sessions**: by the name the uploader gave, else by time — either way a gap over
//! [`SESSION_GAP_S`] starts a new one, so a bulk upload under one name still splits by day. For one player,
//! [`progress`] rolls each session up and ranks the scoring metrics where they sit
//! lowest in their rank bracket into a plan of [`PLAN_LEN`] targets — each with the
//! next bracket's median to aim for and, once there is more than one session, whether
//! the latest session moved toward it. A metric needs [`MIN_MATCHES`] matches before
//! it can be planned: fewer is one match's noise.

use serde::Serialize;

use crate::history::{MetricSnapshot, PlayerSnapshot, SessionRecord};

/// Matches more than this far apart (s) belong to different play sessions.
pub const SESSION_GAP_S: u64 = 2 * 60 * 60;
/// Matches a metric needs before it is planned or its trend claimed.
pub const MIN_MATCHES: usize = 3;
/// Targets in a plan.
pub const PLAN_LEN: usize = 3;
/// A trend counts once the latest session moved this fraction of the gap to target.
const MOVE_FRACTION: f32 = 0.1;

/// One play session, rolled up for one player.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlaySession {
    /// The uploader's name for it, else the day-and-time it started (UTC).
    pub name: String,
    pub start: u64,
    pub matches: usize,
    pub wins: usize,
    pub losses: usize,
    /// Mean decision-discipline composite over its matches.
    pub composite: Option<f32>,
    /// Mean Pacifist headline over its matches.
    pub pacifist: Option<f32>,
}

/// How the latest session moved a metric relative to the sessions before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Trend {
    /// Toward the target.
    Improving,
    Flat,
    /// Away from the target.
    Worse,
    /// Only one session so far — play another, then re-check.
    Unknown,
}

/// One thing to work on.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlanItem {
    pub metric: String,
    /// Mean raw value over the player's matches.
    pub now: f32,
    /// The next bracket up's median (or this bracket's, in the top bracket).
    pub target: f32,
    /// Mean standing within the bracket, 0–100.
    pub pct: f32,
    pub matches: usize,
    pub trend: Trend,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProgressReport {
    pub player: String,
    pub sessions: Vec<PlaySession>,
    pub plan: Vec<PlanItem>,
}

fn mean(xs: impl Iterator<Item = f32>) -> Option<f32> {
    let (sum, n) = xs.fold((0.0, 0usize), |(s, n), x| (s + x, n + 1));
    (n > 0).then(|| sum / n as f32)
}

/// Group `matches` (oldest first) into play sessions: same name (or both unnamed) and close in time.
fn group<'a>(
    matches: &[(&'a SessionRecord, &'a PlayerSnapshot)],
) -> Vec<Vec<(&'a SessionRecord, &'a PlayerSnapshot)>> {
    let mut out: Vec<Vec<(&SessionRecord, &PlayerSnapshot)>> = Vec::new();
    for &(r, p) in matches {
        let joins = out.last().and_then(|g| g.last()).is_some_and(|(prev, _)| {
            match (&prev.session, &r.session) {
                (Some(a), Some(b)) => {
                    a == b && r.when().saturating_sub(prev.when()) <= SESSION_GAP_S
                }
                (None, None) => r.when().saturating_sub(prev.when()) <= SESSION_GAP_S,
                _ => false,
            }
        });
        match out.last_mut() {
            Some(g) if joins => g.push((r, p)),
            _ => out.push(vec![(r, p)]),
        }
    }
    out
}

/// `YYYY-MM-DD HH:MM` (UTC) of a unix time — the default session name.
fn stamp(t: u64) -> String {
    let (days, secs) = (t / 86_400, t % 86_400);
    // Civil-from-days (Howard Hinnant), valid for any date after 1970.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        secs / 3600,
        secs % 3600 / 60
    )
}

fn roll_up(g: &[(&SessionRecord, &PlayerSnapshot)]) -> PlaySession {
    let start = g[0].0.when();
    PlaySession {
        name: g[0].0.session.clone().unwrap_or_else(|| stamp(start)),
        start,
        matches: g.len(),
        wins: g.iter().filter(|(_, p)| p.won == Some(true)).count(),
        losses: g.iter().filter(|(_, p)| p.won == Some(false)).count(),
        composite: mean(g.iter().filter_map(|(_, p)| p.composite)),
        pacifist: mean(g.iter().filter_map(|(_, p)| p.value)),
    }
}

/// This metric's snapshots in the given matches.
fn series<'a>(
    g: &'a [(&'a SessionRecord, &'a PlayerSnapshot)],
    key: &'a str,
) -> impl Iterator<Item = &'a MetricSnapshot> + 'a {
    g.iter()
        .filter_map(move |(_, p)| p.metrics.iter().find(|m| m.key == key))
}

fn trend(
    groups: &[Vec<(&SessionRecord, &PlayerSnapshot)>],
    key: &str,
    now: f32,
    target: f32,
) -> Trend {
    let Some((latest, earlier)) = groups.split_last() else {
        return Trend::Unknown;
    };
    let before = mean(earlier.iter().flat_map(|g| series(g, key)).map(|m| m.raw));
    let after = mean(series(latest, key).map(|m| m.raw));
    let (Some(before), Some(after)) = (before, after) else {
        return Trend::Unknown;
    };
    let toward = (target - now).signum();
    let moved = (after - before) * toward;
    let step = MOVE_FRACTION * (target - now).abs();
    match moved {
        m if m > step => Trend::Improving,
        m if m < -step => Trend::Worse,
        _ => Trend::Flat,
    }
}

/// A player's play sessions and plan from `records`; `None` if they are in none.
pub fn progress(query: &str, records: &[SessionRecord]) -> Option<ProgressReport> {
    let mut mine: Vec<(&SessionRecord, &PlayerSnapshot)> = records
        .iter()
        .filter_map(|r| r.players.iter().find(|p| p.matches(query)).map(|p| (r, p)))
        .collect();
    mine.sort_by_key(|(r, _)| r.when());
    let player = mine.last()?.1.player.clone();
    let groups = group(&mine);

    let mut keys: Vec<&str> = mine
        .iter()
        .flat_map(|(_, p)| &p.metrics)
        .map(|m| m.key.as_str())
        .collect();
    keys.sort_unstable();
    keys.dedup();
    let mut plan: Vec<PlanItem> = keys
        .into_iter()
        .filter_map(|key| {
            let all: Vec<&MetricSnapshot> = mine
                .iter()
                .filter_map(|(_, p)| p.metrics.iter().find(|m| m.key == key))
                .collect();
            if all.len() < MIN_MATCHES {
                return None;
            }
            let now = mean(all.iter().map(|m| m.raw))?;
            let target = mean(all.iter().map(|m| m.next.unwrap_or(m.median)))?;
            Some(PlanItem {
                metric: key.to_string(),
                now,
                target,
                pct: mean(all.iter().map(|m| m.pct))?,
                matches: all.len(),
                trend: trend(&groups, key, now, target),
            })
        })
        .collect();
    plan.sort_by(|a, b| {
        a.pct
            .total_cmp(&b.pct)
            .then_with(|| a.metric.cmp(&b.metric))
    });
    plan.truncate(PLAN_LEN);
    Some(ProgressReport {
        player,
        sessions: groups.iter().map(|g| roll_up(g)).collect(),
        plan,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metric(key: &str, raw: f32, pct: f32) -> MetricSnapshot {
        MetricSnapshot {
            key: key.into(),
            raw,
            pct,
            median: 10.0,
            next: Some(20.0),
        }
    }

    fn rec(
        i: u64,
        saved_at: u64,
        session: Option<&str>,
        won: bool,
        metrics: Vec<MetricSnapshot>,
    ) -> SessionRecord {
        SessionRecord {
            key: format!("k{i}"),
            label: format!("m{i}"),
            saved_at,
            session: session.map(String::from),
            played_at: None,
            players: vec![PlayerSnapshot {
                player: "me".into(),
                won: Some(won),
                composite: Some(50.0 + i as f32),
                metrics,
                ..Default::default()
            }],
        }
    }

    #[test]
    fn matches_group_by_name_else_by_time() {
        let t = 1_700_000_000;
        let recs = vec![
            rec(0, t, None, true, vec![]),
            rec(1, t + 600, None, false, vec![]),
            rec(2, t + SESSION_GAP_S + 601, None, true, vec![]),
            rec(3, t + SESSION_GAP_S + 700, Some("scrims"), true, vec![]),
            rec(4, t + SESSION_GAP_S + 800, Some("scrims"), true, vec![]),
        ];
        let p = progress("me", &recs).unwrap();
        let shape: Vec<_> = p
            .sessions
            .iter()
            .map(|s| (s.matches, s.wins, s.losses))
            .collect();
        assert_eq!(shape, [(2, 1, 1), (1, 1, 0), (2, 2, 0)]);
        assert_eq!(p.sessions[2].name, "scrims");
        assert_eq!(p.sessions[0].name, "2023-11-14 22:13");
    }

    #[test]
    fn a_bulk_upload_under_one_name_splits_by_when_the_matches_were_played() {
        let (saved, day) = (1_800_000_000, 24 * 3600);
        // Saved seconds apart, played on different days, and uploaded out of order.
        let recs: Vec<_> = [(0, 2 * day), (1, 0), (2, 600), (3, day)]
            .into_iter()
            .map(|(i, played)| {
                let mut r = rec(i, saved + i, Some("my-games"), true, vec![]);
                r.played_at = Some(1_700_000_000 + played);
                r
            })
            .collect();
        let p = progress("me", &recs).unwrap();
        let shape: Vec<_> = p.sessions.iter().map(|s| s.matches).collect();
        assert_eq!(
            shape,
            [2, 1, 1],
            "oldest day first, named sessions still split on gaps"
        );
        assert!(p.sessions.iter().all(|s| s.name == "my-games"));
        assert_eq!(p.sessions[0].start, 1_700_000_000);
    }

    #[test]
    fn the_plan_is_the_lowest_metrics_with_enough_matches_and_tracks_the_latest_session() {
        let t = 1_700_000_000;
        let day = 24 * 3600;
        // "boost" sits low and is rising toward 20; "focus" is fine; "rare" has too few matches.
        let recs: Vec<_> = (0..4)
            .map(|i| {
                let mut ms = vec![
                    metric("boost", 5.0 + 3.0 * i as f32, 20.0),
                    metric("focus", 18.0, 80.0),
                ];
                if i == 3 {
                    ms.push(metric("rare", 1.0, 1.0));
                }
                rec(i, t + i * day, None, true, ms)
            })
            .collect();
        let plan = progress("me", &recs).unwrap().plan;
        let names: Vec<_> = plan.iter().map(|p| p.metric.as_str()).collect();
        assert_eq!(
            names,
            ["boost", "focus"],
            "rare has 1 match; lowest pct first"
        );
        assert_eq!((plan[0].target, plan[0].trend), (20.0, Trend::Improving));
        assert_eq!(plan[1].trend, Trend::Flat);
    }

    #[test]
    fn one_session_cannot_show_a_trend_and_an_unknown_player_has_no_report() {
        let t = 1_700_000_000;
        let recs: Vec<_> = (0..3)
            .map(|i| rec(i, t + i, None, true, vec![metric("boost", 5.0, 20.0)]))
            .collect();
        assert_eq!(progress("me", &recs).unwrap().plan[0].trend, Trend::Unknown);
        assert!(progress("nobody", &recs).is_none());
    }
}
