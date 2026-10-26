//! The resumable event stream (C05): numbered events, and replay of the
//! events a client missed while disconnected.

#![cfg(target_os = "linux")]

mod common;

use common::{Client, block_for, start_server};
use rusty_mux_core::{Event, SessionId};
use rusty_mux_proto::{Cursor, Features, Message};
use rusty_uuid::Uuid;

const SEQ: Features = Features::EVENT_SEQ;

fn create_session(client: &mut Client, name: &str) -> SessionId {
    let id = client
        .request(|req| Message::CreateSession {
            req,
            name: name.into(),
        })
        .expect("id");
    SessionId::from_uuid(id)
}

/// Reads until the event announcing `session` arrives.
fn wait_for_session(client: &mut Client, session: SessionId) {
    let created = Event::SessionCreated { session };
    while !client.events.iter().any(|(_, e)| *e == created) {
        client.recv();
    }
}

fn numbers(client: &Client) -> Vec<u64> {
    client
        .events
        .iter()
        .map(|(seq, _)| seq.expect("numbered"))
        .collect()
}

fn last_seen(client: &Client) -> Cursor {
    let start = client.cursor.expect("cursor");
    Cursor {
        seq: numbers(client).last().copied().unwrap_or(start.seq),
        ..start
    }
}

#[test]
fn a_resumed_stream_has_no_gap_and_no_repeat() {
    let path = start_server();
    let mut a = Client::connect_with(&path, SEQ, None);
    assert_eq!(a.cursor.map(|c| c.seq), Some(0));
    let first = create_session(&mut a, "first");
    wait_for_session(&mut a, first);
    let seen = last_seen(&a);
    drop(a);

    // Changes made while A is away, by someone else.
    let mut b = Client::connect(&path);
    let missed = create_session(&mut b, "missed");

    let mut a = Client::connect_with(&path, SEQ, Some(seen));
    assert_eq!(a.cursor, Some(seen), "the server resumed the stream");
    let after = create_session(&mut a, "after");
    wait_for_session(&mut a, after);

    let resumed = numbers(&a);
    let expected: Vec<u64> = (seen.seq + 1..=seen.seq + resumed.len() as u64).collect();
    assert_eq!(resumed, expected, "contiguous from the resume point");
    assert!(
        a.events
            .iter()
            .any(|(_, e)| *e == Event::SessionCreated { session: missed }),
        "the missed change was replayed"
    );
}

#[test]
fn a_cursor_from_another_run_starts_afresh() {
    let path = start_server();
    let mut b = Client::connect(&path);
    create_session(&mut b, "before");

    let stale = Cursor {
        epoch: Uuid::from_bytes([7; 16]),
        seq: 1,
    };
    let mut a = Client::connect_with(&path, SEQ, Some(stale));
    let start = a.cursor.expect("cursor");
    assert_ne!(start, stale, "not resumed");
    let session = create_session(&mut a, "after");
    wait_for_session(&mut a, session);
    assert_eq!(
        numbers(&a).first(),
        Some(&(start.seq + 1)),
        "nothing replayed"
    );
}

#[test]
fn a_client_without_the_feature_gets_unnumbered_events() {
    let path = start_server();
    let mut a = Client::connect_with(&path, Features::NONE, None);
    assert_eq!(a.cursor, None);
    let session = create_session(&mut a, "s");
    wait_for_session(&mut a, session);
    assert!(a.events.iter().all(|(seq, _)| seq.is_none()));
}

#[test]
fn status_changes_reach_only_clients_that_asked() {
    let path = start_server();
    let wanted = SEQ.union(Features::STATUS).union(Features::CHECKPOINT_V3);
    let mut watcher = Client::connect_with(&path, wanted, None);
    let mut plain = Client::connect_with(&path, SEQ, None);
    let block = block_for(&mut watcher);
    watcher.input(block, b"printf '\\033]7501;state=working\\007'\r");
    let status = Event::BlockStatus { block };
    while !watcher.events.iter().any(|(_, e)| *e == status) {
        watcher.recv();
    }

    // Events reach every client in one order, so once the plain client sees
    // a later one it has missed the status event for good.
    let later = create_session(&mut watcher, "later");
    wait_for_session(&mut plain, later);
    assert!(!plain.events.iter().any(|(_, e)| *e == status));
    let gaps = numbers(&plain)
        .windows(2)
        .filter(|w| w[1] != w[0] + 1)
        .count();
    assert_eq!(gaps, 1, "its numbers skip the status event");
}

#[test]
fn replay_sends_status_events_only_to_clients_that_asked() {
    let path = start_server();
    let status = SEQ.union(Features::STATUS);
    let mut away = Vec::new();
    for features in [SEQ, status] {
        let mut client = Client::connect_with(&path, features, None);
        let session = create_session(&mut client, "before");
        wait_for_session(&mut client, session);
        away.push((features, last_seen(&client)));
    }

    // While both are away: a status change, then a later event.
    let mut writer = Client::connect_with(&path, status.union(Features::CHECKPOINT_V3), None);
    let block = block_for(&mut writer);
    writer.input(block, b"printf '\\033]7501;state=working\\007'\r");
    let changed = Event::BlockStatus { block };
    while !writer.events.iter().any(|(_, e)| *e == changed) {
        writer.recv();
    }
    let after = create_session(&mut writer, "after");

    for (features, seen) in away {
        let mut client = Client::connect_with(&path, features, Some(seen));
        assert_eq!(client.cursor, Some(seen), "resumed");
        wait_for_session(&mut client, after);
        let replayed = client.events.iter().any(|(_, e)| *e == changed);
        assert_eq!(
            replayed,
            features.contains(Features::STATUS),
            "{features:?}"
        );
    }
}
