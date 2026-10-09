//! Required versus optional fields, checked on the wire bytes themselves: a small reader
//! written here (not the crate's decoder) finds a table's vtable entry, so a test can delete a
//! field from a reference payload or confirm an encoded message carries every field the
//! schema requires. Which fields are required follows the pinned schema (rev c38374e): an
//! `Option` in the reference types is optional; a string, vector, struct, table or union
//! without one is required.
#![allow(clippy::unwrap_used, clippy::panic)]

mod fixtures;

use rb_rlbot_wire::*;

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn u32_at(b: &[u8], at: usize) -> usize {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap()) as usize
}

fn u16_at(b: &[u8], at: usize) -> usize {
    usize::from(u16::from_le_bytes(b[at..at + 2].try_into().unwrap()))
}

/// Position of the vtable entry for `slot` of the table at `table`, if the vtable is long enough.
fn entry(b: &[u8], table: usize, slot: usize) -> Option<usize> {
    let soffset = i32::from_le_bytes(b[table..table + 4].try_into().unwrap());
    let vtable = usize::try_from(i64::try_from(table).unwrap() - i64::from(soffset)).unwrap();
    let at = vtable + 4 + 2 * slot;
    (at + 2 <= vtable + u16_at(b, vtable)).then_some(at)
}

/// Where field `slot` lives, if the table has it.
fn field(b: &[u8], table: usize, slot: usize) -> Option<usize> {
    let at = entry(b, table, slot)?;
    (u16_at(b, at) != 0).then(|| table + u16_at(b, at))
}

/// The table or vector a reference field points at.
fn follow(b: &[u8], table: usize, slot: usize) -> usize {
    let at = field(b, table, slot).unwrap();
    at + u32_at(b, at)
}

/// Table `i` of the vector of tables in `slot`.
fn element(b: &[u8], table: usize, slot: usize, i: usize) -> usize {
    let at = follow(b, table, slot) + 4 + 4 * i;
    at + u32_at(b, at)
}

fn root(b: &[u8]) -> usize {
    u32_at(b, 0)
}

/// The message table inside the packet (packet slot 1).
fn message(b: &[u8]) -> usize {
    follow(b, root(b), 1)
}

/// Removes a field by zeroing its vtable entry (what a writer that omitted it would have left).
fn clear(b: &mut [u8], table: usize, slot: usize) {
    let at = entry(b, table, slot).unwrap();
    b[at] = 0;
    b[at + 1] = 0;
}

type Locate = fn(&[u8]) -> usize;

fn missing_interface(hex: &str, locate: Locate, slot: usize, name: &'static str) {
    let mut buf = unhex(hex);
    let table = locate(&buf);
    clear(&mut buf, table, slot);
    assert_eq!(
        InterfaceMessage::from_payload(&buf),
        Err(Error::Missing(name)),
        "{name}"
    );
}

fn missing_core(hex: &str, locate: Locate, slot: usize, name: &'static str) {
    let mut buf = unhex(hex);
    let table = locate(&buf);
    clear(&mut buf, table, slot);
    assert_eq!(
        CoreMessage::from_payload(&buf),
        Err(Error::Missing(name)),
        "{name}"
    );
}

#[test]
fn a_missing_required_client_field_is_rejected() {
    missing_interface(fixtures::I_CONNECTION, message, 0, "agent_id");
    missing_interface(fixtures::I_INPUT, message, 1, "controller_state");
    for (slot, name) in [
        (1, "launcher_arg"),
        (4, "game_map_upk"),
        (5, "player_configurations"),
        (6, "script_configurations"),
    ] {
        missing_interface(fixtures::I_CONFIG, message, slot, name);
    }
    missing_interface(fixtures::I_STATE, message, 0, "ball_states");
    missing_interface(fixtures::I_STATE, message, 1, "car_states");
    missing_interface(
        fixtures::I_STATE,
        |b| element(b, message(b), 0, 0),
        0,
        "physics",
    );
}

#[test]
fn a_missing_union_value_is_rejected_for_every_player_class() {
    // Players 0, 1, 2 are a custom bot, a Psyonix bot and a human (see the fixture).
    for (i, _) in ["custom", "psyonix", "human"].iter().enumerate() {
        let mut buf = unhex(fixtures::I_CONFIG);
        let config = message(&buf);
        let player = element(&buf, config, 5, i);
        clear(&mut buf, player, 1);
        assert_eq!(
            InterfaceMessage::from_payload(&buf),
            Err(Error::Missing("variety")),
            "player {i}"
        );
    }
}

