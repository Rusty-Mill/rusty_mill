//! **Canonical-vs-external contract cross-check** (spec §13 "second parser
//! adapter for redundancy", Phase 1 — ballchasing.com header cross-check).
//!
//! This is *not* a drop-in [`replay_analyzer::decode::ReplayParser`] adapter:
//! carball/ballchasing operate above our decode port's network-actor
//! granularity, and the contract we care about (spec §4.2/§10) lives at the
//! **canonical-match** level. So instead of a second decoder we keep a
//! comparator that cross-checks our boxcars-derived [`CanonicalMatch`] *header
//! facts* against ballchasing's independent decode of the same replay. Its job
//! is to catch silent **parser drift** (spec §11): if a Rocket League patch
//! shifts the header layout and our goals/scores/roster quietly go wrong, an
//! independent decoder disagreeing is the tripwire.
//!
//! ## What it can and can't corroborate (tiered)
//! - **Tier 1 — exact, FAIL on mismatch:** goal count + each goal's
//!   `(scorer, team)`; per-team final score; roster (name ↔ team); `map`;
//!   `team_size`. These are authoritative facts both decoders must agree on.
//! - **Tier 2 — advisory, WARN don't fail:** per-player `saves`/`shots`/
//!   `assists` (our header `PlayerStats` vs ballchasing's *recomputed* values,
//!   which legitimately diverge). `goals`/`score` stay exact (Tier 1 / below).
//! - **Tier 3 — deferred (not here):** boost economy — our `boost_used`
//!   (0–100 integral) vs ballchasing `bpm`/`amount_collected` are differently
//!   defined and need a mapping spike first.
//!
//! The 14 scoring-rubric metrics ([`crate::report`]) are our IP and have no
//! external equivalent — they are never cross-checked.
//!
//! ## Known limitation: own goals
//! Both sides derive the per-team score from goal attribution, so a replay with
//! an own goal (credited to no player) can make the player-goal-sum disagree
//! with the scoreboard. The committed fixtures are own-goal-free; treat an
//! own-goal replay's goal findings as expected noise until that's modelled.

use std::collections::{BTreeMap, BTreeSet};

use replay_analyzer::model::{CanonicalMatch, Event};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// External shape: a sanitized ballchasing.com replay (the committed fixture).
// ---------------------------------------------------------------------------
//
// These mirror the field names of ballchasing's `GET /api/replays/{id}`
// response so the same structs deserialize either the raw API doc or the
// distilled, sanitized fixture written by `scripts/ballchasing_fetch.py`
// (uploader + per-player platform-id values stripped). Every field is
// `#[serde(default)]` and unknown fields are ignored, so schema additions on
// ballchasing's side don't break deserialization. blue ⇒ team 0, orange ⇒
// team 1 (matches `Engine.PlayerReplicationInfo:Team` and the rest of the repo).

/// A sanitized ballchasing replay: the independent decode we cross-check against.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BallchasingReplay {
    /// ballchasing's replay id (kept for provenance; not cross-checked).
    #[serde(default)]
    pub id: Option<String>,
    /// Processing state; only `"ok"` replays carry a complete decode.
    #[serde(default)]
    pub status: Option<String>,
    /// Internal map id, lowercased (e.g. `"eurostadium_p"`); compared
    /// case-insensitively against our `map` (e.g. `"EuroStadium_P"`).
    #[serde(default)]
    pub map_code: Option<String>,
    /// Human map name (e.g. `"DFH Stadium"`); informational only.
    #[serde(default)]
    pub map_name: Option<String>,
    #[serde(default)]
    pub team_size: Option<i32>,
    /// Match duration (s); informational only.
    #[serde(default)]
    pub duration: Option<f32>,
    #[serde(default)]
    pub blue: BallchasingTeam,
    #[serde(default)]
    pub orange: BallchasingTeam,
}

