//! Analyze layer: turn a neutral [`DecodedReplay`] into the [`CanonicalMatch`].
//!
//! This layer is decoder-agnostic — it depends only on the [`crate::decode`]
//! port types — and scoring-agnostic. It is the pure core that golden-file and
//! fixture tests exercise without ever touching a `.replay` file.

pub mod bcstats;
pub mod boost_pads;
pub mod coords;
pub mod events;
pub mod features;
pub mod identity;
pub mod normalize;
pub mod reconstruct;
pub mod resample;
pub mod roster_match;
pub mod validate;

use crate::decode::DecodedReplay;
use crate::model::{CanonicalMatch, Event, Resampled};

/// Default resample grid rate (Hz). Matches RL's nominal record rate.
pub const DEFAULT_HZ: f32 = 30.0;

/// This analyzer crate's version, pinned into the canonical model alongside the
/// decoder version for reproducibility.
pub const ANALYZER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Build the canonical match model from a decoded replay.
///
/// `replay_id` is supplied by the caller (typically the file stem or a content
/// hash) since the neutral decode carries no notion of where bytes came from.
pub fn build_canonical(decoded: &DecodedReplay, replay_id: impl Into<String>) -> CanonicalMatch {
    let recon = reconstruct::reconstruct(decoded);
    let duration_s = recon.frames.last().map(|f| f.t).unwrap_or(0.0);

    // T2: derive attack-direction signs, then resample to the fixed grid. Both
    // borrow `recon`, so compute before moving its fields into the model.
    let team_attack_sign = normalize::team_attack_sign(&recon.frames, &recon.tracks);
    let grid = resample::resample(&recon, DEFAULT_HZ);
    let resampled = Resampled {
        hz: DEFAULT_HZ,
        team_attack_sign,
        frames: grid,
    };

    // T4: derive events. Goals come from the authoritative header; resolve each
    // goal's frame index to a time via the native frame stream.
    let mut evs = events::touches(&resampled, &recon.tracks);
    let possessions = events::possessions(&evs);
    evs.extend(possessions);
    evs.extend(events::kickoffs(&resampled));
    evs.extend(events::demos(&recon.demos, &recon.tracks));
    evs.extend(events::stats(&recon.stat_events, &recon.tracks));
    for g in &decoded.meta.goals {
        // The header's goal frame can be one past the last index (a match-ending
        // goal); clamp to the final frame rather than collapsing to t=0.
        let t = if decoded.frames.is_empty() {
            0.0
        } else {
            let idx = (g.frame as usize).min(decoded.frames.len() - 1);
            decoded.frames[idx].time
        };
        evs.push(Event::Goal {
            t,
            scorer: g.scorer.clone(),
            team: g.team,
        });
    }
    evs.sort_by(|a, b| a.time().total_cmp(&b.time()));

    // T5: per-player feature aggregates over the resampled grid + events.
    let features = features::player_features(&resampled, &recon.tracks, &evs);

    // Per-player scoreboard: the header `PlayerStats[]` array is authoritative when
    // present, but some replays ship it empty. In that case synthesize the roster
    // from the coalesced tracks + the network `PRI_TA:Match*` counters (which carry
    // the scoreboard even when the header doesn't) so core stats aren't lost.
    let mut players: Vec<crate::model::PlayerMeta> = if decoded.meta.players.is_empty() {
        recon
            .tracks
            .iter()
            .map(|t| {
                let s = recon.pri_scores.get(&t.pri).copied().unwrap_or_default();
                crate::model::PlayerMeta {
                    name: t.player.clone(),
                    team: t.team.unwrap_or(0),
                    score: s.score,
                    goals: s.goals,
                    assists: s.assists,
                    saves: s.saves,
                    shots: s.shots,
                    // The header carried no roster, so there is no online id either.
                    platform_id: None,
                    car_id: None,
                    car_name: None,
                    camera: None,
                    steering_sensitivity: None,
                }
            })
            .collect()
    } else {
        decoded.meta.players.clone()
    };

    // Attach each player's car body from the loadout (`car_id` = the body the
    // player used = `blue_body` on team 0, `orange_body` on team 1), joined to the
    // scoreboard by name. Resolve the name via the bodies table.
    let car_by_name: std::collections::HashMap<&str, u32> = recon
        .tracks
        .iter()
        .filter_map(|t| {
            recon.pri_body.get(&t.pri).map(|(blue, orange)| {
                let body = if t.team == Some(1) { *orange } else { *blue };
                (t.player.as_str(), body)
            })
        })
        .collect();
    for p in &mut players {
        if let Some(&id) = car_by_name.get(p.name.as_str()) {
            p.car_id = Some(id);
            p.car_name = crate::cars::car_name(id).map(str::to_owned);
        }
    }

    // Attach each player's camera profile + steering sensitivity, joined by name
    // through the track's PRI. When the camera replicated but steering didn't,
    // default to RL's 1.0 (ballchasing does the same).
    let cam_by_name: std::collections::HashMap<&str, (Option<crate::model::Camera>, Option<f32>)> =
        recon
            .tracks
            .iter()
            .map(|t| {
                (
                    t.player.as_str(),
                    (
                        recon.pri_camera.get(&t.pri).copied(),
                        recon.pri_steer.get(&t.pri).copied(),
                    ),
                )
            })
            .collect();
    for p in &mut players {
        if let Some(&(cam, steer)) = cam_by_name.get(p.name.as_str()) {
            p.camera = cam;
            p.steering_sensitivity = steer.or(cam.map(|_| 1.0));
        }
    }

    CanonicalMatch {
        replay_id: replay_id.into(),
        parser_version: decoded.meta.parser_version.clone(),
        analyzer_version: ANALYZER_VERSION.to_string(),
        map: decoded.meta.map.clone(),
        team_size: decoded.meta.team_size,
        record_fps: decoded.meta.record_fps,
        num_frames: recon.frames.len(),
        duration_s,
        team_scores: decoded.meta.team_scores.clone(),
        players,
        tracks: recon.tracks,
        frames: recon.frames,
        resampled,
        events: evs,
        features,
        pickups: recon.pickups,
        powerslides: recon.powerslides,
    }
}
