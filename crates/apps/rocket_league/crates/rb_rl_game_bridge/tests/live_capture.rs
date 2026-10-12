//! What a real Rocket League sent, recorded 2026-10-11 with `rb_rl_game_probe --record` while
//! Freeplay loaded on TrainStation_P (`tests/fixtures/live_freeplay_slice.bin`: 250 messages from
//! just before the map loaded to a ball touch; the player's name replaced by a same-length
//! placeholder, which is a valid in-place edit of the buffer).
//!
//! The first test that did not need the game to be mocked. What it pins is what the live traffic
//! showed, not what the format note predicted.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rb_physics_bullet::BoostPads;
use rb_rl_game_bridge::{
    decode_controller_packet, decode_game_packet, encode_controller_packet, BotSkill, Command,
    GameEvent, GameFolder, Loadout,
};
use rb_rlbot_wire::frame::FrameDecoder;
use rb_rlbot_wire::MatchPhase;

const SLICE: &[u8] = include_bytes!("fixtures/live_freeplay_slice.bin");

fn messages() -> Vec<rb_rl_game_bridge::GameBundle> {
    let mut decoder = FrameDecoder::new();
    decoder.push(SLICE);
    let mut out = Vec::new();
    while let Some(payload) = decoder.next_frame() {
        out.push(decode_game_packet(&payload).unwrap());
    }
    out
}

fn events(bundles: &[rb_rl_game_bridge::GameBundle]) -> impl Iterator<Item = &GameEvent> {
    bundles.iter().flat_map(|b| b.events.iter())
}

#[test]
fn every_message_the_real_game_sent_decodes() {
    let all = messages();
    assert_eq!(all.len(), 250);
    // The two idle menu messages, then the map loads in one big message.
    assert!(all[0].events.is_empty() && all[1].events.is_empty());
    assert!(matches!(all[2].events[0], GameEvent::GameSpawn { .. }));
}

#[test]
fn the_map_announcement_says_what_the_game_is() {
    let all = messages();
    let GameEvent::GameSpawn {
        map_name,
        mode,
        team_size,
        gravity,
    } = &all[2].events[0]
    else {
        panic!("{:?}", all[2].events[0]);
    };
    assert_eq!(map_name, "TrainStation_P");
    assert_eq!((*mode, *team_size), (1, 1), "Soccar, 1v1 Freeplay");
    assert_eq!(gravity.map(|g| g.z), Some(-650.0));
}

#[test]
fn a_human_car_announces_itself_with_command_id_zero_and_a_bot_would_not() {
    let all = messages();
    let spawn = events(&all)
        .find_map(|e| match e {
            GameEvent::PlayerSpawn {
                command_id, team, ..
            } => Some((*command_id, *team)),
            _ => None,
        })
        .unwrap();
    // The format note said a human's spawn carries command id 0; here it is, from the game.
    assert_eq!(spawn, (Some(0), 0));
}

#[test]
fn the_live_pad_layout_is_the_simulations() {
    let all = messages();
    let mut live: Vec<(f32, f32, bool)> = events(&all)
        .filter_map(|e| match e {
            GameEvent::PickupSpawn { location, big, .. } => Some((location.x, location.y, *big)),
            _ => None,
        })
        .collect();
    live.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.total_cmp(&b.0)));
    let pads = BoostPads::standard();
    let sim: Vec<(f32, f32, bool)> = (0..pads.len())
        .filter_map(|i| pads.pad(i))
        .map(|p| (p.position.x, p.position.y, p.big))
        .collect();
    assert_eq!(live.len(), 34);
    assert_eq!(live.iter().filter(|p| p.2).count(), 6, "six big pads");
    for (l, s) in live.iter().zip(&sim) {
        assert!(
            (l.0 - s.0).abs() < 1.0 && (l.1 - s.1).abs() < 1.0 && l.2 == s.2,
            "live pad {l:?} vs simulation {s:?}"
        );
    }
}

#[test]
fn both_goals_are_announced_as_the_game_measures_them() {
    let all = messages();
    let goals: Vec<_> = events(&all)
        .filter_map(|e| match e {
            GameEvent::GoalSpawn {
                center,
                direction,
                width,
                height,
                ..
            } => Some((center.y, direction.y.round(), *width, *height)),
            _ => None,
        })
        .collect();
    assert_eq!(goals.len(), 2);
    // At the two back walls, each facing the field; about 1920 wide and 752 tall.
    assert_eq!((goals[0].0, goals[0].1), (-5120.0, 1.0));
    assert_eq!((goals[1].0, goals[1].1), (5120.0, -1.0));
    for g in &goals {
        assert!(
            (g.2 - 1920.0).abs() < 0.1 && (g.3 - 752.0).abs() < 0.1,
            "{g:?}"
        );
    }
}

#[test]
fn the_match_goes_kickoff_then_active_and_the_cars_move_sensibly() {
    let mut folder = GameFolder::new();
    let mut phases: Vec<MatchPhase> = Vec::new();
    let (mut min_ball_z, mut frames) = (f32::MAX, 0u32);
    for bundle in messages() {
        folder.apply(&bundle);
        let p = folder.packet();
        if phases.last() != Some(&p.match_info.match_phase) {
            phases.push(p.match_info.match_phase);
        }
        if let Some(ball) = p.balls.first() {
            min_ball_z = min_ball_z.min(ball.physics.location.z);
        }
        frames = p.match_info.frame_num;
    }
    assert_eq!(
        phases,
        [
            MatchPhase::Inactive,
            MatchPhase::Kickoff,
            MatchPhase::Active
        ]
    );
    // The ball never goes through the floor (its radius is about 91), and the 250 messages span a
    // little over four seconds of 120 Hz physics frames.
    assert!(min_ball_z > 85.0, "{min_ball_z}");
    assert!((500..=520).contains(&frames), "{frames} frames");
}