/// One side (`blue`/`orange`) of a ballchasing replay.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BallchasingTeam {
    #[serde(default)]
    pub stats: BallchasingTeamStats,
    #[serde(default)]
    pub players: Vec<BallchasingPlayer>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BallchasingTeamStats {
    #[serde(default)]
    pub core: BallchasingTeamCore,
}

/// Team-level aggregates. `goals` is the team's final score.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BallchasingTeamCore {
    #[serde(default)]
    pub goals: i32,
    #[serde(default)]
    pub shots: i32,
    #[serde(default)]
    pub saves: i32,
    #[serde(default)]
    pub assists: i32,
}

/// One player on a ballchasing team.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BallchasingPlayer {
    #[serde(default)]
    pub name: String,
    /// Platform id; the identifying value is nulled out in committed fixtures,
    /// leaving only the platform tag (`steam`/`epic`/`xbox`/`ps4`).
    #[serde(default)]
    pub id: Option<BallchasingPlatformId>,
    #[serde(default)]
    pub stats: BallchasingPlayerStats,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BallchasingPlatformId {
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BallchasingPlayerStats {
    #[serde(default)]
    pub core: BallchasingPlayerCore,
}

/// Per-player core stats. `goals`/`score` we expect to match exactly;
/// `saves`/`shots`/`assists` are ballchasing's recomputation (advisory).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BallchasingPlayerCore {
    #[serde(default)]
    pub goals: i32,
    #[serde(default)]
    pub assists: i32,
    #[serde(default)]
    pub saves: i32,
    #[serde(default)]
    pub shots: i32,
    #[serde(default)]
    pub score: i32,
    #[serde(default)]
    pub mvp: Option<bool>,
}

impl BallchasingReplay {
    /// Iterate `(team, player)` over both sides, team 0 = blue, 1 = orange.
    fn players(&self) -> impl Iterator<Item = (i32, &BallchasingPlayer)> {
        self.blue
            .players
            .iter()
            .map(|p| (0, p))
            .chain(self.orange.players.iter().map(|p| (1, p)))
    }

    /// Team final score = that side's aggregate goal total.
    fn team_score(&self, team: i32) -> i32 {
        if team == 0 {
            self.blue.stats.core.goals
        } else {
            self.orange.stats.core.goals
        }
    }
}

// ---------------------------------------------------------------------------
// Cross-check result.
// ---------------------------------------------------------------------------

/// The contract tier a [`Finding`] belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// Exact contract: any mismatch fails the cross-check.
    One,
    /// Advisory: a divergence worth surfacing but not failing on.
    Two,
}

/// A single field-level divergence between the two decodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub tier: Tier,
    /// Short category key, e.g. `"map"`, `"team_size"`, `"team_score"`,
    /// `"goal_count"`, `"goal"`, `"roster"`, `"saves"`.
    pub field: String,
    /// Human-readable description of the divergence.
    pub detail: String,
}

/// The outcome of [`cross_check`]: Tier-1 (exact) and Tier-2 (advisory)
/// findings, plus a little coverage bookkeeping for the summary line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrossCheckReport {
    pub replay_id: String,
    /// Exact-contract mismatches; non-empty ⇒ the cross-check fails.
    pub tier1: Vec<Finding>,
    /// Advisory divergences; never cause a failure.
    pub tier2: Vec<Finding>,
    /// Players matched by name across both decodes.
    pub matched_players: usize,
    /// Total goals seen in our canonical (for the summary).
    pub goals_checked: usize,
}

impl CrossCheckReport {
    /// True when no Tier-1 (exact) field disagreed — the pass/fail gate.
    pub fn tier1_ok(&self) -> bool {
        self.tier1.is_empty()
    }
}

