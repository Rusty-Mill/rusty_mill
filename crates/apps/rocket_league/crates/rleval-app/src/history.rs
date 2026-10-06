//! Per-account match history and cross-match habit detection (pure, no I/O).
//!
//! A [`SessionRecord`] is the slice of one [`Analysis`] worth keeping: for every
//! player, the Pacifist headline, fault counts and per-dimension scores, plus
//! whether their team won. [`habits`] then reads a player's records across
//! matches and answers Spire's core question — *what shows up in my losses that
//! doesn't in my wins?* — with one prioritized [`Focus`].
//!
//! Player identity is the replay's platform id (`steam:…`, `xbox:…`) when it has
//! one — it survives renames — and the exact display name otherwise (bots, and
//! sessions saved before ids were recorded). The two are not merged: an older
//! name-only session is a separate identity from the same person's id-keyed ones.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use replay_scoring::Report;

use crate::pipeline::Analysis;

/// Wins *and* losses needed before a win-vs-loss contrast is claimed as a habit.
/// With fewer, a "pattern" is one match's noise.
pub const MIN_MATCHES_PER_SIDE: usize = 2;
/// Smallest win-minus-loss dimension gap (score points) worth calling a habit.
pub const MIN_HABIT_GAP: f32 = 5.0;

/// One dimension's score in one match, with the evidence weight behind it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DimensionSnapshot {
    pub label: String,
    pub value: f32,
    pub opportunities: usize,
}

/// One scoring metric in one match, against the player's rank bracket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricSnapshot {
    pub key: String,
    pub raw: f32,
    /// Standing within the bracket, 0–100 (higher is better).
    pub pct: f32,
    /// The bracket's median raw for this metric.
    pub median: f32,
    /// The next bracket up's median — the target to climb toward.
    pub next: Option<f32>,
}

/// One player's stored result for one match.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlayerSnapshot {
    pub player: String,
    /// Stable platform identity; `None` for bots and sessions saved before ids
    /// were recorded.
    #[serde(default)]
    pub platform_id: Option<String>,
    pub team: i32,
    /// `None` for a drawn or unscored match.
    pub won: Option<bool>,
    /// Major-capped Pacifist headline; `None` when nothing was scoreable.
    pub value: Option<f32>,
    pub majors: u32,
    pub minors: u32,
    /// Most frequent Minor-fault criterion that match (e.g. `"F9"`).
    pub top_minor_fault: Option<String>,
    pub dimensions: Vec<DimensionSnapshot>,
    /// Decision-discipline composite (0–100) that match; `None` in records saved before it was kept.
    #[serde(default)]
    pub composite: Option<f32>,
    /// Scoring metrics against the rank bracket; empty without rank norms or in older records.
    #[serde(default)]
    pub metrics: Vec<MetricSnapshot>,
}

/// One analysed match as stored in an account's history.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionRecord {
    /// Content hash of the replay bytes — the idempotency key.
    pub key: String,
    /// Human label (normally the uploaded file's stem).
    pub label: String,
    /// Unix seconds when the session was saved; orders the history.
    pub saved_at: u64,
    /// The play session the uploader named; unnamed matches are grouped by time (see `progress`).
    #[serde(default)]
    pub session: Option<String>,
    /// When the match was played (replay header date, unix seconds); `saved_at` for records without one.
    #[serde(default)]
    pub played_at: Option<u64>,
    pub players: Vec<PlayerSnapshot>,
}

impl PlayerSnapshot {
    /// The identity history groups this player under: platform id, else name.
    pub fn key(&self) -> &str {
        self.platform_id.as_deref().unwrap_or(&self.player)
    }

    /// Whether `query` (a platform id or a display name) names this player.
    pub fn matches(&self, query: &str) -> bool {
        self.platform_id.as_deref() == Some(query) || self.player == query
    }
}

