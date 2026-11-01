//! Who each client is and how it is connected (C25, Rex delta D8): a
//! principal and a transport on every client entry, `whoami`, and
//! `ClientConnectionChanged` when a takeover moves a client.

#![cfg(any(target_os = "linux", windows))]

mod common;

use common::{Client, start_server};
use rusty_mux_core::{Event, Transport};
use rusty_mux_proto::{Entry, Features, Message, Takeover};

const IDENTITY: Features = Features::CHECKPOINT_V6
    .union(Features::EVENT_SEQ)
    .union(Features::INSPECT)
    .union(Features::LABELS)
    .union(Features::TAKEOVER)
    .union(Features::IDENTITY);
const PLAIN: Features = Features::CHECKPOINT_V6
    .union(Features::EVENT_SEQ)
    .union(Features::INSPECT)
    .union(Features::LABELS)
    .union(Features::TAKEOVER);

fn entries(answer: Vec<Message>) -> Vec<Entry> {
    answer
        .into_iter()
        .filter_map(|m| match m {
            Message::Entry { entry, .. } => Some(entry),
            _ => None,
        })
        .collect()
}

/// The principal the server must report: its own user, the only one a
/// personal server serves.
#[cfg(target_os = "linux")]
fn owner() -> String {
    format!("uid:{}", rusty_libc::process::geteuid())
}

#[cfg(windows)]
fn owner() -> String {
    "sid:".to_owned()
}

#[test]
fn whoami_names_the_caller_its_principal_and_transport() {
    let path = start_server();
    let mut me = Client::connect_with(&path, IDENTITY, None);
    let answer = entries(me.answer(|req| Message::Whoami { req }));
    let [
        Entry::Client {
            client,
            principal: Some(principal),
            transport,
            ..
        },
    ] = answer.as_slice()
    else {
        panic!("expected one client entry with a principal, got {answer:?}");
    };
    assert_eq!(*client, me.id());
    assert!(
        principal.id().starts_with(&owner()),
        "{principal:?} is not {}",
        owner()
    );
    assert_eq!(*transport, Some(Transport::Socket));
}

#[test]
fn every_listed_client_carries_its_principal() {
    let path = start_server();
    let mut me = Client::connect_with(&path, IDENTITY, None);
    let other = Client::connect_with(&path, PLAIN, None);
    let listed = entries(me.answer(|req| Message::List { req }));
    for id in [me.id(), other.id()] {
        let found = listed.iter().any(|e| {
            matches!(e, Entry::Client { client, principal: Some(p), transport: Some(Transport::Socket), .. }
                if *client == id && p.id().starts_with(&owner()))
        });
        assert!(
            found,
            "client {id} listed without its principal: {listed:?}"
        );
    }
}

#[test]
fn whoami_is_refused_without_the_feature() {
    let path = start_server();
    let mut plain = Client::connect_with(&path, PLAIN, None);
    assert!(!plain.features.contains(Features::IDENTITY));
    let answer = plain.answer(|req| Message::Whoami { req });
    assert!(
        matches!(answer.as_slice(), [Message::Failed { .. }]),
        "{answer:?}"
    );
}

/// A takeover is a new connection for the same client: watchers with the
/// feature learn it and how the client is now connected; others do not.
#[test]
fn a_takeover_tells_identity_watchers_the_connection_changed() {
    let path = start_server();
    let mut watcher = Client::connect_with(&path, IDENTITY, None);
    let mut plain = Client::connect_with(&path, PLAIN, None);
    let old = Client::connect_with(&path, IDENTITY, None);
    let token = Takeover {
        client: old.id(),
        token: old.token.expect("a token"),
    };
    let new = Client::taking_over(&path, IDENTITY, None, Some(token));
    assert_eq!(new.id(), old.id());
    let changed = |e: &Event| {
        matches!(e, Event::ClientConnectionChanged { client, principal, transport: Transport::Socket }
            if *client == old.id() && principal.id().starts_with(&owner()))
    };
    while !watcher.events.iter().any(|(_, e)| changed(e)) {
        watcher.recv();
    }
    // Its arrival, earlier, named its principal too.
    assert!(watcher.events.iter().any(|(_, e)| matches!(
        e,
        Event::ClientConnected { client, principal: Some(p) }
            if *client == old.id() && p.id().starts_with(&owner())
    )));
    // `plain` has seen everything before a Sync it asks for afterwards.
    plain.answer(|req| Message::Sync { req });
    assert!(
        !plain
            .events
            .iter()
            .any(|(_, e)| matches!(e, Event::ClientConnectionChanged { .. })),
        "a client without IDENTITY got {:?}",
        plain.events
    );
}
