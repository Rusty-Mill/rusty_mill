//! What RLBotCS's `BridgeHandler` decides on every message from the game, minus the I/O.
//!
//! Evidence: `RLBotCS/Server/BridgeHandler.cs`, `HandleServer` (lines 54-189) and
//! `ConsiderDistributingPacket` (229-267). The game sends more messages than it renders
//! frames (it is asked for 240 per second against 120 physics ticks), so most of the rules
//! are about telling a message that advanced game time from one that did not.

use rb_rlbot_wire::{GamePacket, MatchConfiguration, MatchPhase, PlayerClass};

/// Time must move by more than this many seconds to count as a new tick
/// (`BridgeHandler.cs:91`).
pub const TIME_ADVANCE_EPSILON: f32 = 0.001;

/// How many no-progress messages are tolerated in a paused-like phase before one is
/// distributed anyway (`MAX_TICK_SKIP`, `BridgeHandler.cs:24`).
pub const MAX_TICK_SKIP: u32 = 1;

/// Who is in the match, as far as distribution is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Roster {
    /// Every player in the configuration.
    pub players: usize,
    /// The players that are not human.
    pub bots: usize,
}

impl Roster {
    pub fn from_configuration(configuration: &MatchConfiguration) -> Roster {
        let configured = &configuration.player_configurations;
        Roster {
            players: configured.len(),
            bots: configured
                .iter()
                .filter(|p| !matches!(p.variety, PlayerClass::Human))
                .count(),
        }
    }
}

/// One message from the game, already folded into a complete snapshot by whatever implements
/// the wire (see `GameLink`).
#[derive(Debug, Clone, PartialEq)]
pub struct Clump {
    pub packet: GamePacket,
    /// The message announced a freshly loaded map (`MessageHandler.ReceivedMatchInfo`).
    pub map_just_loaded: bool,
}

/// The counters `BridgeContext` keeps between messages.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TickTracker {
    seconds_elapsed: f32,
    ticks_skipped: u32,
    ticks_since_map_load: u32,
}

impl TickTracker {
    /// Folds in the time of the next message and says whether time advanced.
    pub fn observe(&mut self, seconds_elapsed: f32) -> bool {
        let advanced = seconds_elapsed - self.seconds_elapsed > TIME_ADVANCE_EPSILON;
        self.seconds_elapsed = seconds_elapsed;
        self.ticks_skipped = if advanced { 0 } else { self.ticks_skipped + 1 };
        self.ticks_since_map_load += 1;
        advanced
    }

    pub fn map_loaded(&mut self) {
        self.ticks_since_map_load = 0;
    }

    /// Messages since time last advanced.
    pub fn ticks_skipped(&self) -> u32 {
        self.ticks_skipped
    }

    /// Messages since the map loaded; queued spawn commands are only released once this is
    /// at least 2 (`BridgeHandler.cs:177`).
    pub fn ticks_since_map_load(&self) -> u32 {
        self.ticks_since_map_load
    }
}

/// Whether a message's snapshot goes out to the bots (`ConsiderDistributingPacket`).
///
/// Nothing goes out without a roster (no match configured). A message that advanced time goes
/// out; one that did not goes out only in `Replay`, `Paused`, `Ended` or `Inactive`, and only
/// after more than [`MAX_TICK_SKIP`] such messages, so a paused game still updates its bots a
/// little. Either way the snapshot must hold every player that is required: all of them in an
/// inactive or ended match, otherwise just the bots (humans may leave), assuming bots take the
/// lower car indices as RLBotCS does.
pub fn should_distribute(
    roster: Option<Roster>,
    packet: &GamePacket,
    time_advanced: bool,
    ticks_skipped: u32,
) -> bool {
    let Some(roster) = roster else {
        return false;
    };
    let phase = packet.match_info.match_phase;
    let slow_phase = matches!(
        phase,
        MatchPhase::Replay | MatchPhase::Paused | MatchPhase::Ended | MatchPhase::Inactive
    );
    let due = ticks_skipped > MAX_TICK_SKIP && slow_phase;
    if !time_advanced && !due {
        return false;
    }
    let inactive = matches!(phase, MatchPhase::Inactive | MatchPhase::Ended);
    let required = if inactive {
        roster.players
    } else {
        roster.bots
    };
    packet.players.len() >= required
}

#[cfg(test)]
mod tests {
    use super::*;
    use rb_rlbot_wire::{AirState, MatchInfo, Physics, PlayerInfo};