impl SessionRecord {
    /// When the match was played, falling back to when it was saved.
    pub fn when(&self) -> u64 {
        self.played_at.unwrap_or(self.saved_at)
    }

    /// Project the parts of `analysis` that history needs.
    pub fn from_analysis(key: String, saved_at: u64, analysis: &Analysis) -> Self {
        let winner = winning_team(&analysis.team_scores);
        let players = analysis
            .pacifist
            .players
            .iter()
            .map(|p| {
                let report = analysis.scores.iter().find(|r| r.target_player == p.player);
                PlayerSnapshot {
                    player: p.player.clone(),
                    platform_id: p.platform_id.clone(),
                    team: p.team,
                    won: winner.map(|w| w == p.team),
                    value: p.value,
                    majors: p.majors,
                    minors: p.minors,
                    top_minor_fault: p.top_minor_fault.as_ref().map(|f| f.criterion.clone()),
                    dimensions: p
                        .dimensions
                        .iter()
                        .map(|d| DimensionSnapshot {
                            label: d.label.clone(),
                            value: d.value,
                            opportunities: d.opportunities,
                        })
                        .collect(),
                    composite: report.map(|r| r.composite),
                    metrics: report.map(metric_snapshots).unwrap_or_default(),
                }
            })
            .collect();
        Self {
            key,
            label: analysis.replay_id.clone(),
            saved_at,
            session: None,
            played_at: analysis.played_at,
            players,
        }
    }

    /// Put this match in the named play session (blank names mean "unnamed").
    pub fn in_session(mut self, name: &str) -> Self {
        let name: String = name
            .chars()
            .filter(|c| !c.is_control())
            .take(MAX_SESSION_NAME)
            .collect();
        self.session = Some(name.trim().to_string()).filter(|n| !n.is_empty());
        self
    }
}

/// Longest stored session name.
pub const MAX_SESSION_NAME: usize = 64;

/// The bracket-relative metrics of `report` (none without rank norms).
fn metric_snapshots(report: &Report) -> Vec<MetricSnapshot> {
    let experimental = |key: &str| {
        report
            .metrics
            .iter()
            .any(|m| m.key == key && m.experimental)
    };
    (report.relative.iter())
        .flat_map(|rel| &rel.metrics)
        .filter(|m| !experimental(&m.key))
        .filter_map(|m| {
            Some(MetricSnapshot {
                key: m.key.clone(),
                raw: m.raw?,
                pct: m.within_rank_pct,
                median: m.bracket_median,
                next: m.next_median,
            })
        })
        .collect()
}

/// The team with the strictly highest score, or `None` for a draw / no scores.
fn winning_team(team_scores: &[(i32, i32)]) -> Option<i32> {
    let best = team_scores.iter().max_by_key(|(_, s)| *s)?;
    let tied = team_scores.iter().filter(|(_, s)| *s == best.1).count() > 1;
    (!tied).then_some(best.0)
}

// ---------------------------------------------------------------------------
// History listing
// ---------------------------------------------------------------------------

/// A player seen in at least one stored match.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlayerEntry {
    /// What to pass back as `?player=`: platform id if known, else the name.
    pub key: String,
    /// Most recent display name.
    pub name: String,
    pub matches: usize,
}

/// One row of the session list.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionEntry {
    pub key: String,
    pub label: String,
    pub saved_at: u64,
    pub players: usize,
}

/// What `GET /api/history` returns.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HistorySummary {
    pub sessions: Vec<SessionEntry>,
    /// Most-seen players first — the account owner is almost always at the top.
    pub players: Vec<PlayerEntry>,
}

