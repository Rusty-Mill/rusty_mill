//! Names on every kind, the client list and force-disconnect (C33).

#![cfg(any(target_os = "linux", windows))]

mod common;

use common::{Client, block_for, new_block, start_server, window_for};
use rusty_mux_core::{BlockId, ClientId, EntityId, Event, MAX_NAME, SessionId};
use rusty_mux_proto::{Code, Entry, Features, Message};

const LABELS: Features = Features::CHECKPOINT_V5
    .union(Features::EVENT_SEQ)
    .union(Features::INSPECT)
    .union(Features::LABELS);
const PLAIN: Features = Features::CHECKPOINT_V5
    .union(Features::EVENT_SEQ)
    .union(Features::INSPECT);

fn rename(target: EntityId, name: &str) -> impl FnOnce(u64) -> Message {
    let name = name.to_owned();
    move |req| Message::Rename { req, target, name }
}

fn entries(client: &mut Client) -> Vec<Entry> {
    client
        .answer(|req| Message::List { req })
        .into_iter()
        .filter_map(|m| match m {
            Message::Entry { entry, .. } => Some(entry),
            _ => None,
        })
        .collect()
}

fn failed_code(answer: &[Message]) -> Option<Code> {
    match answer {
        [Message::Failed { code, .. }] => *code,
        other => panic!("expected Failed, got {other:?}"),
    }
}

/// Reads until `event` arrives.
fn wait_for_event(client: &mut Client, event: &Event) {
    while !client.events.iter().any(|(_, e)| e == event) {
        client.recv();
    }
}

#[test]
fn renames_reach_the_listing_and_only_clients_that_asked() {
    let path = start_server();
    let mut namer = Client::connect_with(&path, LABELS, None);
    let mut plain = Client::connect_with(&path, PLAIN, None);
    let window = window_for(&mut namer);
    let block = new_block(&mut namer, window);
    let session = entries(&mut namer)
        .iter()
        .find_map(|e| match e {
            Entry::Session { session, .. } => Some(*session),
            _ => None,
        })
        .expect("a session");
    let renames = [
        (EntityId::Session(session), "work"),
        (EntityId::Window(window), "logs"),
        (EntityId::Block(block), "build"),
        (EntityId::Client(plain.id()), "laptop"),
    ];
    for (target, name) in renames {
        namer.request(rename(target, name));
    }
    wait_for_event(
        &mut namer,
        &Event::ClientRenamed {
            client: plain.id(),
            name: "laptop".into(),
        },
    );
    let renamed: Vec<&Event> = namer
        .events
        .iter()
        .map(|(_, e)| e)
        .filter(|e| {
            matches!(
                e,
                Event::SessionRenamed { .. }
                    | Event::WindowRenamed { .. }
                    | Event::BlockRenamed { .. }
                    | Event::ClientRenamed { .. }
            )
        })
        .collect();
    assert_eq!(renamed.len(), 4, "{renamed:?}");

    let listed = entries(&mut namer);
    assert!(
        listed
            .iter()
            .any(|e| matches!(e, Entry::Session { name, .. } if name == "work"))
    );
    assert!(
        listed
            .iter()
            .any(|e| matches!(e, Entry::Window { name, .. } if name == "logs"))
    );
    assert!(listed.iter().any(
        |e| matches!(e, Entry::Block { block: b, name, .. } if *b == block && name == "build")
    ));
    let clients: Vec<(ClientId, &str)> = listed
        .iter()
        .filter_map(|e| match e {
            Entry::Client { client, name, .. } => Some((*client, name.as_str())),
            _ => None,
        })
        .collect();
    assert_eq!(clients.len(), 2, "{clients:?}");
    assert!(clients.contains(&(plain.id(), "laptop")));
    assert!(clients.contains(&(namer.id(), "")));

    // Without the feature: names on blocks, but no clients and no events.
    let plain_list = entries(&mut plain);
    assert!(!plain_list.iter().any(|e| matches!(e, Entry::Client { .. })));
    assert!(
        plain_list
            .iter()
            .any(|e| matches!(e, Entry::Block { name, .. } if name == "build"))
    );
    assert!(!plain.events.iter().any(|(_, e)| matches!(
        e,
        Event::SessionRenamed { .. }
            | Event::WindowRenamed { .. }
            | Event::BlockRenamed { .. }
            | Event::ClientRenamed { .. }
    )));
    let refused = plain.answer(rename(EntityId::Block(block), "x"));
    assert_eq!(failed_code(&refused), None);
}