    fn car() -> PlayerInfo {
        PlayerInfo {
            physics: Physics::default(),
            air_state: AirState::OnGround,
            demolished_timeout: 0.0,
            is_bot: true,
            name: String::new(),
            team: 0,
            boost: 0.0,
            player_id: 0,
        }
    }

    fn packet(phase: MatchPhase, cars: usize) -> GamePacket {
        GamePacket {
            players: vec![car(); cars],
            boost_pads: Vec::new(),
            balls: Vec::new(),
            match_info: MatchInfo {
                seconds_elapsed: 0.0,
                game_time_remaining: 0.0,
                is_overtime: false,
                is_unlimited_time: false,
                match_phase: phase,
                frame_num: 0,
            },
            teams: Vec::new(),
        }
    }

    const ROSTER: Option<Roster> = Some(Roster {
        players: 3,
        bots: 2,
    });

    #[test]
    fn time_must_advance_by_more_than_a_millisecond() {
        let mut t = TickTracker::default();
        assert!(t.observe(0.1));
        assert!(!t.observe(0.1005), "half a millisecond is not a new tick");
        assert_eq!(t.ticks_skipped(), 1);
        assert!(!t.observe(0.1005));
        assert_eq!(t.ticks_skipped(), 2);
        assert!(t.observe(0.2));
        assert_eq!(t.ticks_skipped(), 0, "advancing resets the skip count");
        assert!(!t.observe(0.1), "time going backwards is not an advance");
    }

    #[test]
    fn map_load_restarts_its_own_counter_only() {
        let mut t = TickTracker::default();
        t.observe(1.0);
        t.observe(2.0);
        assert_eq!(t.ticks_since_map_load(), 2);
        t.map_loaded();
        assert_eq!(t.ticks_since_map_load(), 0);
        t.observe(3.0);
        assert_eq!(t.ticks_since_map_load(), 1);
    }

    #[test]
    fn nothing_goes_out_without_a_match_configuration() {
        assert!(!should_distribute(
            None,
            &packet(MatchPhase::Active, 3),
            true,
            0
        ));
    }

    #[test]
    fn an_advancing_active_packet_with_all_bots_goes_out() {
        assert!(should_distribute(
            ROSTER,
            &packet(MatchPhase::Active, 2),
            true,
            0
        ));
    }

    #[test]
    fn a_packet_missing_a_bot_is_held_back_but_a_missing_human_is_fine_mid_match() {
        assert!(!should_distribute(
            ROSTER,
            &packet(MatchPhase::Active, 1),
            true,
            0
        ));
        // Two cars are the two bots; the human (index 2) may have left.
        assert!(should_distribute(
            ROSTER,
            &packet(MatchPhase::Active, 2),
            true,
            0
        ));
    }

    #[test]
    fn an_inactive_or_ended_match_needs_every_player() {
        for phase in [MatchPhase::Inactive, MatchPhase::Ended] {
            assert!(!should_distribute(ROSTER, &packet(phase, 2), true, 0));
            assert!(should_distribute(ROSTER, &packet(phase, 3), true, 0));
        }
    }

    #[test]
    fn without_progress_only_slow_phases_distribute_and_only_after_the_skip_budget() {
        let slow = packet(MatchPhase::Replay, 3);
        assert!(!should_distribute(ROSTER, &slow, false, 0));
        assert!(!should_distribute(ROSTER, &slow, false, MAX_TICK_SKIP));
        assert!(should_distribute(ROSTER, &slow, false, MAX_TICK_SKIP + 1));
        // A live phase never distributes a message that did not advance time.
        let live = packet(MatchPhase::Active, 3);
        assert!(!should_distribute(ROSTER, &live, false, 99));
    }

    #[test]
    fn the_roster_counts_non_humans_as_bots() {
        use rb_rlbot_wire::PlayerConfiguration;
        let human = PlayerConfiguration {
            variety: PlayerClass::Human,
            team: 0,
            player_id: 0,
        };
        let psyonix = PlayerConfiguration {
            variety: PlayerClass::PsyonixBot(rb_rlbot_wire::PsyonixBot {
                name: "p".into(),
                skill: rb_rlbot_wire::PsyonixSkill::Pro,
            }),
            team: 1,
            player_id: 1,
        };
        let cfg = MatchConfiguration {
            player_configurations: vec![human, psyonix],
            ..MatchConfiguration::default()
        };
        assert_eq!(
            Roster::from_configuration(&cfg),
            Roster {
                players: 2,
                bots: 1
            }
        );
    }
}