#[test]
fn missing_required_strings_inside_a_player_are_rejected() {
    let player = |b: &[u8]| element(b, message(b), 5, 0);
    for (slot, name) in [
        (0, "name"),
        (1, "root_dir"),
        (2, "run_command"),
        (4, "agent_id"),
    ] {
        let mut buf = unhex(fixtures::I_CONFIG);
        let bot = follow(&buf, player(&buf), 1);
        clear(&mut buf, bot, slot);
        assert_eq!(
            InterfaceMessage::from_payload(&buf),
            Err(Error::Missing(name))
        );
    }
}

#[test]
fn a_missing_required_core_field_is_rejected() {
    missing_core(fixtures::C_TEAM, message, 1, "controllables");
    for (slot, name) in [
        (0, "players"),
        (1, "boost_pads"),
        (2, "balls"),
        (3, "match_info"),
        (4, "teams"),
        (5, "tiles"),
    ] {
        missing_core(fixtures::C_PACKET, message, slot, name);
    }
    for (slot, name) in [
        (0, "physics"),
        (1, "score_info"),
        (2, "hitbox"),
        (3, "hitbox_offset"),
        (10, "name"),
        (14, "accolades"),
        (15, "last_input"),
        (20, "dodge_dir"),
    ] {
        missing_core(
            fixtures::C_PACKET,
            |b| element(b, message(b), 0, 0),
            slot,
            name,
        );
    }
    // The collision shape is a union: either half missing is an error.
    for slot in [1, 2] {
        missing_core(
            fixtures::C_PACKET,
            |b| element(b, message(b), 2, 0),
            slot,
            "shape",
        );
    }
    missing_core(
        fixtures::C_PACKET,
        |b| element(b, message(b), 2, 0),
        0,
        "physics",
    );
    for (slot, name) in [(0, "boost_pads"), (1, "goals"), (2, "tiles")] {
        missing_core(fixtures::C_FIELD, message, slot, name);
    }
    missing_core(
        fixtures::C_FIELD,
        |b| element(b, message(b), 0, 0),
        0,
        "location",
    );
    missing_core(
        fixtures::C_FIELD,
        |b| element(b, message(b), 1, 0),
        1,
        "location",
    );
    missing_core(
        fixtures::C_FIELD,
        |b| element(b, message(b), 1, 0),
        2,
        "direction",
    );
}

#[test]
fn genuinely_optional_fields_may_be_absent() {
    // Mutators, loadouts and environment are `Option` in the reference types.
    let mut buf = unhex(fixtures::I_CONFIG);
    let config = message(&buf);
    clear(&mut buf, config, 10);
    let bot = follow(&buf, element(&buf, config, 5, 0), 1);
    clear(&mut buf, bot, 6);
    let Ok(InterfaceMessage::MatchConfiguration(c)) = InterfaceMessage::from_payload(&buf) else {
        panic!("optional fields are not required");
    };
    assert_eq!(c.mutators, None);
    let PlayerClass::CustomBot(bot) = &c.player_configurations[0].variety else {
        panic!("custom bot");
    };
    assert_eq!(bot.environment, None);

    // State setting: the match info, a car's physics and boost are all optional.
    let mut buf = unhex(fixtures::I_STATE);
    let state = message(&buf);
    clear(&mut buf, state, 2);
    let car = element(&buf, state, 1, 0);
    clear(&mut buf, car, 0);
    clear(&mut buf, car, 1);
    let Ok(InterfaceMessage::DesiredGameState(s)) = InterfaceMessage::from_payload(&buf) else {
        panic!("optional fields are not required");
    };
    assert_eq!(s.match_info, None);
    assert_eq!(
        s.car_states[0],
        DesiredCarState {
            physics: None,
            boost_amount: None
        }
    );
}

// ---- what this crate writes must carry every field the schema requires ----

/// Every slot in `slots` is present in the table at `table`.
fn all_present(b: &[u8], table: usize, slots: &[usize], what: &str) {
    for slot in slots {
        assert!(
            field(b, table, *slot).is_some(),
            "{what} lacks required slot {slot}"
        );
    }
}

fn physics() -> Physics {
    Physics {
        location: Vec3 {
            x: 1.0,
            y: 2.0,
            z: 3.0,
        },
        ..Physics::default()
    }
}