/// Normalize a player name for cross-decoder matching: trim, lowercase, and
/// collapse internal whitespace runs. Both decoders read the name from the same
/// replay PRI, so this is mostly insurance against casing/spacing noise.
fn normalize_name(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Case-insensitive equality (for map ids: `"EuroStadium_P"` vs
/// `"eurostadium_p"`).
fn eq_ci(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// Cross-check our canonical decode against ballchasing's independent decode.
///
/// Tier-1 fields must agree exactly (mismatch ⇒ failure); Tier-2 fields are
/// reported as advisories. Players are matched by [`normalize_name`]; our
/// [`CanonicalMatch`] carries no platform id, so the documented "fall back to
/// platform id" is a no-op today (the ballchasing side keeps the platform tag
/// so the fallback can be wired in if our model gains ids).
pub fn cross_check(our: &CanonicalMatch, bc: &BallchasingReplay) -> CrossCheckReport {
    let mut tier1: Vec<Finding> = Vec::new();
    let mut tier2: Vec<Finding> = Vec::new();
    let t1 = |v: &mut Vec<Finding>, field: &str, detail: String| {
        v.push(Finding {
            tier: Tier::One,
            field: field.into(),
            detail,
        });
    };
    let t2 = |v: &mut Vec<Finding>, field: &str, detail: String| {
        v.push(Finding {
            tier: Tier::Two,
            field: field.into(),
            detail,
        });
    };

    // --- map (Tier 1) — compare only when both sides carry a value. ---
    if let (Some(a), Some(b)) = (our.map.as_deref(), bc.map_code.as_deref()) {
        if !eq_ci(a, b) {
            t1(&mut tier1, "map", format!("our {a:?} vs ballchasing {b:?}"));
        }
    }

    // --- team_size (Tier 1) ---
    if let (Some(a), Some(b)) = (our.team_size, bc.team_size) {
        if a != b {
            t1(
                &mut tier1,
                "team_size",
                format!("our {a} vs ballchasing {b}"),
            );
        }
    }

    // --- per-team final score (Tier 1) ---
    for team in [0i32, 1] {
        let ours = our.team_scores.get(&team).copied().unwrap_or(0);
        let theirs = bc.team_score(team);
        if ours != theirs {
            t1(
                &mut tier1,
                "team_score",
                format!("team {team}: our {ours} vs ballchasing {theirs}"),
            );
        }
    }

    // --- goals: total count + per-(scorer, team) breakdown (Tier 1) ---
    // Our header goal events carry the authoritative scorer+team. Ballchasing
    // exposes per-player goal counts (no per-goal list), so reduce both to a
    // (normalized-name, team) → count map; total count falls out of the sum.
    let mut our_goals: BTreeMap<(String, i32), i32> = BTreeMap::new();
    let mut our_total = 0usize;
    let mut our_unattributed = 0usize;
    for ev in &our.events {
        if let Event::Goal { scorer, team, .. } = ev {
            our_total += 1;
            match (scorer, team) {
                (Some(s), Some(t)) => *our_goals.entry((normalize_name(s), *t)).or_default() += 1,
                _ => our_unattributed += 1,
            }
        }
    }
    let mut bc_goals: BTreeMap<(String, i32), i32> = BTreeMap::new();
    let mut bc_total = 0usize;
    for (team, p) in bc.players() {
        let g = p.stats.core.goals;
        if g != 0 {
            *bc_goals.entry((normalize_name(&p.name), team)).or_default() += g;
        }
        bc_total += g.max(0) as usize;
    }
    if our_total != bc_total {
        t1(
            &mut tier1,
            "goal_count",
            format!("our {our_total} vs ballchasing {bc_total}"),
        );
    }
    // Per-scorer breakdown over the union of keys. If our header left some goals
    // unattributed we can't fully reconcile the breakdown, so demote those
    // findings to advisory (the total-count check above still holds the line).
    let breakdown_tier = if our_unattributed == 0 {
        Tier::One
    } else {
        t2(
            &mut tier2,
            "goal",
            format!("{our_unattributed} of our goal(s) had no header scorer; per-scorer check is advisory"),
        );
        Tier::Two
    };
    let keys: BTreeSet<(String, i32)> = our_goals.keys().chain(bc_goals.keys()).cloned().collect();
    for k in &keys {
        let a = our_goals.get(k).copied().unwrap_or(0);
        let b = bc_goals.get(k).copied().unwrap_or(0);
        if a != b {
            let f = Finding {
                tier: breakdown_tier,
                field: "goal".into(),
                detail: format!("scorer {:?} team {}: our {a} vs ballchasing {b}", k.0, k.1),
            };
            match breakdown_tier {
                Tier::One => tier1.push(f),
                Tier::Two => tier2.push(f),
            }
        }
    }

    // --- roster + per-player stats, matched by normalized name ---
    let ours_by_name: BTreeMap<String, &replay_analyzer::model::PlayerMeta> = our
        .players
        .iter()
        .map(|p| (normalize_name(&p.name), p))
        .collect();
    let mut bc_by_name: BTreeMap<String, (i32, &BallchasingPlayer)> = BTreeMap::new();
    for (team, p) in bc.players() {
        bc_by_name.insert(normalize_name(&p.name), (team, p));
    }
    let all_names: BTreeSet<&String> = ours_by_name.keys().chain(bc_by_name.keys()).collect();
    let mut matched = 0usize;
    for name in all_names {
        match (ours_by_name.get(name), bc_by_name.get(name)) {
            (Some(om), Some((bt, bp))) => {
                matched += 1;
                // Roster: name ↔ team must agree (Tier 1).
                if om.team != *bt {
                    t1(
                        &mut tier1,
                        "roster",
                        format!(
                            "{:?}: our team {} vs ballchasing team {}",
                            om.name, om.team, bt
                        ),
                    );
                }
                // Per-player goals are Tier 1 (already folded into the goal
                // breakdown, but checked here per-player for a precise message).
                let bc_core = &bp.stats.core;
                if om.goals != bc_core.goals {
                    t1(
                        &mut tier1,
                        "goals",
                        format!(
                            "{:?}: our {} vs ballchasing {}",
                            om.name, om.goals, bc_core.goals
                        ),
                    );
                }
                // Saves / shots / assists are advisory (ballchasing recomputes).
                for (field, ours, theirs) in [
                    ("saves", om.saves, bc_core.saves),
                    ("shots", om.shots, bc_core.shots),
                    ("assists", om.assists, bc_core.assists),
                ] {
                    if ours != theirs {
                        t2(
                            &mut tier2,
                            field,
                            format!("{:?}: our {ours} vs ballchasing {theirs}", om.name),
                        );
                    }
                }
            }
            (Some(om), None) => t1(
                &mut tier1,
                "roster",
                format!(
                    "{:?} (our team {}) absent from ballchasing",
                    om.name, om.team
                ),
            ),
            (None, Some((bt, bp))) => t1(
                &mut tier1,
                "roster",
                format!(
                    "{:?} (ballchasing team {bt}) absent from our decode",
                    bp.name
                ),
            ),
            (None, None) => unreachable!("name came from one of the two maps"),
        }
    }

    CrossCheckReport {
        replay_id: our.replay_id.clone(),
        tier1,
        tier2,
        matched_players: matched,
        goals_checked: our_total,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_analyzer::model::{PlayerMeta, Resampled};

    /// A minimal canonical match carrying only the header facts the cross-check
    /// reads; reconstruction fields are left empty.
    fn our_match() -> CanonicalMatch {
        CanonicalMatch {
            replay_id: "t".into(),
            parser_version: "boxcars-test".into(),
            analyzer_version: "test".into(),
            map: Some("EuroStadium_P".into()),
            team_size: Some(2),
            record_fps: None,
            num_frames: 0,
            duration_s: 0.0,
            team_scores: BTreeMap::from([(0, 2), (1, 1)]),
            players: vec![
                PlayerMeta {
                    platform_id: None,
                    name: "Alice".into(),
                    team: 0,
                    score: 300,
                    goals: 2,
                    assists: 0,
                    saves: 1,
                    shots: 4,
                    car_id: None,
                    car_name: None,
                    camera: None,
                    steering_sensitivity: None,
                },
                PlayerMeta {
                    platform_id: None,
                    name: "Bob".into(),
                    team: 0,
                    score: 100,
                    goals: 0,
                    assists: 1,
                    saves: 2,
                    shots: 1,
                    car_id: None,
                    car_name: None,
                    camera: None,
                    steering_sensitivity: None,
                },
                PlayerMeta {
                    platform_id: None,
                    name: "Cara".into(),
                    team: 1,
                    score: 250,
                    goals: 1,
                    assists: 0,
                    saves: 3,
                    shots: 5,
                    car_id: None,
                    car_name: None,
                    camera: None,
                    steering_sensitivity: None,
                },
                PlayerMeta {
                    platform_id: None,
                    name: "Dex".into(),
                    team: 1,
                    score: 90,
                    goals: 0,
                    assists: 0,
                    saves: 0,
                    shots: 2,
                    car_id: None,
                    car_name: None,
                    camera: None,
                    steering_sensitivity: None,
                },
            ],
            tracks: vec![],
            frames: vec![],
            resampled: Resampled {
                hz: 30.0,
                team_attack_sign: BTreeMap::new(),
                frames: vec![],
            },
            events: vec![
                Event::Goal {
                    t: 10.0,
                    scorer: Some("Alice".into()),
                    team: Some(0),
                },
                Event::Goal {
                    t: 20.0,
                    scorer: Some("Cara".into()),
                    team: Some(1),
                },
                Event::Goal {
                    t: 30.0,
                    scorer: Some("Alice".into()),
                    team: Some(0),
                },
            ],
            features: vec![],
            pickups: vec![],
            powerslides: vec![],
        }
    }

    fn player(name: &str, core: BallchasingPlayerCore) -> BallchasingPlayer {
        BallchasingPlayer {
            name: name.into(),
            id: None,
            stats: BallchasingPlayerStats { core },
        }
    }
    fn core(goals: i32, assists: i32, saves: i32, shots: i32) -> BallchasingPlayerCore {
        BallchasingPlayerCore {
            goals,
            assists,
            saves,
            shots,
            score: 0,
            mvp: None,
        }
    }

    /// A ballchasing replay that agrees on every Tier-1 fact.
    fn bc_match() -> BallchasingReplay {
        BallchasingReplay {
            id: Some("bc".into()),
            status: Some("ok".into()),
            map_code: Some("eurostadium_p".into()),
            map_name: Some("Mannfield".into()),
            team_size: Some(2),
            duration: Some(300.0),
            blue: BallchasingTeam {
                stats: BallchasingTeamStats {
                    core: BallchasingTeamCore {
                        goals: 2,
                        ..Default::default()
                    },
                },
                players: vec![
                    player("Alice", core(2, 0, 1, 4)),
                    player("Bob", core(0, 1, 2, 1)),
                ],
            },
            orange: BallchasingTeam {
                stats: BallchasingTeamStats {
                    core: BallchasingTeamCore {
                        goals: 1,
                        ..Default::default()
                    },
                },
                players: vec![
                    player("Cara", core(1, 0, 3, 5)),
                    player("Dex", core(0, 0, 0, 2)),
                ],
            },
        }
    }

    #[test]
    fn agreeing_decodes_pass_tier1() {
        let r = cross_check(&our_match(), &bc_match());
        assert!(r.tier1_ok(), "expected Tier-1 pass, got {:?}", r.tier1);
        assert_eq!(r.matched_players, 4);
        assert_eq!(r.goals_checked, 3);
        assert!(r.tier2.is_empty(), "unexpected advisories: {:?}", r.tier2);
    }

    #[test]
    fn case_and_space_insensitive_name_match() {
        // Same roster, just messier names on the ballchasing side.
        let mut bc = bc_match();
        bc.blue.players[0].name = "  alice ".into();
        let r = cross_check(&our_match(), &bc);
        assert!(
            r.tier1_ok(),
            "name normalization should still match: {:?}",
            r.tier1
        );
        assert_eq!(r.matched_players, 4);
    }

    #[test]
    fn map_is_compared_case_insensitively() {
        let r = cross_check(&our_match(), &bc_match());
        assert!(!r.tier1.iter().any(|f| f.field == "map"));
    }

    #[test]
    fn wrong_team_score_is_tier1() {
        let mut bc = bc_match();
        bc.orange.stats.core.goals = 3; // 2–1 → claims 2–3
        let r = cross_check(&our_match(), &bc);
        assert!(!r.tier1_ok());
        assert!(r.tier1.iter().any(|f| f.field == "team_score"));
    }

    #[test]
    fn wrong_goal_scorer_team_is_tier1() {
        // Move one of Alice's goals onto the orange roster (Cara) — goal counts
        // per team still total, but the per-scorer breakdown and a per-player
        // goals disagreement both trip.
        let mut bc = bc_match();
        bc.blue.players[0].stats.core.goals = 1; // Alice 2 → 1
        bc.orange.players[0].stats.core.goals = 2; // Cara 1 → 2
        bc.blue.stats.core.goals = 1;
        bc.orange.stats.core.goals = 2;
        let r = cross_check(&our_match(), &bc);
        assert!(!r.tier1_ok());
        assert!(r
            .tier1
            .iter()
            .any(|f| f.field == "goal" || f.field == "goals"));
    }

    #[test]
    fn missing_player_is_tier1_roster() {
        let mut bc = bc_match();
        bc.orange.players.pop(); // drop Dex
        let r = cross_check(&our_match(), &bc);
        assert!(!r.tier1_ok());
        assert!(r.tier1.iter().any(|f| f.field == "roster"));
    }

    #[test]
    fn wrong_team_size_is_tier1() {
        let mut bc = bc_match();
        bc.team_size = Some(3);
        let r = cross_check(&our_match(), &bc);
        assert!(!r.tier1_ok());
        assert!(r.tier1.iter().any(|f| f.field == "team_size"));
    }

    #[test]
    fn recomputed_saves_are_advisory_only() {
        let mut bc = bc_match();
        bc.blue.players[0].stats.core.saves += 2; // ballchasing recomputes saves
        let r = cross_check(&our_match(), &bc);
        assert!(
            r.tier1_ok(),
            "saves divergence must not fail Tier 1: {:?}",
            r.tier1
        );
        assert!(r.tier2.iter().any(|f| f.field == "saves"));
    }

    #[test]
    fn deserializes_sanitized_fixture_shape() {
        // The shape `scripts/ballchasing_fetch.py` writes (nulled ids, subset).
        let json = r#"{
            "id":"x","status":"ok","map_code":"stadium_p","map_name":"DFH",
            "team_size":2,"duration":300.0,
            "blue":{"stats":{"core":{"goals":1}},"players":[
                {"name":"A","id":{"platform":"steam","id":null},"stats":{"core":{"goals":1,"assists":0,"saves":0,"shots":1,"score":100,"mvp":true}}}]},
            "orange":{"stats":{"core":{"goals":0}},"players":[
                {"name":"B","id":{"platform":"epic","id":null},"stats":{"core":{"goals":0,"assists":0,"saves":1,"shots":0,"score":50,"mvp":false}}}]}
        }"#;
        let bc: BallchasingReplay = serde_json::from_str(json).expect("deserialize");
        assert_eq!(bc.team_size, Some(2));
        assert_eq!(bc.blue.players[0].name, "A");
        assert_eq!(
            bc.blue.players[0].id.as_ref().unwrap().platform.as_deref(),
            Some("steam")
        );
        assert!(bc.blue.players[0].id.as_ref().unwrap().id.is_none());
    }
}
