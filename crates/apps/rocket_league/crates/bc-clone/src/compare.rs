//! Team aggregation + the **exact-match validator**: diff our ballchasing-shaped
//! document against a real `GET /replays/{id}` document, field by field.
//!
//! Both documents share the nesting `{blue,orange} → players[] → stats →
//! {core,boost,movement,positioning,demo} → field`. The differ pairs rosters by
//! (side, name) — tolerant of ballchasing's accent-stripping, via the workspace
//! `roster_match` — then, for every numeric field we emit, records `(ours,
//! theirs)` and reports the per-field agreement. A gate over a **core** set
//! (the authoritative + well-defined channels) turns it into a regression oracle.

use crate::{Player, Stats, UNIMPLEMENTED};
use replay_analyzer::analyze::roster_match::{team_anchored_pairs, RosterSlot};
use serde_json::{Map, Value};

/// Integer fields that must match **exactly** (any difference is a failure).
const EXACT: &[&str] = &[
    "goals",
    "assists",
    "saves",
    "shots",
    "score",
    "goals_against",
    "shots_against",
    "count_collected_big",
    "count_collected_small",
    "count_stolen_big",
    "count_stolen_small",
    "inflicted",
    "taken",
    "goals_against_while_last_defender",
];

/// Continuous fields the gate holds to a tight relative-error bound — the
/// authoritative pad model, speed buckets, thirds/halves, distances.
const CORE: &[&str] = &[
    "amount_collected",
    "amount_stolen",
    "amount_overfill",
    "bpm",
    "avg_amount",
    "percent_zero_boost",
    "percent_full_boost",
    "avg_speed",
    "total_distance",
    "percent_slow_speed",
    "percent_boost_speed",
    "percent_supersonic_speed",
    "percent_ground",
    "avg_distance_to_ball",
    "avg_distance_to_mates",
    "percent_defensive_third",
    "percent_neutral_third",
    "percent_offensive_third",
    "percent_defensive_half",
    "percent_offensive_half",
    "percent_behind_ball",
    "percent_infront_ball",
];

/// Relative-error bound for the `CORE` continuous channels.
const CORE_REL_TOL: f32 = 0.12;

/// True if a field is averaged (not summed) when aggregating a team from its
/// players — rates, percentages, and the replicated per-side `*_against` values.
fn is_mean(field: &str) -> bool {
    field.starts_with("percent_")
        || field.starts_with("avg_")
        || matches!(
            field,
            "bpm" | "bcpm" | "shots_against" | "goals_against" | "shooting_percentage"
        )
}

/// Aggregate a side's players into a team [`Stats`] block: extensive fields
/// (counts, amounts, times, distance, goals…) sum; intensive fields (rates,
/// percents, the `*_against` values) average; `mvp` is the side's OR.
pub fn aggregate_side(players: &[Player]) -> Stats {
    if players.is_empty() {
        return Stats::default();
    }
    let objs: Vec<Value> = players
        .iter()
        .map(|p| serde_json::to_value(&p.stats).expect("stats serialize"))
        .collect();
    // Each player's stats is an object of {group: {field: value}}; aggregate
    // group-by-group, field-by-field.
    let groups = objs[0].as_object().expect("stats object").clone();
    let mut out = Map::new();
    for (group, _) in groups {
        let per_group: Vec<&Map<String, Value>> = objs
            .iter()
            .map(|o| o[&group].as_object().expect("group object"))
            .collect();
        out.insert(group, Value::Object(aggregate_obj(&per_group)));
    }
    serde_json::from_value(Value::Object(out)).expect("aggregated stats")
}

/// Aggregate one group (a flat `{field: number|bool}` map) across players.
fn aggregate_obj(objs: &[&Map<String, Value>]) -> Map<String, Value> {
    let mut out = Map::new();
    for (key, sample) in objs[0] {
        if sample.is_boolean() {
            let any = objs.iter().any(|o| o[key].as_bool().unwrap_or(false));
            out.insert(key.clone(), Value::Bool(any));
            continue;
        }
        let vals: Vec<f64> = objs
            .iter()
            .map(|o| o[key].as_f64().unwrap_or(0.0))
            .collect();
        let sum: f64 = vals.iter().sum();
        let raw = if is_mean(key) {
            sum / vals.len() as f64
        } else {
            sum
        };
        // Integer-typed fields (counts, goals, the replicated `*_against`) must
        // round-trip back into `i32`/`u32`, so keep a whole result an integer.
        let all_int = objs.iter().all(|o| o[key].is_i64() || o[key].is_u64());
        let value = if all_int && raw.fract() == 0.0 {
            Value::from(raw as i64)
        } else {
            Value::from(raw)
        };
        out.insert(key.clone(), value);
    }
    out
}

/// Per-(group, field) accumulator of `(ours, theirs)` pairs during a comparison.
struct Channel {
    group: String,
    field: String,
    pairs: Vec<(f32, f32)>,
}

/// One field's agreement across all paired players.
#[derive(Debug, Clone)]
pub struct FieldDiff {
    pub group: String,
    pub field: String,
    pub n: usize,
    pub median_rel_err: f32,
    pub max_abs_err: f32,
    pub exact: bool,
}

/// The full comparison outcome.
#[derive(Debug, Clone, Default)]
pub struct CompareReport {
    pub players_paired: usize,
    pub players_ours: usize,
    pub players_theirs: usize,
    pub diffs: Vec<FieldDiff>,
}

