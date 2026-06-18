//! Mechanical **skill detection** over the canonical match model.
//!
//! A pure consumer of [`replay_analyzer`]'s [`CanonicalMatch`], like the
//! `scoring` and `value` crates: no parsing, no I/O, no clock. It answers a
//! different question than scoring — scoring rates *decision discipline*; this
//! crate catalogs the *individual mechanics* a player executed, and lets a caller
//! **verify whether a given skill was performed** in the replay (optionally within
//! a player or a time window).
//!
//! Detection is heuristic and inferred from kinematics (spec §0): every instance
//! carries a confidence, and the thresholds live in a versioned [`SkillConfig`]
//! so results are reproducible and tunable.
//!
//! # Example
//! ```no_run
//! use replay_analyzer::{analyze::build_canonical, decode::{ReplayParser, boxcars_adapter::BoxcarsParser}};
//! use replay_skills::{detect_all, SkillConfig, Skill};
//! let data = std::fs::read("match.replay").unwrap();
//! let decoded = BoxcarsParser::new().parse(&data).unwrap();
//! let canonical = build_canonical(&decoded, "match");
//! let report = detect_all(&canonical, &SkillConfig::default());
//! assert!(report.performed(Skill::Aerial) || !report.performed(Skill::Aerial));
//! ```

pub mod calibrate;
pub mod config;
pub mod detect;
pub mod outcome;
pub mod profile;
pub mod report;
pub mod skill;
pub mod value_link;

use std::collections::{BTreeMap, BTreeSet};

use replay_analyzer::model::CanonicalMatch;

pub use config::{SkillConfig, SKILL_CONFIG_VERSION};
pub use outcome::{outcomes, SkillOutcome};
pub use profile::{profiles, PlayerSkillProfile, SkillStat};
pub use report::{PlayerSkills, SkillInstance, SkillReport};
pub use skill::{Detection, Skill, SkillCategory};
pub use value_link::{skill_values, SkillValue, TouchDv};

/// Detect every catalogued skill in a match, producing a [`SkillReport`].
///
/// Runs each detector in [`detect`], merges and time-sorts the instances, and
/// rolls them up per player. Deterministic for a given `(match, cfg)`.
pub fn detect_all(m: &CanonicalMatch, cfg: &SkillConfig) -> SkillReport {
    let r = &m.resampled;

    let mut instances = Vec::new();
    instances.extend(detect::supersonic(r, &m.tracks, cfg));
    instances.extend(detect::aerials(r, &m.events, cfg));
    instances.extend(detect::air_dribbles(r, &m.events, cfg));
    instances.extend(detect::double_touches(r, &m.events, cfg));
    instances.extend(detect::ceiling_plays(r, &m.tracks, cfg));
    instances.extend(detect::wall_plays(r, &m.events, cfg));
    instances.extend(detect::ground_dribbles(r, &m.tracks, cfg));
    instances.extend(detect::flicks(r, &m.events, cfg));
    instances.extend(detect::power_shots(r, &m.events, cfg));
    instances.extend(detect::redirects(r, &m.events, cfg));
    instances.extend(detect::kickoff_first_touches(r, &m.events, cfg));
    instances.extend(detect::boost_steals(&m.tracks, r, cfg));
    instances.extend(detect::demos(&m.events, &m.tracks));

    // Deterministic order: time, then skill, then player.
    instances.sort_by(|a, b| {
        a.t.total_cmp(&b.t)
            .then(a.skill.cmp(&b.skill))
            .then(a.pri.cmp(&b.pri))
    });

    let players = rollup(m, &instances);

    SkillReport {
        replay_id: m.replay_id.clone(),
        config_version: cfg.version.clone(),
        analyzer_version: m.analyzer_version.clone(),
        parser_version: m.parser_version.clone(),
        instances,
        players,
    }
}

/// Candidate-mode metrics: per skill, the pre-gate metric for *every* candidate
/// event, so the calibrator can fit detection floors — not just ramp tops — from
/// the full distribution (gated observations only ever sit above the floor).
///
/// Each candidate holds the detector's *context* gates (what makes the event an
/// attempt at that mechanic) and drops only the magnitude *floor* being fit. Touch
/// metrics: aerial → car height at every touch; flick → up-velocity of every carried
/// ball; power shot → speed of every goalward touch; redirect → turn angle of every
/// fast-in/out goalward touch. Run durations (every run, short ones included):
/// supersonic, ceiling play, ground dribble. Extend as detectors gain a `*_candidates`
/// twin.
pub fn candidate_metrics(m: &CanonicalMatch, cfg: &SkillConfig) -> BTreeMap<Skill, Vec<f32>> {
    let (r, ev) = (&m.resampled, &m.events);
    BTreeMap::from([
        (Skill::Aerial, detect::aerial_candidates(r, ev, cfg)),
        (Skill::Flick, detect::flick_candidates(r, ev, cfg)),
        (Skill::PowerShot, detect::power_shot_candidates(r, ev, cfg)),
        (Skill::Redirect, detect::redirect_candidates(r, ev, cfg)),
        (Skill::Supersonic, detect::supersonic_candidates(r, cfg)),
        (Skill::CeilingPlay, detect::ceiling_candidates(r, cfg)),
        (Skill::GroundDribble, detect::dribble_candidates(r, cfg)),
    ])
}

/// Per-player skill counts: one [`PlayerSkills`] per track (plus a synthetic
/// entry for any instance credited to a PRI without a track), team/PRI sorted.
fn rollup(m: &CanonicalMatch, instances: &[SkillInstance]) -> Vec<PlayerSkills> {
    let mut counts: BTreeMap<i32, BTreeMap<Skill, usize>> = BTreeMap::new();
    for i in instances {
        *counts.entry(i.pri).or_default().entry(i.skill).or_default() += 1;
    }

    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for t in &m.tracks {
        seen.insert(t.pri);
        out.push(PlayerSkills {
            pri: t.pri,
            player: t.player.clone(),
            team: t.team,
            counts: counts.get(&t.pri).cloned().unwrap_or_default(),
        });
    }
    // Credit any instance whose PRI never produced a coalesced track.
    for (pri, c) in &counts {
        if !seen.contains(pri) {
            out.push(PlayerSkills {
                pri: *pri,
                player: "<unknown>".to_string(),
                team: None,
                counts: c.clone(),
            });
        }
    }

    out.sort_by(|a, b| a.team.cmp(&b.team).then(a.pri.cmp(&b.pri)));
    out
}
