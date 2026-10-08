//! Lobby-completeness gate.
//!
//! A player who disconnects mid-match, or who stays connected but goes AFK for
//! a real stretch, doesn't just make *their own* report meaningless — it makes
//! their teammate's support/rotation metrics meaningless too, and turns the
//! opposing team's stretch of the game into an unrepresentative man-advantage
//! (§11: "a missing/AFK teammate makes support metrics meaningless" already
//! motivated the low-confidence gate, but nothing previously checked for it
//! directly). This module checks every track in the match, not just the
//! scoring target, so the whole lobby's confidence degrades together.
//!
//! Two independent signals, either one sufficient to fail a track:
//! - **Coverage** — does the track's sample span cover most of the match?
//!   Catches a disconnect/leave (the track simply stops).
//! - **Idle** — within its span, is there a long stretch of near-zero speed
//!   outside kickoff/goal dead time? Catches AFK-while-still-connected, which
//!   coverage alone cannot see.

use replay_analyzer::model::{Event, PlayerTrack};
use serde::{Deserialize, Serialize};

/// Thresholds for [`lobby_fully_present`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CoverageConfig {
    /// A track must span at least this fraction of the match duration (first
    /// sample to last) to count as present for the game.
    pub min_span_frac: f32,
    /// Speed (uu/s) below which a car counts as idle. Comfortably under any
    /// real driving/steering speed, but above position-noise jitter.
    pub idle_speed_uu_s: f32,
    /// A continuous idle run longer than this (s), outside dead time, flags
    /// the track even though it never disconnected.
    pub max_idle_run_s: f32,
    /// Seconds of grace around a `Kickoff`/after a `Goal` event during which
    /// near-zero speed is expected (countdown / goal-reset freeze) and never
    /// counts toward an idle run.
    pub dead_time_pad_s: f32,
}

impl Default for CoverageConfig {
    fn default() -> Self {
        CoverageConfig {
            min_span_frac: 0.90,
            idle_speed_uu_s: 60.0,
            max_idle_run_s: 20.0,
            dead_time_pad_s: 5.0,
        }
    }
}

fn speed(v: replay_analyzer::model::Vec3) -> f32 {
    (v.x * v.x + v.y * v.y + v.z * v.z).sqrt()
}

/// Windows of legitimate "everyone is briefly stationary" dead time.
fn dead_time_windows(events: &[Event], pad_s: f32) -> Vec<(f32, f32)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Kickoff { t } => Some((*t - pad_s, *t + pad_s)),
            Event::Goal { t, .. } => Some((*t, *t + pad_s)),
            _ => None,
        })
        .collect()
}

fn in_dead_time(t: f32, windows: &[(f32, f32)]) -> bool {
    windows.iter().any(|(a, b)| t >= *a && t <= *b)
}

/// Longest continuous run (s) of sub-`idle_speed_uu_s` speed in `track`,
/// skipping samples inside dead time (they neither extend nor break a run).
fn longest_idle_run_s(track: &PlayerTrack, cfg: &CoverageConfig, dead: &[(f32, f32)]) -> f32 {
    let mut longest = 0.0f32;
    let mut run_start: Option<f32> = None;
    let mut last_t = None;
    for s in &track.samples {
        if in_dead_time(s.t, dead) {
            continue;
        }
        if speed(s.v) < cfg.idle_speed_uu_s {
            run_start.get_or_insert(s.t);
        } else if let Some(start) = run_start.take() {
            longest = longest.max(last_t.unwrap_or(start) - start);
        }
        last_t = Some(s.t);
    }
    if let Some(start) = run_start {
        longest = longest.max(last_t.unwrap_or(start) - start);
    }
    longest
}