/// Relative error against `theirs`, floored so a near-zero reference doesn't
/// explode the ratio.
fn rel_err(ours: f32, theirs: f32) -> f32 {
    (ours - theirs).abs() / theirs.abs().max(1.0)
}

fn median(mut xs: Vec<f32>) -> f32 {
    if xs.is_empty() {
        return 0.0;
    }
    xs.sort_by(|a, b| a.total_cmp(b));
    xs[xs.len() / 2]
}

/// Pull `doc[side]["players"]` as a slice of JSON objects.
fn players<'a>(doc: &'a Value, side: &str) -> Vec<&'a Value> {
    doc.get(side)
        .and_then(|s| s.get("players"))
        .and_then(|p| p.as_array())
        .map(|a| a.iter().collect())
        .unwrap_or_default()
}

fn name_of(p: &Value) -> String {
    p.get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("")
        .to_string()
}

/// Compare our document against a real ballchasing document, field by field.
pub fn compare_documents(ours: &Value, theirs: &Value) -> CompareReport {
    // Collect (team, name, &player) for both docs across the two sides; the names
    // are owned locally so the borrowed `RosterSlot`s stay valid for the pairing.
    let (mut our_teams, mut our_names, mut our_players) = (Vec::new(), Vec::new(), Vec::new());
    for (team, side) in [(0, "blue"), (1, "orange")] {
        for p in players(ours, side) {
            our_teams.push(team);
            our_names.push(name_of(p));
            our_players.push(p);
        }
    }
    let (mut their_teams, mut their_names, mut their_players) =
        (Vec::new(), Vec::new(), Vec::new());
    for (team, side) in [(0, "blue"), (1, "orange")] {
        for p in players(theirs, side) {
            their_teams.push(team);
            their_names.push(name_of(p));
            their_players.push(p);
        }
    }
    let our_slots: Vec<RosterSlot> = our_names
        .iter()
        .zip(&our_teams)
        .map(|(n, t)| RosterSlot::new(Some(*t), n))
        .collect();
    let their_slots: Vec<RosterSlot> = their_names
        .iter()
        .zip(&their_teams)
        .map(|(n, t)| RosterSlot::new(Some(*t), n))
        .collect();

    // Accumulate the (ours, theirs) pairs per (group, field) over every paired
    // player.
    let mut acc: Vec<Channel> = Vec::new();
    let mut paired = 0usize;
    for (oi, ti) in team_anchored_pairs(&our_slots, &their_slots) {
        paired += 1;
        let (op, tp) = (our_players[oi], their_players[ti]);
        let (Some(os), Some(ts)) = (op.get("stats"), tp.get("stats")) else {
            continue;
        };
        let (Some(og), Some(tg)) = (os.as_object(), ts.as_object()) else {
            continue;
        };
        for (group, gv) in og {
            let Some(fields) = gv.as_object() else {
                continue;
            };
            let their_group = tg.get(group).and_then(|g| g.as_object());
            for (field, ours_v) in fields {
                if UNIMPLEMENTED.contains(&field.as_str()) {
                    continue;
                }
                let (Some(o), Some(t)) = (
                    ours_v.as_f64(),
                    their_group
                        .and_then(|g| g.get(field))
                        .and_then(|v| v.as_f64()),
                ) else {
                    continue;
                };
                let i = match acc
                    .iter()
                    .position(|c| c.group == *group && c.field == *field)
                {
                    Some(i) => i,
                    None => {
                        acc.push(Channel {
                            group: group.clone(),
                            field: field.clone(),
                            pairs: Vec::new(),
                        });
                        acc.len() - 1
                    }
                };
                acc[i].pairs.push((o as f32, t as f32));
            }
        }
    }

    let diffs = acc
        .into_iter()
        .map(|c| {
            let rels: Vec<f32> = c.pairs.iter().map(|(o, t)| rel_err(*o, *t)).collect();
            let max_abs = c
                .pairs
                .iter()
                .map(|(o, t)| (o - t).abs())
                .fold(0.0f32, f32::max);
            FieldDiff {
                exact: EXACT.contains(&c.field.as_str()),
                n: c.pairs.len(),
                median_rel_err: median(rels),
                max_abs_err: max_abs,
                group: c.group,
                field: c.field,
            }
        })
        .collect();

    CompareReport {
        players_paired: paired,
        players_ours: our_players.len(),
        players_theirs: their_players.len(),
        diffs,
    }
}

impl CompareReport {
    /// Gate failures: every roster player must pair, every `EXACT` field must
    /// match exactly, and every `CORE` continuous field must agree within
    /// [`CORE_REL_TOL`].
    pub fn failures(&self) -> Vec<String> {
        let mut fails = Vec::new();
        if self.players_paired < self.players_ours.max(self.players_theirs) {
            fails.push(format!(
                "roster: paired {}/{} (ours {}, theirs {})",
                self.players_paired,
                self.players_ours.max(self.players_theirs),
                self.players_ours,
                self.players_theirs
            ));
        }
        for d in &self.diffs {
            if d.n == 0 {
                continue;
            }
            if d.exact && d.max_abs_err > 0.0 {
                fails.push(format!(
                    "{}.{}: exact field differs (max abs {})",
                    d.group, d.field, d.max_abs_err
                ));
            } else if CORE.contains(&d.field.as_str()) && d.median_rel_err > CORE_REL_TOL {
                fails.push(format!(
                    "core {}.{}: median rel err {:.3} > {:.2}",
                    d.group, d.field, d.median_rel_err, CORE_REL_TOL
                ));
            }
        }
        fails
    }
}