pub fn summarize(records: &[SessionRecord]) -> HistorySummary {
    let sessions = records
        .iter()
        .map(|r| SessionEntry {
            key: r.key.clone(),
            label: r.label.clone(),
            saved_at: r.saved_at,
            players: r.players.len(),
        })
        .collect();
    // Records are oldest first, so the last name seen for a key is the current one.
    let mut seen: BTreeMap<&str, (&str, usize)> = BTreeMap::new();
    for p in records.iter().flat_map(|r| &r.players) {
        let e = seen.entry(p.key()).or_insert((p.player.as_str(), 0));
        e.0 = p.player.as_str();
        e.1 += 1;
    }
    let mut players: Vec<PlayerEntry> = seen
        .into_iter()
        .map(|(key, (name, matches))| PlayerEntry {
            key: key.to_string(),
            name: name.to_string(),
            matches,
        })
        .collect();
    players.sort_by(|a, b| b.matches.cmp(&a.matches).then_with(|| a.key.cmp(&b.key)));
    HistorySummary { sessions, players }
}

// ---------------------------------------------------------------------------
// Habits
// ---------------------------------------------------------------------------

/// Why a [`Focus`] was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FocusKind {
    /// Scores meaningfully lower in losses than in wins.
    LossHabit,
    /// Not enough wins/losses to contrast; simply the weakest dimension.
    WeakestDimension,
}

/// The single thing to work on next.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Focus {
    pub kind: FocusKind,
    pub dimension: String,
    /// The dimension's overall score across matches.
    pub score: f32,
    /// Win-minus-loss gap; present only for [`FocusKind::LossHabit`].
    pub gap: Option<f32>,
    pub reason: String,
}

/// One Pacifist dimension across every match of the player.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DimensionHabit {
    pub label: String,
    pub overall: Option<f32>,
    pub in_wins: Option<f32>,
    pub in_losses: Option<f32>,
    /// `in_wins - in_losses`; `None` unless both sides had opportunities.
    pub gap: Option<f32>,
    pub opportunities: usize,
}

/// A Minor-fault criterion that recurs across matches.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RecurringFault {
    pub criterion: String,
    /// Matches in which it was the player's most frequent Minor.
    pub matches: usize,
}

/// One point on the trend line.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrendPoint {
    pub label: String,
    pub value: Option<f32>,
    pub won: Option<bool>,
}

/// A player's cross-match read.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HabitReport {
    /// Most recent display name of the matched player.
    pub player: String,
    pub matches: usize,
    pub wins: usize,
    pub losses: usize,
    pub dimensions: Vec<DimensionHabit>,
    pub focus: Option<Focus>,
    pub recurring_fault: Option<RecurringFault>,
    pub trend: Vec<TrendPoint>,
}

/// Compute a player's [`HabitReport`] from `records` (oldest first). `query` is a
/// platform id or a display name (see [`PlayerSnapshot::matches`]). Returns
/// `None` when the player appears in no record.
pub fn habits(query: &str, records: &[SessionRecord]) -> Option<HabitReport> {
    let mine: Vec<(&SessionRecord, &PlayerSnapshot)> = records
        .iter()
        .filter_map(|r| r.players.iter().find(|p| p.matches(query)).map(|p| (r, p)))
        .collect();
    if mine.is_empty() {
        return None;
    }
    let snaps: Vec<&PlayerSnapshot> = mine.iter().map(|(_, p)| *p).collect();
    let wins = snaps.iter().filter(|p| p.won == Some(true)).count();
    let losses = snaps.iter().filter(|p| p.won == Some(false)).count();

    let dimensions = dimension_habits(&snaps);
    let focus = pick_focus(&dimensions, wins, losses);
    let player = snaps
        .last()
        .map_or(query, |p| p.player.as_str())
        .to_string();
    Some(HabitReport {
        player,
        matches: snaps.len(),
        wins,
        losses,
        dimensions,
        focus,
        recurring_fault: recurring_fault(&snaps),
        trend: mine
            .iter()
            .map(|(r, p)| TrendPoint {
                label: r.label.clone(),
                value: p.value,
                won: p.won,
            })
            .collect(),
    })
}

