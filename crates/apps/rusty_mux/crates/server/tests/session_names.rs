//! Generated session names (C50): an empty name from a client with
//! `SESSION_NAMES` becomes one no other session has.

#![cfg(any(target_os = "linux", windows))]

mod common;

use common::{Client, start_server};
use rusty_mux_core::{EntityId, SessionId, is_generated};
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

/// The name and generated mark `list` reports for `session`.
fn listed(client: &mut Client, session: SessionId) -> (String, bool) {
    client
        .answer(|req| Message::List { req })
        .into_iter()
        .find_map(|m| match m {
            Message::Entry {
                entry:
                    Entry::Session {
                        session: s,
                        name,
                        generated,
                        ..
                    },
                ..
            } if s == session => Some((name, generated)),
            _ => None,
        })
        .expect("the session")
}

fn name_of(client: &mut Client, session: SessionId) -> String {
    listed(client, session).0
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

#[test]
fn the_generated_mark_lasts_until_a_rename() {
    let path = start_server();
    let mut client = Client::connect_with(&path, NAMES.union(Features::LABELS), None);
    let made = create(&mut client, "");
    let (name, generated) = listed(&mut client, made);
    assert!(generated, "{name}");
    // Chosen in the generated shape, it is still chosen.
    let chosen = create(&mut client, &format!("{name}-2"));
    assert!(!listed(&mut client, chosen).1);

    client.request(|req| Message::Rename {
        req,
        target: EntityId::Session(made),
        name: "fix-login".into(),
    });
    assert_eq!(listed(&mut client, made), ("fix-login".into(), false));
}

/// Sends `name-session` and returns the failure reason, if refused.
fn name_session(client: &mut Client, session: SessionId, name: &str) -> Option<String> {
    let answer = client.answer(|req| Message::NameSession {
        req,
        session,
        name: name.into(),
    });
    match answer.last() {
        Some(Message::Done { .. }) => None,
        Some(Message::Failed { reason, .. }) => Some(reason.clone()),
        other => panic!("unexpected answer {other:?}"),
    }
}

#[test]
fn a_stale_naming_attempt_keeps_a_name_chosen_after_it_was_issued() {
    let path = start_server();
    let both = NAMES.union(Features::LABELS).union(Features::NAME_SESSION);
    let mut person = Client::connect_with(&path, both, None);
    let mut agent = Client::connect_with(&path, both, None);
    let session = create(&mut person, "");
    // The hook saw a generated name and told the agent to name it later.
    assert!(listed(&mut agent, session).1);

    person.request(|req| Message::Rename {
        req,
        target: EntityId::Session(session),
        name: "release-notes".into(),
    });
    let refused = name_session(&mut agent, session, "fix-login").expect("refused");
    assert!(refused.contains("chosen name"), "{refused}");
    assert_eq!(
        listed(&mut person, session),
        ("release-notes".into(), false)
    );
}

#[test]
fn of_two_stale_naming_attempts_only_the_first_takes_effect() {
    let path = start_server();
    let features = NAMES.union(Features::NAME_SESSION);
    let mut first = Client::connect_with(&path, features, None);
    let mut second = Client::connect_with(&path, features, None);
    let session = create(&mut first, "");
    // Both agents' hooks saw the generated name.
    assert!(listed(&mut first, session).1);
    assert!(listed(&mut second, session).1);

    assert_eq!(name_session(&mut first, session, "fix-login"), None);
    assert!(name_session(&mut second, session, "write-docs").is_some());
    assert_eq!(listed(&mut second, session), ("fix-login".into(), false));
}

#[test]
fn name_session_needs_its_feature() {
    let path = start_server();
    let mut client = Client::connect_with(&path, NAMES, None);
    let session = create(&mut client, "");
    let refused = name_session(&mut client, session, "fix-login").expect("refused");
    assert!(refused.contains("requires feature"), "{refused}");
    assert!(listed(&mut client, session).1);
}