#[test]
fn renaming_a_missing_entity_is_refused_as_not_found() {
    let path = start_server();
    let mut client = Client::connect_with(&path, LABELS, None);
    let missing = rusty_uuid::Uuid::new_v4();
    for target in [
        EntityId::Session(SessionId::from_uuid(missing)),
        EntityId::Block(BlockId::from_uuid(missing)),
        EntityId::Client(ClientId::from_uuid(missing)),
    ] {
        let answer = client.answer(rename(target, "x"));
        assert_eq!(failed_code(&answer), Some(Code::NotFound), "{target:?}");
    }
}

#[test]
fn a_disconnected_client_loses_its_lease_and_its_connection() {
    let path = start_server();
    let mut holder = Client::connect_with(&path, LABELS, None);
    let block = block_for(&mut holder);
    let mut admin = Client::connect_with(&path, LABELS, None);
    let target = holder.id();

    // Not itself, and not a client that is not there.
    let own = admin.id();
    let answer = admin.answer(|req| Message::Disconnect { req, client: own });
    assert_eq!(failed_code(&answer), None);
    let missing = ClientId::from_uuid(rusty_uuid::Uuid::new_v4());
    let answer = admin.answer(|req| Message::Disconnect {
        req,
        client: missing,
    });
    assert_eq!(failed_code(&answer), Some(Code::NotFound));

    admin.request(|req| Message::Disconnect {
        req,
        client: target,
    });
    holder.wait_closed();
    wait_for_event(&mut admin, &Event::ClientDisconnected { client: target });
    let freed = Event::LeaseChanged {
        block,
        holder: None,
    };
    assert!(admin.events.iter().any(|(_, e)| *e == freed));
    assert!(
        !entries(&mut admin)
            .iter()
            .any(|e| matches!(e, Entry::Client { client, .. } if *client == target))
    );
    // The lease is free for another client.
    admin.request(|req| Message::Attach { req, block });
    admin.request(|req| Message::AcquireLease { req, block });
}

#[test]
fn disconnecting_needs_the_feature() {
    let path = start_server();
    let mut victim = Client::connect_with(&path, LABELS, None);
    let mut plain = Client::connect_with(&path, PLAIN, None);
    let target = victim.id();
    let answer = plain.answer(|req| Message::Disconnect {
        req,
        client: target,
    });
    assert_eq!(failed_code(&answer), None);
    assert!(matches!(
        victim.answer(|req| Message::List { req }).last(),
        Some(Message::Done { .. })
    ));
}

#[test]
fn an_oversized_name_is_refused_and_changes_nothing() {
    let path = start_server();
    let mut holder = Client::connect_with(&path, LABELS, None);
    let block = block_for(&mut holder);
    let mut watcher = Client::connect_with(&path, LABELS, None);
    holder.request(rename(EntityId::Block(block), "build"));

    let over = "x".repeat(MAX_NAME + 1);
    for target in [EntityId::Block(block), EntityId::Client(watcher.id())] {
        let answer = holder.answer(rename(target, &over));
        assert_eq!(failed_code(&answer), None, "{target:?}");
    }
    let answer = holder.answer(|req| Message::CreateSession {
        req,
        name: over.clone(),
    });
    assert_eq!(failed_code(&answer), None);

    // Both clients are still connected, the lease kept and the name too.
    let listed = entries(&mut watcher);
    assert!(listed.iter().any(|e| matches!(
        e,
        Entry::Block { block: b, name, holder: Some(h), .. }
            if *b == block && name == "build" && *h == holder.id()
    )));
    assert_eq!(
        listed
            .iter()
            .filter(|e| matches!(e, Entry::Client { .. }))
            .count(),
        2
    );
    assert!(!watcher.events.iter().any(|(_, e)| matches!(
        e,
        Event::SessionCreated { .. } | Event::ClientRenamed { .. }
    )));
    // Exactly the limit is accepted.
    holder.request(rename(EntityId::Block(block), &"x".repeat(MAX_NAME)));
}
