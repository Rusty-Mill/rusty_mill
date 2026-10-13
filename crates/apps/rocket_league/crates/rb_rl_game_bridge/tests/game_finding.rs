//! `GameListener::wait_for_game`: the game is the connection that speaks like it, not the first
//! one to arrive.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use rb_rl_game_bridge::{
    decode_game_packet, encode_game_packet, GameBundle, GameEvent, GameListener, MAX_CANDIDATES,
};
use rb_rlbot_wire::frame::frame;

/// A framed first message as the game sends it.
fn game_message() -> Vec<u8> {
    let bundle = GameBundle {
        app_seconds: 0.0,
        delta_physics_frames: 1,
        events: vec![GameEvent::GameSpawn {
            map_name: "Stadium_P".into(),
            mode: 1,
            team_size: 1,
            gravity: None,
        }],
    };
    frame(&encode_game_packet(&bundle).unwrap()).unwrap()
}

fn connect(listener: &GameListener) -> TcpStream {
    TcpStream::connect(listener.local_addr().unwrap()).unwrap()
}

/// Reads one framed message from the adopted stream and decodes it.
fn read_first(mut stream: TcpStream) -> GameBundle {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut len = [0u8; 2];
    stream.read_exact(&mut len).unwrap();
    let mut payload = vec![0u8; usize::from(u16::from_be_bytes(len))];
    stream.read_exact(&mut payload).unwrap();
    decode_game_packet(&payload).unwrap()
}

fn wait(listener: &GameListener) -> (Option<TcpStream>, Vec<String>) {
    let mut notes = Vec::new();
    let stream = listener
        .wait_for_game(Duration::from_secs(10), |n| notes.push(n.to_string()))
        .unwrap();
    (stream, notes)
}

#[test]
fn nobody_connecting_times_out() {
    let listener = GameListener::bind(0).unwrap();
    let got = listener
        .wait_for_game(Duration::from_millis(60), |_| {})
        .unwrap();
    assert!(got.is_none());
}

#[test]
fn a_silent_connection_does_not_hold_the_games_place() {
    let listener = GameListener::bind(0).unwrap();
    let _squatter = connect(&listener);
    let mut game = connect(&listener);
    game.write_all(&game_message()).unwrap();
    let (stream, notes) = wait(&listener);
    // The game was taken, with its first message still unread on the stream.
    let first = read_first(stream.expect("the game was not found"));
    assert!(matches!(first.events[0], GameEvent::GameSpawn { .. }));
    assert!(
        notes.iter().any(|n| n.contains("waiting to speak")),
        "{notes:?}"
    );
}

#[test]
fn a_connection_that_sends_junk_or_closes_is_dropped_and_the_game_still_found() {
    let listener = GameListener::bind(0).unwrap();
    let mut junk = connect(&listener);
    // A complete frame whose payload is not a game message.
    junk.write_all(&frame(&[0xFF; 40]).unwrap()).unwrap();
    let mut game = connect(&listener);
    game.write_all(&game_message()).unwrap();
    let (stream, notes) = wait(&listener);
    assert!(stream.is_some());
    assert!(
        notes.iter().any(|n| n.contains("not the game's")),
        "junk should be named: {notes:?}"
    );
}

#[test]
fn a_connection_that_closes_without_a_word_is_dropped() {
    let listener = GameListener::bind(0).unwrap();
    let addr = listener.local_addr().unwrap();
    drop(connect(&listener));
    let late = thread::spawn(move || {
        thread::sleep(Duration::from_millis(200));
        let mut game = TcpStream::connect(addr).unwrap();
        game.write_all(&game_message()).unwrap();
        game
    });
    let (stream, notes) = wait(&listener);
    assert!(stream.is_some());
    assert!(
        notes.iter().any(|n| n.contains("closed without a word")),
        "{notes:?}"
    );
    drop(late.join().unwrap());
}

#[test]
fn a_first_message_that_arrives_in_pieces_is_waited_for() {
    let listener = GameListener::bind(0).unwrap();
    let mut game = connect(&listener);
    let message = game_message();
    let (head, tail) = message.split_at(message.len() / 2);
    game.write_all(head).unwrap();
    let tail = tail.to_vec();
    let writer = thread::spawn(move || {
        thread::sleep(Duration::from_millis(150));
        game.write_all(&tail).unwrap();
        game
    });
    let (stream, _) = wait(&listener);
    let first = read_first(stream.expect("the game was not found"));
    assert!(matches!(first.events[0], GameEvent::GameSpawn { .. }));
    drop(writer.join().unwrap());
}

#[test]
fn a_lone_silent_connection_is_waited_on_past_the_timeout_not_given_up_on() {
    let listener = GameListener::bind(0).unwrap();
    let addr = listener.local_addr().unwrap();
    let _quiet = connect(&listener);
    let (tx, rx) = mpsc::channel();
    let finder = thread::spawn(move || {
        // The timeout is short; the wait must still not end while a connection is waiting.
        let got = listener
            .wait_for_game(Duration::from_millis(100), |_| {})
            .unwrap();
        tx.send(got.is_some()).unwrap();
    });
    assert!(
        rx.recv_timeout(Duration::from_millis(600)).is_err(),
        "gave up while a connection was waiting"
    );
    let mut game = TcpStream::connect(addr).unwrap();
    game.write_all(&game_message()).unwrap();
    assert!(rx.recv_timeout(Duration::from_secs(5)).unwrap());
    finder.join().unwrap();
}

#[test]
fn silent_connections_cannot_fill_the_places_and_keep_the_game_out() {
    let listener = GameListener::bind(0).unwrap();
    let squatters: Vec<TcpStream> = (0..MAX_CANDIDATES + 5)
        .map(|_| connect(&listener))
        .collect();
    let mut game = connect(&listener);
    game.write_all(&game_message()).unwrap();
    let (stream, notes) = wait(&listener);
    // The newest connection wins a full house, so the game, arriving last, gets in.
    let first = read_first(stream.expect("the game was kept out"));
    assert!(matches!(first.events[0], GameEvent::GameSpawn { .. }));
    assert!(
        notes.iter().any(|n| n.contains("dropped the oldest")),
        "{notes:?}"
    );
    drop(squatters);
}
