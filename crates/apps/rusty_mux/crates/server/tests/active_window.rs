//! Each session's active window (C38): `select-window`, the
//! `ActiveWindowChanged` event and the session entry's active window.

#![cfg(any(target_os = "linux", windows))]

mod common;

use common::{Client, start_server, window_for};
use rusty_mux_core::{Event, SessionId, WindowId};
use rusty_mux_proto::{Code, Entry, Features, Message};

const PLAIN: Features = Features::CHECKPOINT_V6
    .union(Features::EVENT_SEQ)
    .union(Features::INSPECT);
const ACTIVE: Features = PLAIN.union(Features::ACTIVE_WINDOW);

/// The session of `window` and its active window, as `list` reports them.
fn listed(client: &mut Client, window: WindowId) -> (SessionId, Option<WindowId>) {
    let answer = client.answer(|req| Message::List { req });
    let entries: Vec<Entry> = answer
        .into_iter()
        .filter_map(|m| match m {
            Message::Entry { entry, .. } => Some(entry),
            _ => None,
        })
        .collect();
    let session = entries
        .iter()
        .find_map(|e| match e {
            Entry::Window {
                window: w, session, ..
            } if *w == window => Some(*session),
            _ => None,
        })
        .expect("the window");
    let active = entries
        .iter()
        .find_map(|e| match e {
            Entry::Session {
                session: s, active, ..
            } if *s == session => Some(*active),
            _ => None,
        })
        .expect("the session");
    (session, active)
}

fn new_window(client: &mut Client, session: SessionId, name: &str) -> WindowId {
    let id = client
        .request(|req| Message::CreateWindow {
            req,
            session,
            name: name.into(),
        })
        .expect("id");
    WindowId::from_uuid(id)
}

fn wait_for_event(client: &mut Client, event: &Event) {
    while !client.events.iter().any(|(_, e)| e == event) {
        client.recv();
    }
}

#[test]
fn selecting_a_window_makes_it_active_and_says_so() {
    let path = start_server();
    let mut client = Client::connect_with(&path, ACTIVE, None);
    let first = window_for(&mut client);
    let (session, active) = listed(&mut client, first);
    assert_eq!(active, Some(first), "the first window is active");
    wait_for_event(
        &mut client,
        &Event::ActiveWindowChanged {
            session,
            window: Some(first),
        },
    );

    let second = new_window(&mut client, session, "bg");
    assert_eq!(
        listed(&mut client, first).1,
        Some(first),
        "a new window is not"
    );
    client.request(|req| Message::SelectWindow {
        req,
        window: second,
    });
    assert_eq!(listed(&mut client, first).1, Some(second));
    wait_for_event(
        &mut client,
        &Event::ActiveWindowChanged {
            session,
            window: Some(second),
        },
    );

    let gone = WindowId::from_uuid(rusty_uuid::Uuid::new_v4());
    let answer = client.answer(|req| Message::SelectWindow { req, window: gone });
    assert!(
        matches!(
            answer.as_slice(),
            [Message::Failed {
                code: Some(Code::NotFound),
                ..
            }]
        ),
        "{answer:?}"
    );
}

#[test]
fn a_client_without_the_feature_can_not_select_and_hears_nothing() {
    let path = start_server();
    let mut plain = Client::connect_with(&path, PLAIN, None);
    let mut watcher = Client::connect_with(&path, ACTIVE, None);
    let first = window_for(&mut plain);
    let (session, active) = listed(&mut plain, first);
    assert_eq!(active, Some(first), "the listing carries it all the same");
    let answer = plain.answer(|req| Message::SelectWindow { req, window: first });
    assert!(
        matches!(answer.as_slice(), [Message::Failed { reason, .. }] if reason.contains("feature")),
        "{answer:?}"
    );

    // Events reach clients in one order: once the plain client sees the
    // later window, it has missed the active-window event for good.
    let changed = Event::ActiveWindowChanged {
        session,
        window: Some(first),
    };
    wait_for_event(&mut watcher, &changed);
    let later = new_window(&mut plain, session, "later");
    wait_for_event(
        &mut plain,
        &Event::WindowCreated {
            window: later,
            session,
        },
    );
    assert!(
        !plain
            .events
            .iter()
            .any(|(_, e)| matches!(e, Event::ActiveWindowChanged { .. }))
    );
}

#[test]
fn replay_sends_the_event_only_to_clients_that_asked() {
    let path = start_server();
    let mut away = Vec::new();
    for features in [PLAIN, ACTIVE] {
        let client = Client::connect_with(&path, features, None);
        away.push((features, client.cursor.expect("cursor")));
    }

    // While both are away: a window becomes active, then a later event.
    let mut writer = Client::connect_with(&path, ACTIVE, None);
    let window = window_for(&mut writer);
    let (session, _) = listed(&mut writer, window);
    let changed = Event::ActiveWindowChanged {
        session,
        window: Some(window),
    };
    let later = new_window(&mut writer, session, "later");
    let created = Event::WindowCreated {
        window: later,
        session,
    };

    for (features, seen) in away {
        let mut client = Client::connect_with(&path, features, Some(seen));
        assert_eq!(client.cursor, Some(seen), "resumed");
        wait_for_event(&mut client, &created);
        let replayed = client.events.iter().any(|(_, e)| *e == changed);
        assert_eq!(replayed, features == ACTIVE, "{features:?}");
    }
}