/// Opportunity-weighted mean of `(value, opportunities)` pairs; `None` when no
/// pair carries any opportunity.
fn weighted_mean(pairs: impl Iterator<Item = (f32, usize)>) -> Option<f32> {
    let (sum, weight) = pairs.fold((0.0_f32, 0_usize), |(s, w), (v, o)| {
        (s + v * o as f32, w + o)
    });
    (weight > 0).then(|| sum / weight as f32)
}

fn dimension_habits(snaps: &[&PlayerSnapshot]) -> Vec<DimensionHabit> {
    // BTreeMap keeps the output order stable regardless of snapshot order.
    let mut labels: BTreeMap<&str, ()> = BTreeMap::new();
    for d in snaps.iter().flat_map(|p| &p.dimensions) {
        labels.insert(d.label.as_str(), ());
    }
    labels
        .into_keys()
        .map(|label| {
            let pairs = |want: Option<bool>| {
                snaps
                    .iter()
                    .filter(move |p| want.is_none() || p.won == want)
                    .flat_map(|p| p.dimensions.iter().filter(|d| d.label == label))
                    .map(|d| (d.value, d.opportunities))
            };
            let in_wins = weighted_mean(pairs(Some(true)));
            let in_losses = weighted_mean(pairs(Some(false)));
            DimensionHabit {
                label: label.to_string(),
                overall: weighted_mean(pairs(None)),
                in_wins,
                in_losses,
                gap: in_wins.zip(in_losses).map(|(w, l)| w - l),
                opportunities: pairs(None).map(|(_, o)| o).sum(),
            }
        })
        .collect()
}

fn pick_focus(dimensions: &[DimensionHabit], wins: usize, losses: usize) -> Option<Focus> {
    if wins >= MIN_MATCHES_PER_SIDE && losses >= MIN_MATCHES_PER_SIDE {
        if let Some(f) = loss_habit(dimensions) {
            return Some(f);
        }
    }
    weakest(dimensions)
}

fn loss_habit(dimensions: &[DimensionHabit]) -> Option<Focus> {
    let top = dimensions
        .iter()
        .filter_map(|d| Some((d, d.gap?, d.overall?)))
        .filter(|(_, gap, _)| *gap >= MIN_HABIT_GAP)
        .max_by(|a, b| a.1.total_cmp(&b.1))?;
    let (d, gap, overall) = top;
    Some(Focus {
        kind: FocusKind::LossHabit,
        dimension: d.label.clone(),
        score: overall,
        gap: Some(gap),
        reason: format!(
            "{} drops {:.0} points in your losses ({:.0} in wins vs {:.0} in losses) — \
             the clearest thing that changes when you lose.",
            d.label,
            gap,
            d.in_wins.unwrap_or_default(),
            d.in_losses.unwrap_or_default(),
        ),
    })
}

fn weakest(dimensions: &[DimensionHabit]) -> Option<Focus> {
    let (d, overall) = dimensions
        .iter()
        .filter_map(|d| Some((d, d.overall?)))
        .min_by(|a, b| a.1.total_cmp(&b.1))?;
    Some(Focus {
        kind: FocusKind::WeakestDimension,
        dimension: d.label.clone(),
        score: overall,
        gap: None,
        reason: format!(
            "{} is your lowest-scoring dimension ({:.0}). Upload more matches, wins and \
             losses both, to see whether it is what costs you games.",
            d.label, overall
        ),
    })
}

