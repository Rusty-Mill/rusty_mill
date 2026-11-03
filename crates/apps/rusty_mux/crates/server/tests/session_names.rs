//! Generated session names (C50): an empty name from a client with
//! `SESSION_NAMES` becomes one no other session has.

#![cfg(any(target_os = "linux", windows))]

mod common;

use common::{Client, start_server};
use rusty_mux_core::{SessionId, is_generated};
use rusty_mux_proto::{Entry, Features, Message};

const PLAIN: Features = Features::CHECKPOINT_V6.union(Features::INSPECT);
const NAMES: Features = PLAIN.union(Features::SESSION_NAMES);

fn create(client: &mut Client, name: &str) -> SessionId {
    let id = client
        .request(|req| Message::CreateSession {
            req,
            name: name.into(),
        })
        .expect("id");
    SessionId::from_uuid(id)
}

/// The name `list` reports for `session`.
fn name_of(client: &mut Client, session: SessionId) -> String {
    client
        .answer(|req| Message::List { req })
        .into_iter()
        .find_map(|m| match m {
            Message::Entry {
                entry: Entry::Session {
                    session: s, name, ..
                },
                ..
            } if s == session => Some(name),
            _ => None,
        })
        .expect("the session")
}

#[test]
fn an_empty_name_is_generated_and_unique() {
    let path = start_server();
    let mut client = Client::connect_with(&path, NAMES, None);
    let sessions: Vec<SessionId> = (0..20).map(|_| create(&mut client, "")).collect();
    let names: Vec<String> = sessions.iter().map(|&s| name_of(&mut client, s)).collect();
    for (i, name) in names.iter().enumerate() {
        assert!(is_generated(name), "{name}");
        assert!(!names[..i].contains(name), "{name} twice");
    }
    // A name given is kept.
    let chosen = create(&mut client, "work");
    assert_eq!(name_of(&mut client, chosen), "work");
}

#[test]
fn an_empty_name_stays_empty_for_a_client_without_the_bit() {
    let path = start_server();
    let mut client = Client::connect_with(&path, PLAIN, None);
    let session = create(&mut client, "");
    assert_eq!(name_of(&mut client, session), "");
}