/// Whether every track was present (span) and actually playing (no long idle
/// run) for the whole match. `true` for an empty/zero-duration match (nothing
/// to gate on).
pub fn lobby_fully_present(
    tracks: &[PlayerTrack],
    events: &[Event],
    duration_s: f32,
    cfg: &CoverageConfig,
) -> bool {
    if duration_s <= 0.0 || tracks.is_empty() {
        return true;
    }
    let dead = dead_time_windows(events, cfg.dead_time_pad_s);
    tracks.iter().all(|tr| {
        let span = match (tr.samples.first(), tr.samples.last()) {
            (Some(a), Some(b)) => b.t - a.t,
            _ => 0.0,
        };
        span / duration_s >= cfg.min_span_frac
            && longest_idle_run_s(tr, cfg, &dead) <= cfg.max_idle_run_s
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_analyzer::model::{TrackGap, Vec3};

    fn sample(t: f32, speed_uu_s: f32) -> replay_analyzer::model::TrackSample {
        replay_analyzer::model::TrackSample {
            t,
            actor_id: 0,
            p: Vec3 {
                x: 0.0,
                y: 0.0,
                z: 17.0,
            },
            v: Vec3 {
                x: speed_uu_s,
                y: 0.0,
                z: 0.0,
            },
            boost: Some(50),
            rot: None,
        }
    }

    fn track(
        player: &str,
        pri: i32,
        samples: Vec<replay_analyzer::model::TrackSample>,
    ) -> PlayerTrack {
        PlayerTrack {
            player: player.to_string(),
            pri,
            team: Some(0),
            num_segments: 1,
            samples,
            gaps: Vec::<TrackGap>::new(),
        }
    }

    /// Four tracks, each driving (500 uu/s) at 1-second steps across the whole
    /// match — the clean baseline every other test perturbs.
    fn full_lobby(duration: f32) -> Vec<PlayerTrack> {
        let n = duration as i32;
        let samples = |pri: i32| -> Vec<_> {
            (0..=n)
                .map(|i| sample(i as f32, 500.0 + pri as f32))
                .collect()
        };
        vec![
            track("a", 1, samples(1)),
            track("b", 2, samples(2)),
            track("c", 3, samples(3)),
            track("d", 4, samples(4)),
        ]
    }

    #[test]
    fn full_game_all_present_passes() {
        let tracks = full_lobby(300.0);
        assert!(lobby_fully_present(
            &tracks,
            &[],
            300.0,
            &CoverageConfig::default()
        ));
    }

    #[test]
    fn early_disconnect_fails_the_whole_lobby() {
        let mut tracks = full_lobby(300.0);
        // One player's track stops at t=60 of a 300s match (span 20%).
        tracks[3].samples.retain(|s| s.t <= 60.0);
        let cfg = CoverageConfig::default();
        assert!(!lobby_fully_present(&tracks, &[], 300.0, &cfg));
    }

    #[test]
    fn brief_stop_is_not_flagged() {
        let mut tracks = full_lobby(300.0);
        // A few seconds parked (e.g. a save-ready stance) is not AFK.
        for s in tracks[0]
            .samples
            .iter_mut()
            .filter(|s| (100.0..105.0).contains(&s.t))
        {
            s.v = Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            };
        }
        assert!(lobby_fully_present(
            &tracks,
            &[],
            300.0,
            &CoverageConfig::default()
        ));
    }

    #[test]
    fn sustained_mid_game_afk_fails_even_though_still_connected() {
        let mut tracks = full_lobby(300.0);
        // Present the whole match (span 100%) but frozen for 30s mid-game.
        for s in tracks[1]
            .samples
            .iter_mut()
            .filter(|s| (150.0..180.0).contains(&s.t))
        {
            s.v = Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            };
        }
        let cfg = CoverageConfig::default();
        let span_frac = {
            let tr = &tracks[1];
            (tr.samples.last().unwrap().t - tr.samples.first().unwrap().t) / 300.0
        };
        assert!(span_frac >= cfg.min_span_frac, "still spans the match");
        assert!(!lobby_fully_present(&tracks, &[], 300.0, &cfg));
    }

    #[test]
    fn goal_reset_freeze_is_not_flagged() {
        let mut tracks = full_lobby(300.0);
        // Everyone stops for a goal-reset freeze at t=100; well within dead time.
        for tr in tracks.iter_mut() {
            for s in tr
                .samples
                .iter_mut()
                .filter(|s| (99.0..103.0).contains(&s.t))
            {
                s.v = Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                };
            }
        }
        let events = vec![Event::Goal {
            t: 100.0,
            scorer: None,
            team: None,
        }];
        assert!(lobby_fully_present(
            &tracks,
            &events,
            300.0,
            &CoverageConfig::default()
        ));
    }
}