fn recurring_fault(snaps: &[&PlayerSnapshot]) -> Option<RecurringFault> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for c in snaps.iter().filter_map(|p| p.top_minor_fault.as_deref()) {
        *counts.entry(c).or_default() += 1;
    }
    counts
        .into_iter()
        .filter(|(_, n)| *n >= 2)
        .max_by_key(|(_, n)| *n)
        .map(|(criterion, matches)| RecurringFault {
            criterion: criterion.to_string(),
            matches,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dim(label: &str, value: f32, opportunities: usize) -> DimensionSnapshot {
        DimensionSnapshot {
            label: label.into(),
            value,
            opportunities,
        }
    }

    fn snap(won: Option<bool>, dims: Vec<DimensionSnapshot>) -> PlayerSnapshot {
        PlayerSnapshot {
            player: "me".into(),
            platform_id: None,
            team: 0,
            won,
            value: Some(50.0),
            majors: 0,
            minors: 1,
            top_minor_fault: Some("F9".into()),
            dimensions: dims,
            ..Default::default()
        }
    }

    fn record(i: usize, p: PlayerSnapshot) -> SessionRecord {
        SessionRecord {
            key: format!("k{i}"),
            label: format!("m{i}"),
            saved_at: i as u64,
            players: vec![p],
            ..Default::default()
        }
    }

    fn history(specs: &[(Option<bool>, f32, f32)]) -> Vec<SessionRecord> {
        specs
            .iter()
            .enumerate()
            .map(|(i, (won, a, b))| {
                record(
                    i,
                    snap(*won, vec![dim("Shadow", *a, 10), dim("Boost", *b, 10)]),
                )
            })
            .collect()
    }

    #[test]
    fn winner_is_the_strictly_higher_score() {
        assert_eq!(winning_team(&[(0, 3), (1, 1)]), Some(0));
        assert_eq!(winning_team(&[(0, 1), (1, 4)]), Some(1));
        assert_eq!(winning_team(&[(0, 2), (1, 2)]), None, "draw");
        assert_eq!(winning_team(&[]), None);
    }

    #[test]
    fn unknown_player_has_no_habits() {
        assert!(habits("nobody", &history(&[(Some(true), 50.0, 50.0)])).is_none());
    }

    #[test]
    fn loss_habit_is_the_dimension_that_drops_most_in_losses() {
        // Shadow: 80 in wins, 50 in losses (gap 30). Boost: 60 vs 58 (gap 2).
        let recs = history(&[
            (Some(true), 80.0, 60.0),
            (Some(true), 80.0, 60.0),
            (Some(false), 50.0, 58.0),
            (Some(false), 50.0, 58.0),
        ]);
        let r = habits("me", &recs).unwrap();
        assert_eq!((r.matches, r.wins, r.losses), (4, 2, 2));
        let f = r.focus.unwrap();
        assert_eq!(f.kind, FocusKind::LossHabit);
        assert_eq!(f.dimension, "Shadow");
        assert!((f.gap.unwrap() - 30.0).abs() < 1e-4);
    }

    #[test]
    fn too_few_losses_falls_back_to_weakest_dimension() {
        let recs = history(&[
            (Some(true), 80.0, 40.0),
            (Some(true), 80.0, 40.0),
            (Some(false), 20.0, 40.0), // only one loss
        ]);
        let f = habits("me", &recs).unwrap().focus.unwrap();
        assert_eq!(f.kind, FocusKind::WeakestDimension);
        assert_eq!(f.dimension, "Boost");
        assert_eq!(f.gap, None);
    }

    #[test]
    fn small_gaps_are_not_called_habits() {
        let recs = history(&[
            (Some(true), 62.0, 40.0),
            (Some(true), 62.0, 40.0),
            (Some(false), 60.0, 40.0),
            (Some(false), 60.0, 40.0),
        ]);
        let f = habits("me", &recs).unwrap().focus.unwrap();
        assert_eq!(f.kind, FocusKind::WeakestDimension);
    }

    #[test]
    fn dimension_means_are_opportunity_weighted() {
        let recs = vec![
            record(0, snap(Some(true), vec![dim("Shadow", 100.0, 1)])),
            record(1, snap(Some(true), vec![dim("Shadow", 0.0, 9)])),
        ];
        let r = habits("me", &recs).unwrap();
        assert!((r.dimensions[0].overall.unwrap() - 10.0).abs() < 1e-4);
        assert_eq!(r.dimensions[0].opportunities, 10);
    }

    #[test]
    fn zero_opportunity_dimensions_have_no_score() {
        let recs = vec![record(0, snap(Some(true), vec![dim("Shadow", 100.0, 0)]))];
        let r = habits("me", &recs).unwrap();
        assert_eq!(r.dimensions[0].overall, None);
        assert!(r.focus.is_none());
    }

    #[test]
    fn draws_count_toward_neither_side() {
        let recs = history(&[(None, 50.0, 50.0), (Some(true), 50.0, 50.0)]);
        let r = habits("me", &recs).unwrap();
        assert_eq!((r.matches, r.wins, r.losses), (2, 1, 0));
    }

    #[test]
    fn recurring_fault_needs_two_matches() {
        let one = history(&[(Some(true), 50.0, 50.0)]);
        assert!(habits("me", &one).unwrap().recurring_fault.is_none());
        let two = history(&[(Some(true), 50.0, 50.0), (Some(false), 50.0, 50.0)]);
        let rf = habits("me", &two).unwrap().recurring_fault.unwrap();
        assert_eq!((rf.criterion.as_str(), rf.matches), ("F9", 2));
    }

    #[test]
    fn summary_lists_players_most_seen_first() {
        let mut recs = history(&[(Some(true), 50.0, 50.0), (Some(false), 50.0, 50.0)]);
        recs[0].players.push(PlayerSnapshot {
            player: "mate".into(),
            ..snap(Some(true), vec![])
        });
        let s = summarize(&recs);
        assert_eq!(s.sessions.len(), 2);
        assert_eq!(
            s.players[0],
            PlayerEntry {
                key: "me".into(),
                name: "me".into(),
                matches: 2
            }
        );
        assert_eq!(s.players[1].name, "mate");
    }

    fn with_id(mut r: SessionRecord, name: &str, id: Option<&str>) -> SessionRecord {
        r.players[0].player = name.into();
        r.players[0].platform_id = id.map(String::from);
        r
    }

    #[test]
    fn a_rename_keeps_one_identity_when_there_is_a_platform_id() {
        let base = history(&[(Some(true), 50.0, 50.0), (Some(false), 50.0, 50.0)]);
        let recs = vec![
            with_id(base[0].clone(), "OldName", Some("steam:1")),
            with_id(base[1].clone(), "NewName", Some("steam:1")),
        ];
        let s = summarize(&recs);
        assert_eq!(s.players.len(), 1, "one person, not two");
        assert_eq!(
            (
                s.players[0].key.as_str(),
                s.players[0].name.as_str(),
                s.players[0].matches
            ),
            ("steam:1", "NewName", 2),
            "keyed by id, shown under the latest name"
        );
        let by_id = habits("steam:1", &recs).unwrap();
        assert_eq!((by_id.matches, by_id.player.as_str()), (2, "NewName"));
        // A name query still finds the matches that used that name.
        assert_eq!(habits("OldName", &recs).unwrap().matches, 1);
    }

    #[test]
    fn name_only_sessions_are_a_separate_identity_from_id_keyed_ones() {
        let base = history(&[(Some(true), 50.0, 50.0), (Some(true), 50.0, 50.0)]);
        let recs = vec![
            with_id(base[0].clone(), "me", None),
            with_id(base[1].clone(), "me", Some("steam:1")),
        ];
        let s = summarize(&recs);
        assert_eq!(s.players.len(), 2);
        assert_eq!(habits("steam:1", &recs).unwrap().matches, 1);
    }

    #[test]
    fn records_saved_before_platform_ids_still_load() {
        let json = r#"{"key":"k","label":"l","saved_at":1,"players":[{"player":"me","team":0,
            "won":true,"value":50.0,"majors":0,"minors":0,"top_minor_fault":null,"dimensions":[]}]}"#;
        let r: SessionRecord = serde_json::from_str(json).unwrap();
        assert_eq!(r.players[0].platform_id, None);
        assert_eq!(r.players[0].key(), "me");
    }
}