fn player() -> PlayerInfo {
    PlayerInfo {
        physics: physics(),
        air_state: AirState::InAir,
        demolished_timeout: 0.0,
        is_bot: false,
        name: String::new(),
        team: 1,
        boost: 12.0,
        player_id: 4,
    }
}

fn packet(players: usize, balls: usize) -> CoreMessage {
    CoreMessage::GamePacket(GamePacket {
        players: vec![player(); players],
        boost_pads: vec![],
        balls: vec![BallInfo { physics: physics() }; balls],
        match_info: MatchInfo {
            seconds_elapsed: 0.0,
            game_time_remaining: 0.0,
            is_overtime: false,
            is_unlimited_time: false,
            match_phase: MatchPhase::Inactive,
            frame_num: 0,
        },
        teams: vec![],
    })
}

#[test]
fn an_encoded_game_packet_carries_every_required_field_empty_or_populated() {
    for (players, balls) in [(0, 0), (1, 0), (0, 1), (2, 3)] {
        let buf = packet(players, balls).to_payload().unwrap();
        let msg = message(&buf);
        all_present(&buf, msg, &[0, 1, 2, 3, 4, 5], "GamePacket");
        all_present(&buf, follow(&buf, msg, 3), &[], "MatchInfo");
        for i in 0..players {
            let p = element(&buf, msg, 0, i);
            all_present(&buf, p, &[0, 1, 2, 3, 10, 14, 15, 20], "PlayerInfo");
            all_present(&buf, follow(&buf, p, 2), &[], "hitbox");
        }
        for i in 0..balls {
            let ball = element(&buf, msg, 2, i);
            all_present(&buf, ball, &[0, 1, 2], "BallInfo");
            // The shape union carries a real tag, not "none".
            assert_eq!(buf[field(&buf, ball, 1).unwrap()], 2, "SphereShape");
        }
        assert_eq!(
            CoreMessage::from_payload(&buf).unwrap(),
            packet(players, balls)
        );
    }
}

#[test]
fn an_encoded_field_info_carries_every_required_field_empty_or_populated() {
    let pad = BoostPad {
        location: Vec3::default(),
        is_full_boost: true,
    };
    let goal = GoalInfo {
        team_num: 0,
        location: Vec3::default(),
        direction: Vec3::default(),
        width: 1.0,
        height: 2.0,
    };
    for info in [
        FieldInfo::default(),
        FieldInfo {
            boost_pads: vec![pad],
            goals: vec![],
        },
        FieldInfo {
            boost_pads: vec![pad, pad],
            goals: vec![goal],
        },
    ] {
        let buf = CoreMessage::FieldInfo(info.clone()).to_payload().unwrap();
        let msg = message(&buf);
        all_present(&buf, msg, &[0, 1, 2], "FieldInfo");
        for i in 0..info.boost_pads.len() {
            all_present(&buf, element(&buf, msg, 0, i), &[0], "BoostPad");
        }
        for i in 0..info.goals.len() {
            all_present(&buf, element(&buf, msg, 1, i), &[1, 2], "GoalInfo");
        }
        assert_eq!(
            CoreMessage::from_payload(&buf).unwrap(),
            CoreMessage::FieldInfo(info)
        );
    }
}

#[test]
fn encoded_client_messages_carry_every_required_field() {
    let buf = InterfaceMessage::MatchConfiguration(MatchConfiguration {
        player_configurations: vec![PlayerConfiguration {
            variety: PlayerClass::Human,
            team: 0,
            player_id: 0,
        }],
        ..MatchConfiguration::default()
    })
    .to_payload()
    .unwrap();
    let config = message(&buf);
    all_present(&buf, config, &[1, 4, 5, 6], "MatchConfiguration");
    all_present(
        &buf,
        element(&buf, config, 5, 0),
        &[0, 1],
        "PlayerConfiguration",
    );

    let buf = InterfaceMessage::ConnectionSettings(ConnectionSettings::default())
        .to_payload()
        .unwrap();
    all_present(
        &buf,
        message(&buf),
        &[0],
        "ConnectionSettings (empty id still written)",
    );

    let buf = InterfaceMessage::DesiredGameState(DesiredGameState::default())
        .to_payload()
        .unwrap();
    all_present(&buf, message(&buf), &[0, 1], "DesiredGameState");

    let buf = InterfaceMessage::PlayerInput(PlayerInput::default())
        .to_payload()
        .unwrap();
    all_present(&buf, message(&buf), &[1], "PlayerInput");
}
