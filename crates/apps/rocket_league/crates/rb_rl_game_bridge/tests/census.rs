//! The tag census: it counts what is in a recording and tells a tag the decoders only treat as
//! `Other` apart from one that is not in the format at all.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rb_rl_game_bridge::{
    encode_controller_packet, encode_game_packet, Census, Command, GameBundle, GameEvent,
    CONTROLLER_COMMANDS, GAME_MESSAGES,
};
use rb_rlbot_wire::frame::FrameDecoder;
use rusty_flatbuffers::Builder;

const LIVE: &[u8] = include_bytes!("fixtures/live_freeplay_slice.bin");
const LIVE_BOT: &[u8] = include_bytes!("fixtures/live_bot_spawn_slice.bin");
const ACCEPTED: &[u8] = include_bytes!("fixtures/live_spawnbot_accepted.bin");

fn payloads(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut decoder = FrameDecoder::new();
    decoder.push(bytes);
    std::iter::from_fn(|| decoder.next_frame()).collect()
}

fn bundle(events: Vec<GameEvent>) -> Vec<u8> {
    encode_game_packet(&GameBundle {
        app_seconds: 0.0,
        delta_physics_frames: 0,
        events,
    })
    .unwrap()
}

/// A `ControllerPacket` holding one message with command tag `tag` and no body.
fn controller_with_tag(tag: u8) -> Vec<u8> {
    let mut b = Builder::new();
    let mut message = b.start_table();
    message.add_scalar(1, tag, 0u8).unwrap();
    let message = message.finish().unwrap();
    let vector = b.create_offset_vector(&[message]).unwrap();
    let mut root = b.start_table();
    root.add_offset(0, vector).unwrap();
    let root = root.finish().unwrap();
    b.finish(root, None).unwrap()
}

#[test]
fn the_tables_match_the_unions_in_the_format_note() {
    assert_eq!(GAME_MESSAGES.len(), 23);
    assert_eq!(GAME_MESSAGES[22], "PlayerPickupChange");
    assert_eq!(CONTROLLER_COMMANDS.len(), 14);
    assert_eq!(CONTROLLER_COMMANDS[13], "SetPaused");
}

#[test]
fn a_known_but_undecoded_tag_is_counted_but_not_unknown() {
    let mut census = Census::default();
    // 12 is BallHit, which the decoder reports as `Other`; 22 is the last tag in the union.
    census.add_game_packet(&bundle(vec![GameEvent::Other(12), GameEvent::Other(22)]));
    assert_eq!(census.inbound.get(&12), Some(&1));
    assert!(census.unknown_inbound().is_empty());
    assert!(census.report().contains("BallHit"));
    assert!(census
        .report()
        .contains("tags outside the known unions: none"));
}

#[test]
fn a_tag_outside_the_union_is_flagged_by_direction() {
    let mut census = Census::default();
    census.add_game_packet(&bundle(vec![GameEvent::Other(23), GameEvent::Other(200)]));
    census.add_controller_packet(&controller_with_tag(14));
    assert_eq!(census.unknown_inbound(), [23, 200]);
    assert_eq!(census.unknown_outbound(), [14]);
    let report = census.report();
    assert!(report.contains("UNKNOWN"), "{report}");
    assert!(report.contains("game to RLBot [23, 200], RLBot to game [14]"));
}

#[test]
fn unreadable_frames_are_counted_not_fatal() {
    let mut census = Census::default();
    census.add_game_packet(&[1, 2, 3]);
    census.add_controller_packet(&[]);
    census.add_game_packet(&bundle(vec![GameEvent::Other(8)]));
    assert_eq!(census.unreadable_frames, 2);
    assert_eq!(census.inbound.get(&8), Some(&1));
}

#[test]
fn a_payload_cut_off_inside_its_events_counts_none_of_them() {
    let mut census = Census::default();
    let whole = bundle(vec![GameEvent::Other(8), GameEvent::Other(9)]);
    // Every proper prefix is either unreadable or counts whole wrappers only, never a guess.
    for cut in 0..whole.len() {
        let mut one = Census::default();
        one.add_game_packet(&whole[..cut]);
        let counted: u64 = one.inbound.values().sum();
        assert!(counted == 0 || counted == 2, "cut {cut}: {counted}");
        assert_eq!(
            one.unreadable_frames,
            u64::from(counted == 0 && cut < whole.len())
        );
    }
    census.add_game_packet(&whole);
    assert_eq!(census.inbound.values().sum::<u64>(), 2);
}

#[test]
fn the_live_traffic_holds_only_tags_in_the_union() {
    let mut census = Census::default();
    for payload in payloads(LIVE).iter().chain(&payloads(LIVE_BOT)) {
        census.add_game_packet(payload);
    }
    assert_eq!(census.unreadable_frames, 0);
    assert!(census.unknown_inbound().is_empty(), "{}", census.report());
    // BallHit (12), PlayerInputChange (18) and BallExtraDataChange (21) were seen live as `Other`.
    for tag in [12, 18, 21] {
        assert!(census.inbound.contains_key(&tag), "{}", census.report());
    }
}

#[test]
fn what_the_bridge_sends_is_in_the_command_union() {
    let mut census = Census::default();
    for payload in payloads(ACCEPTED) {
        census.add_controller_packet(&payload);
    }
    let sent = encode_controller_packet(
        1,
        &[
            Command::Exec("x".into()),
            Command::SetPaused(true),
            Command::EndMatch,
        ],
    )
    .unwrap();
    census.add_controller_packet(&sent);
    assert_eq!(census.unreadable_frames, 0);
    assert!(census.unknown_outbound().is_empty());
    assert!(
        census.outbound.contains_key(&1),
        "the SpawnBot the game accepted"
    );
}
