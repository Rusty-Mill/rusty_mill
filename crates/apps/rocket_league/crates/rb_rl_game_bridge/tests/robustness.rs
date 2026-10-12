//! Things the review of #623 found: a socket that takes part of a frame, buffers that lie, and
//! humans reported as bots.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{self, Cursor, ErrorKind, Read, Write};

use rb_rl_game_bridge::{
    decode_controller_packet, decode_game_packet, encode_game_packet, Command, GameBundle,
    GameCommand, GameEvent, GameFolder, GameLink, RocketLeagueLink, WireError,
};
use rb_rlbot_wire::frame::FrameDecoder;
use rusty_flatbuffers::{Builder, Offset};

/// A non-blocking socket as a stalled game leaves it: every other write is refused, and the
/// ones that succeed take only 7 bytes.
struct Choppy {
    input: Cursor<Vec<u8>>,
    out: Vec<u8>,
    calls: usize,
}

impl Read for Choppy {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.input.read(buf)
    }
}

impl Write for Choppy {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.calls += 1;
        if self.calls % 2 == 1 {
            return Err(ErrorKind::WouldBlock.into());
        }
        let n = buf.len().min(7);
        self.out.extend_from_slice(&buf[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_socket_that_takes_part_of_a_frame_never_sees_a_half_frame_then_a_new_one() {
    let mut link = RocketLeagueLink::new(Choppy {
        input: Cursor::new(Vec::new()),
        out: Vec::new(),
        calls: 0,
    });
    // Each send is accepted even though the socket takes it a few bytes at a time.
    for text in ["a".repeat(50), "b".repeat(50), "c".repeat(50)] {
        link.send(GameCommand::Console(text)).unwrap();
    }
    assert!(link.pending_bytes() > 0, "the socket had not taken it all");
    // Whatever the game has not taken is written, in order, as the link next runs.
    for _ in 0..5_000 {
        if link.pending_bytes() == 0 {
            break;
        }
        let _ = link.poll_clump();
    }
    assert_eq!(link.pending_bytes(), 0);

    let mut decoder = FrameDecoder::new();
    decoder.push(&link.into_inner().out);
    let mut sent = Vec::new();
    while let Some(payload) = decoder.next_frame() {
        sent.extend(decode_controller_packet(&payload).unwrap());
    }
    let texts: Vec<(u16, String)> = sent
        .into_iter()
        .map(|(id, c)| match c {
            Command::Exec(t) => (id, t),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        texts,
        [
            (1, "a".repeat(50)),
            (2, "b".repeat(50)),
            (3, "c".repeat(50))
        ],
        "every frame arrived whole and in order, with consecutive ids"
    );
}

// ---- buffers that lie ----

/// A `GamePacket` buffer holding the given pre-built event tables, built the way the game would
/// but with whatever bytes the test wants in them.
fn packet_of(events: impl FnOnce(&mut Builder) -> Vec<(u8, Option<Offset>)>) -> Vec<u8> {
    let mut b = Builder::new();
    let made = events(&mut b);
    let mut wrappers = Vec::new();
    for (tag, body) in made {
        let mut t = b.start_table();
        t.add_scalar(0, tag, 0u8).unwrap();
        if let Some(o) = body {
            t.add_offset(1, o).unwrap();
        }
        wrappers.push(t.finish().unwrap());
    }
    let vector = b.create_offset_vector(&wrappers).unwrap();
    let mut t = b.start_table();
    t.add_offset(1, vector).unwrap();
    let root = t.finish().unwrap();
    b.finish(root, None).unwrap()
}

#[test]
fn an_unreadable_event_that_carries_no_state_is_one_odd_event_not_the_end() {
    let bytes = packet_of(|b| {
        // PlayerPhysicsChange (tag 8) with no player id and no physics: re-sent as it changes.
        let missing = b.start_table().finish().unwrap();
        // BallSpawn (tag 10) with an id: a good event after the odd one.
        let mut t = b.start_table();
        t.add_struct(0, &9u16.to_le_bytes(), 2).unwrap();
        let good = t.finish().unwrap();
        vec![(8, Some(missing)), (10, Some(good))]
    });
    assert!(rb_rl_game_bridge::decode_game_packet_strict(&bytes).is_err());
    let bundle = decode_game_packet(&bytes).unwrap();
    assert_eq!(
        bundle.events,
        [GameEvent::Other(8), GameEvent::BallSpawn { id: 9 }]
    );
}

#[test]
fn an_unreadable_event_that_carries_state_is_a_fault_not_a_skip() {
    // Each of these is announced once and never repeated, so dropping it would leave the link
    // wrong for good: a lost PlayerSpawn means a spawn acknowledgement that never comes.
    for (tag, name) in [
        (1, "GameSpawn"),
        (2, "GameStateChange"),
        (4, "GoalSpawn"),
        (5, "PickupSpawn"),
        (7, "PlayerSpawn"),
        (10, "BallSpawn"),
        (13, "TeamScoreChange"),
        (14, "ActorDestroyed"),
    ] {
        assert!(rb_rl_game_bridge::carries_state(tag), "{name}");
        // An empty body: every required field is missing (GameSpawn, GameStateChange and
        // TeamScoreChange have none, so an empty body is readable for those; a body-less event is
        // a fault for all).
        let empty = packet_of(|b| vec![(tag, Some(b.start_table().finish().unwrap()))]);
        let bodiless = packet_of(|_| vec![(tag, None)]);
        // No body at all is a fault for every one of them.
        match decode_game_packet(&bodiless) {
            Err(WireError::State { tag: t, .. }) => assert_eq!(t, tag, "{name}"),
            other => panic!("{name} without a body: {other:?}"),
        }
        // An empty body is a fault where a required field is missing; these have none, so it
        // reads as defaults.
        match decode_game_packet(&empty) {
            Err(WireError::State { tag: t, .. }) => assert_eq!(t, tag, "{name}"),
            Ok(_) if matches!(tag, 1 | 2 | 13) => {}
            other => panic!("{name} with an empty body: {other:?}"),
        }
    }
    // An unknown phase byte is a fault too, with the cause named.
    let bytes = packet_of(|b| {
        let mut t = b.start_table();
        t.add_scalar(0, 99u8, 0u8).unwrap();
        vec![(2, Some(t.finish().unwrap()))]
    });
    let err = decode_game_packet(&bytes).unwrap_err();
    assert!(matches!(
        &err,
        WireError::State { tag: 2, cause } if matches!(**cause, WireError::UnknownEnum { .. })
    ));
    assert!(err.to_string().contains("event 2"), "{err}");
    // The clock, a pad's state, and the physics and boost events are not state.
    for tag in [3, 6, 8, 9, 11] {
        assert!(!rb_rl_game_bridge::carries_state(tag));
    }
}

/// A link over `frames`, already framed, as a game that sent them and went quiet.
fn link_over(frames: Vec<Vec<u8>>) -> RocketLeagueLink<Choppy> {
    let input: Vec<u8> = frames
        .iter()
        .flat_map(|f| rb_rlbot_wire::frame::frame(f).unwrap())
        .collect();
    RocketLeagueLink::new(Choppy {
        input: Cursor::new(input),
        out: Vec::new(),
        calls: 0,
    })
}

fn map_and_human() -> Vec<u8> {
    encode_game_packet(&GameBundle {
        app_seconds: 0.0,
        delta_physics_frames: 0,
        events: vec![
            GameEvent::GameSpawn {
                map_name: "Stadium_P".into(),
                mode: 1,
                team_size: 1,
                gravity: None,
            },
            GameEvent::PlayerSpawn {
                id: 5,
                command_id: Some(0),
                name: "human".into(),
                team: 0,
            },
        ],
    })
    .unwrap()
}

#[test]
fn an_unreadable_spawn_after_the_map_ends_the_link_instead_of_leaving_a_stale_claim() {
    // The map and a human's car arrive; then the spawn of the bot's car arrives unreadable. If
    // the link skipped it, the spawn acknowledgement would never be seen and startup would wait
    // for ever; if a later spawn then reused its actor id it could inherit the bot's claim.
    let bad_spawn = packet_of(|b| vec![(7, Some(b.start_table().finish().unwrap()))]);
    let mut link = link_over(vec![map_and_human(), bad_spawn]);
    assert!(link.next_clump().unwrap().is_some(), "the opening is read");
    let err = link.next_clump().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidData);
    assert!(err.to_string().contains("event 7"), "{err}");
}

#[test]
fn an_unreadable_map_announcement_ends_the_link_too() {
    // A second map: its GameSpawn is the one-shot that would reset the folded state.
    let bad_map = packet_of(|_| vec![(1, None)]);
    let mut link = link_over(vec![map_and_human(), bad_map]);
    assert!(link.next_clump().unwrap().is_some());
    let err = link.next_clump().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidData);
    assert!(err.to_string().contains("event 1"), "{err}");
}

#[test]
fn an_unreadable_physics_update_after_the_map_does_not_end_the_link() {
    // Physics is re-sent as it changes, so one lost update heals itself.
    let odd = packet_of(|b| vec![(8, Some(b.start_table().finish().unwrap()))]);
    let mut link = link_over(vec![map_and_human(), odd]);
    assert!(link.next_clump().unwrap().is_some());
    assert!(
        link.next_clump().unwrap().is_some(),
        "the odd update is skipped"
    );
}

#[test]
fn a_message_cannot_make_decoding_allocate_without_bound() {
    // More events than any real message holds.
    let many = GameBundle {
        app_seconds: 0.0,
        delta_physics_frames: 0,
        events: vec![GameEvent::BallSpawn { id: 1 }; 5_000],
    };
    assert_eq!(
        decode_game_packet(&encode_game_packet(&many).unwrap()).unwrap_err(),
        WireError::TooLarge("events")
    );
    // Few events but a great deal of string between them.
    let long = GameBundle {
        app_seconds: 0.0,
        delta_physics_frames: 0,
        events: (0..3)
            .map(|_| GameEvent::GameSpawn {
                map_name: "m".repeat(30_000),
                mode: 1,
                team_size: 1,
                gravity: None,
            })
            .collect(),
    };
    assert_eq!(
        decode_game_packet(&encode_game_packet(&long).unwrap()).unwrap_err(),
        WireError::TooLarge("string bytes")
    );
    // A real map announcement is far below both limits.
    let real = GameBundle {
        app_seconds: 0.0,
        delta_physics_frames: 0,
        events: vec![GameEvent::GameSpawn {
            map_name: "TrainStation_P".into(),
            mode: 1,
            team_size: 1,
            gravity: None,
        }],
    };
    assert!(decode_game_packet(&encode_game_packet(&real).unwrap()).is_ok());
}

// ---- humans are not bots ----

#[test]
fn only_cars_with_a_spawn_command_are_reported_as_bots() {
    let spawn = |id, command_id, name: &str| GameEvent::PlayerSpawn {
        id,
        command_id,
        name: name.into(),
        team: 0,
    };
    let mut folder = GameFolder::new();
    folder.apply(&GameBundle {
        app_seconds: 0.0,
        delta_physics_frames: 1,
        events: vec![
            spawn(1, Some(0), "Human"),
            spawn(2, Some(7), "Bot"),
            spawn(3, None, "Unannounced"),
        ],
    });
    let players = folder.packet().players;
    let flags: Vec<(String, bool)> = players.into_iter().map(|p| (p.name, p.is_bot)).collect();
    assert_eq!(
        flags,
        [
            ("Human".to_string(), false),
            ("Bot".to_string(), true),
            ("Unannounced".to_string(), false)
        ]
    );
}

#[test]
fn strict_analysis_distinguishes_missing_known_events_from_opaque_events() {
    use rb_rl_game_bridge::decode_game_packet_strict;
    for body in [false, true] {
        let bytes = packet_of(|b| vec![(8, body.then(|| b.start_table().finish().unwrap()))]);
        assert!(matches!(
            decode_game_packet_strict(&bytes),
            Err(WireError::Missing(_))
        ));
        assert_eq!(
            decode_game_packet(&bytes).unwrap().events,
            [GameEvent::Other(8)]
        );
    }
    let bytes = packet_of(|_| vec![(0, None), (200, None)]);
    assert_eq!(
        decode_game_packet_strict(&bytes).unwrap().events,
        [GameEvent::Other(0), GameEvent::Other(200)]
    );
}

#[test]
fn missing_known_command_body_is_not_silently_skipped() {
    for tag in [3u8, 200] {
        let mut b = Builder::new();
        let mut t = b.start_table();
        t.add_struct(0, &1u16.to_le_bytes(), 2).unwrap();
        t.add_scalar(1, tag, 0u8).unwrap();
        let wrapper = t.finish().unwrap();
        let v = b.create_offset_vector(&[wrapper]).unwrap();
        let mut t = b.start_table();
        t.add_offset(0, v).unwrap();
        let root = t.finish().unwrap();
        let payload = b.finish(root, None).unwrap();
        let got = decode_controller_packet(&payload);
        if tag == 3 {
            assert_eq!(got.unwrap_err(), WireError::Missing("command body"));
        } else {
            assert!(got.unwrap().is_empty());
        }
    }
}

#[test]
fn pickup_state_enum_is_validated_without_losing_tolerant_session_handling() {
    use rb_rl_game_bridge::decode_game_packet_strict;
    for state in [0u8, 1, 99, 255] {
        let bytes = packet_of(|b| {
            let mut t = b.start_table();
            t.add_struct(0, &7u16.to_le_bytes(), 2).unwrap();
            t.add_scalar(1, state, 0u8).unwrap();
            vec![(6, Some(t.finish().unwrap()))]
        });
        if state <= 1 {
            let expected = vec![GameEvent::PickupState {
                id: 7,
                picked_up: state == 1,
            }];
            assert_eq!(decode_game_packet_strict(&bytes).unwrap().events, expected);
            assert_eq!(decode_game_packet(&bytes).unwrap().events, expected);
        } else {
            assert_eq!(
                decode_game_packet_strict(&bytes).unwrap_err(),
                WireError::UnknownEnum {
                    name: "PickupStateType",
                    value: state
                }
            );
            assert_eq!(
                decode_game_packet(&bytes).unwrap().events,
                [GameEvent::Other(6)]
            );
        }
    }
}