#[test]
fn freeplays_scoreboard_clock_counts_up_so_an_untimed_match_reports_none() {
    // Live: the clock read 1, 2, 3, ... 163 over the session. As a time remaining it would be
    // wrong, so a match the configuration calls untimed ignores it.
    let all = messages();
    let seconds = events(&all)
        .find_map(|e| match e {
            GameEvent::TimeChange { display_seconds } => Some(*display_seconds),
            _ => None,
        })
        .unwrap();
    assert!(
        seconds <= 2,
        "the clock started near zero, not at a limit: {seconds}"
    );

    let mut timed = GameFolder::new();
    let mut untimed = GameFolder::new();
    untimed.set_unlimited_time(true);
    for bundle in &all {
        timed.apply(bundle);
        untimed.apply(bundle);
    }
    assert!(!timed.packet().match_info.is_unlimited_time);
    let p = untimed.packet();
    assert!(p.match_info.is_unlimited_time);
    assert_eq!(p.match_info.game_time_remaining, 0.0);
}

#[test]
fn unmodelled_events_come_through_as_other_with_the_documented_tags() {
    let all = messages();
    let mut tags: Vec<u8> = events(&all)
        .filter_map(|e| match e {
            GameEvent::Other(t) => Some(*t),
            _ => None,
        })
        .collect();
    tags.sort_unstable();
    tags.dedup();
    // BallHit (12), PlayerInputChange (18) and BallExtraDataChange (21).
    assert_eq!(tags, [12, 18, 21]);
}

// ---- the second live run: the bridge spawned a bot and drove it ----

const BOT_SLICE: &[u8] = include_bytes!("fixtures/live_bot_spawn_slice.bin");
const ACCEPTED: &[u8] = include_bytes!("fixtures/live_spawnbot_accepted.bin");

fn bundles(bytes: &[u8]) -> Vec<rb_rl_game_bridge::GameBundle> {
    let mut decoder = FrameDecoder::new();
    decoder.push(bytes);
    let mut out = Vec::new();
    while let Some(payload) = decoder.next_frame() {
        out.push(decode_game_packet(&payload).unwrap());
    }
    out
}

fn payloads(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut decoder = FrameDecoder::new();
    decoder.push(bytes);
    std::iter::from_fn(|| decoder.next_frame()).collect()
}

#[test]
fn the_game_answers_a_spawnbot_with_a_playerspawn_carrying_its_command_id() {
    let all = bundles(BOT_SLICE);
    let spawns: Vec<_> = events(&all)
        .filter_map(|e| match e {
            GameEvent::PlayerSpawn {
                id,
                command_id,
                name,
                team,
            } => Some((*id, *command_id, name.clone(), *team)),
            _ => None,
        })
        .collect();
    // The human first (command id 0), then the bot, answering the SpawnBot that was command 1.
    assert_eq!(
        spawns,
        [
            (38, Some(0), "Freeplay1".to_string(), 0),
            (39, Some(1), "rusty_bullet/chaser_bot".to_string(), 0),
        ]
    );
}

#[test]
fn the_fold_ties_the_bots_car_to_its_index_and_moves_the_human_along() {
    let all = bundles(BOT_SLICE);
    let mut folder = GameFolder::new();
    // The human is in the opening message; the bridge sends its spawn two messages later.
    for bundle in &all[..4] {
        folder.apply(bundle);
    }
    assert_eq!(
        folder.actor_for_player(0),
        Some(38),
        "the human holds index 0 at first"
    );
    folder.expect_spawn(1, 0);
    for bundle in &all[4..] {
        folder.apply(bundle);
    }
    assert_eq!(
        folder.actor_for_player(0),
        Some(39),
        "the bot has the index it was spawned for"
    );
    assert!(folder.is_claimed(0));
    assert_eq!(
        folder.actor_for_player(1),
        Some(38),
        "the human moved along, not lost"
    );
    assert!(!folder.is_claimed(1));
    let names: Vec<_> = folder
        .packet()
        .players
        .into_iter()
        .map(|p| p.name)
        .collect();
    assert_eq!(names, ["rusty_bullet/chaser_bot", "Freeplay1"]);
}

#[test]
fn our_spawnbot_is_byte_for_byte_what_the_game_accepted() {
    let sent = payloads(ACCEPTED);
    assert_eq!(sent.len(), 2);
    let ours = encode_controller_packet(
        1,
        &[Command::SpawnBot {
            name: "rusty_bullet/chaser_bot".into(),
            skill: BotSkill::Custom,
            team: 0,
            loadout: Loadout::standard(0),
        }],
    )
    .unwrap();
    assert_eq!(
        ours, sent[0],
        "the encoder drifted from the bytes the real game took"
    );

    // The input that followed went to the bot's car, the actor the game announced.
    let commands = decode_controller_packet(&sent[1]).unwrap();
    let (id, Command::PlayerInput { actor_id, input }) = &commands[0] else {
        panic!("{commands:?}");
    };
    assert_eq!((*id, *actor_id), (2, 39));
    assert!(input.boost && input.throttle == 1.0);
}
