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
use std::io::BufRead;
use std::process::{Command, Stdio};

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

/// Kills and reaps the bridge process however the test ends, so a failed assertion or a timeout
/// leaves nothing running.
struct KillOnDrop(std::process::Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
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

/// Reviewer case 2: the game is queued with its whole first message and then more silent
/// connections than there are places, all before the wait begins. The game must not be the one
/// dropped to make room.
#[test]
fn a_game_that_arrived_first_is_not_dropped_by_the_silent_connections_after_it() {
    let listener = GameListener::bind(0).unwrap();
    let mut game = connect(&listener);
    game.write_all(&game_message()).unwrap();
    let silent: Vec<TcpStream> = (0..MAX_CANDIDATES * 2)
        .map(|_| connect(&listener))
        .collect();
    let (stream, _) = wait(&listener);
    let first = read_first(stream.expect("the game was dropped to make room"));
    assert!(matches!(first.events[0], GameEvent::GameSpawn { .. }));
    drop(silent);
}

/// Reviewer case 4: a few bytes of a message and then a clean close cannot be seen as an end of
/// stream with `peek`, so they are dropped as stalled instead, and the timeout then resumes.
#[test]
fn a_truncated_message_followed_by_a_close_is_dropped_as_stalled() {
    let message = game_message();
    for cut in [1, 2, 5, message.len() - 1] {
        let listener = GameListener::bind(0).unwrap();
        let mut truncated = connect(&listener);
        truncated.write_all(&message[..cut]).unwrap();
        drop(truncated);
        let (tx, rx) = mpsc::channel();
        // Nothing else ever connects: once the stalled peer is dropped the timeout must run
        // out. A bound on the whole wait turns "never ends" into a failure, not a hang.
        let worker = thread::spawn(move || {
            let mut notes = Vec::new();
            let got = listener
                .wait_for_game_stalling(
                    Duration::from_millis(100),
                    Duration::from_millis(150),
                    |n| notes.push(n.to_string()),
                )
                .unwrap();
            tx.send((got.is_none(), notes)).unwrap();
        });
        let (nothing_found, notes) = rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap_or_else(|_| panic!("cut at {cut}: the wait never ended"));
        worker.join().unwrap();
        assert!(nothing_found, "cut at {cut}");
        assert!(
            notes.iter().any(|n| n.contains("stalled")),
            "cut at {cut}: {notes:?}"
        );
    }
}

/// A partial message that keeps growing is not stalled.
#[test]
fn a_message_still_arriving_is_not_called_stalled() {
    let listener = GameListener::bind(0).unwrap();
    let mut game = connect(&listener);
    let message = game_message();
    let writer = thread::spawn(move || {
        for byte in message {
            game.write_all(&[byte]).unwrap();
            thread::sleep(Duration::from_millis(2));
        }
        game
    });
    let got = listener
        .wait_for_game_stalling(Duration::from_secs(10), Duration::from_millis(500), |_| {})
        .unwrap();
    assert!(got.is_some());
    drop(writer.join().unwrap());
}

/// Rejecting junk and then overflowing drops the oldest of the ones left, in arrival order.
/// (This sees the eviction side only; the rejection side is pinned by the unit test
/// `rejecting_a_peer_keeps_arrival_order_for_the_next_overflow` in `link.rs`.)
#[test]
fn a_rejection_then_an_overflow_drops_the_oldest_remaining_in_order() {
    let listener = GameListener::bind(0).unwrap();
    let mut junk = connect(&listener);
    junk.write_all(&frame(&[0xFF; 40]).unwrap()).unwrap();
    let quiet: Vec<TcpStream> = (0..MAX_CANDIDATES + 2)
        .map(|_| connect(&listener))
        .collect();
    let mut game = connect(&listener);
    game.write_all(&game_message()).unwrap();
    let (stream, notes) = wait(&listener);
    assert!(stream.is_some());
    let evicted: Vec<String> = notes
        .iter()
        .filter(|n| n.contains("dropped the oldest"))
        .cloned()
        .collect();
    // Ten quiet connections and the game after the rejected junk, eight places: the first three
    // quiet ones were dropped, oldest first, and the junk was dropped for what it was.
    assert_eq!(evicted.len(), 3, "{notes:?}");
    for (n, peer) in evicted.iter().zip(&quiet[..3]) {
        assert!(
            n.contains(&peer.local_addr().unwrap().to_string()),
            "{n} should name {}",
            peer.local_addr().unwrap()
        );
    }
    assert!(
        notes.iter().any(|n| n.contains("not the game's")),
        "{notes:?}"
    );
}

/// Reviewer case 1: what the live-test runner waits for. A silent game (dialled, said nothing)
/// must not hold back the line that starts the bots, and the runner's own gate - the dial line -
/// must appear while the game is still silent. `rl-lib.ps1` waits for `waiting to speak`.
#[test]
fn a_silent_game_does_not_hold_back_the_lines_the_runner_waits_for() {
    let free = || {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let (game_port, bots_port) = (free(), free());
    let mut child = KillOnDrop(
        Command::new(env!("CARGO_BIN_EXE_rb_rl_bridge"))
            .args([
                "--game-port",
                &game_port.to_string(),
                "--port",
                &bots_port.to_string(),
                "bot:0",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let (tx, rx) = mpsc::channel();
    let stdout = child.0.stdout.take().unwrap();
    let reader = thread::spawn(move || {
        for line in std::io::BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
        {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let next = |what: &str| {
        rx.recv_timeout(Duration::from_secs(10))
            .unwrap_or_else(|_| panic!("no line for: {what}"))
    };
    // The bots' port is announced without any game.
    let mut seen = Vec::new();
    loop {
        let line = next("serving bots on");
        seen.push(line.clone());
        if line.contains("serving bots on") {
            break;
        }
    }
    // The game dials and stays silent: the dial line appears, "connected" does not.
    let _silent_game = TcpStream::connect(("127.0.0.1", game_port)).unwrap();
    loop {
        let line = next("waiting to speak");
        assert!(!line.contains("the game connected"), "{line}");
        if line.contains("waiting to speak") {
            break;
        }
    }
    assert!(
        rx.recv_timeout(Duration::from_millis(500)).is_err(),
        "it should be waiting"
    );
    // Now it speaks, and only then is it the game.
    let mut speaking = TcpStream::connect(("127.0.0.1", game_port)).unwrap();
    speaking.write_all(&game_message()).unwrap();
    loop {
        if next("the game connected").contains("the game connected") {
            break;
        }
    }
    drop(child);
    reader.join().unwrap();
}
