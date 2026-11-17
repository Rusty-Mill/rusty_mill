//! Sessions across client windows (C64): a client asks another client of
//! its account, or every other one, to show a session. The server only
//! relays: the ask is a message, never an event, so a reconnect never
//! repeats it. Another account's clients are refused in the model's
//! principal check, which one test process can't set up (see `shared.rs`).

#![cfg(target_os = "linux")]

mod common;

use common::{Client, start_server};
use rusty_mux_core::{ClientId, SessionId};
use rusty_mux_proto::{Entry, Features, Message, ShowTo};

const SHOWS: Features = Features::CHECKPOINT_V6
    .union(Features::EVENT_SEQ)
    .union(Features::INSPECT)
    .union(Features::LABELS)
    .union(Features::SHOW_SESSION);

fn session_of(client: &mut Client) -> SessionId {
    let session = SessionId::from_uuid(
        client
            .request(|req| Message::CreateSession {
                req,
                name: "work".into(),
            })
            .expect("id"),
    );
    client.request(|req| Message::CreateWindow {
        req,
        session,
        name: "w".into(),
    });
    session
}

/// Reads until a [`Message::SessionShown`] arrives.
fn shown(client: &mut Client) -> (SessionId, ClientId) {
    loop {
        if let Message::SessionShown { session, by } = client.recv() {
            return (session, by);
        }
    }
}

/// A request's answer: `Ok` for Done, the reason for Failed.
fn ask(client: &mut Client, session: SessionId, to: ShowTo) -> Result<(), String> {
    let answer = client.answer(|req| Message::ShowSession { req, session, to });
    match answer.last() {
        Some(Message::Done { .. }) => Ok(()),
        Some(Message::Failed { reason, .. }) => Err(reason.clone()),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn one_window_is_asked_and_the_others_are_not() {
    let path = start_server();
    let mut a = Client::connect_with(&path, SHOWS, None);
    let mut b = Client::connect_with(&path, SHOWS, None);
    let mut c = Client::connect_with(&path, SHOWS, None);
    let session = session_of(&mut a);
    ask(&mut a, session, ShowTo::Client(b.id())).expect("done");
    assert_eq!(shown(&mut b), (session, a.id()));
    // Nothing reached c: its next message is its own answer.
    ask(&mut c, session, ShowTo::Client(a.id())).expect("done");
    assert_eq!(shown(&mut a), (session, c.id()));
}

#[test]
fn every_other_window_is_asked_but_not_the_sender() {
    let path = start_server();
    let mut a = Client::connect_with(&path, SHOWS, None);
    let mut b = Client::connect_with(&path, SHOWS, None);
    let mut c = Client::connect_with(&path, SHOWS, None);
    let session = session_of(&mut a);
    ask(&mut a, session, ShowTo::Others).expect("done");
    assert_eq!(shown(&mut b), (session, a.id()));
    assert_eq!(shown(&mut c), (session, a.id()));
    // The sender hears nothing: it is Done first and then an ask from b.
    ask(&mut b, session, ShowTo::Client(a.id())).expect("done");
    assert_eq!(shown(&mut a), (session, b.id()));
}

#[test]
fn with_nobody_else_it_is_still_done() {
    let path = start_server();
    let mut a = Client::connect_with(&path, SHOWS, None);
    let session = session_of(&mut a);
    ask(&mut a, session, ShowTo::Others).expect("done");
}

#[test]
fn refused_for_an_unknown_session_oneself_or_a_client_that_cannot_show() {
    let path = start_server();
    let mut a = Client::connect_with(&path, SHOWS, None);
    let plain = Client::connect_with(&path, Features::CHECKPOINT_V6, None);
    let session = session_of(&mut a);
    let unknown = SessionId::from_uuid(rusty_uuid::Uuid::new_v4());
    assert!(ask(&mut a, unknown, ShowTo::Others).is_err());
    let own = a.id();
    assert!(ask(&mut a, session, ShowTo::Client(own)).is_err());
    assert!(ask(&mut a, session, ShowTo::Client(plain.id())).is_err());
    // A client without the bit can't ask either.
    let mut old = Client::connect_with(&path, Features::CHECKPOINT_V6, None);
    assert!(ask(&mut old, session, ShowTo::Others).is_err());
}

#[test]
fn asks_are_rate_limited_per_sender() {
    let path = start_server();
    let mut a = Client::connect_with(&path, SHOWS, None);
    let mut b = Client::connect_with(&path, SHOWS, None);
    let session = session_of(&mut a);
    let target = b.id();
    let results: Vec<_> = (0..8)
        .map(|_| ask(&mut a, session, ShowTo::Client(target)))
        .collect();
    let allowed = results.iter().filter(|r| r.is_ok()).count();
    assert!((4..8).contains(&allowed), "{results:?}");
    assert!(results.last().expect("last").is_err());
    // b still hears exactly the allowed ones.
    for _ in 0..4 {
        shown(&mut b);
    }
}

#[test]
fn a_listing_says_which_clients_can_show_sessions() {
    let path = start_server();
    let mut a = Client::connect_with(&path, SHOWS, None);
    let plain = Client::connect_with(
        &path,
        Features::CHECKPOINT_V6
            .union(Features::INSPECT)
            .union(Features::LABELS),
        None,
    );
    let mut showable = Vec::new();
    for message in a.answer(|req| Message::List { req }) {
        if let Message::Entry {
            entry:
                Entry::Client {
                    client,
                    showable: s,
                    ..
                },
            ..
        } = message
        {
            showable.push((client, s));
        }
    }
    assert!(showable.contains(&(a.id(), Some(true))), "{showable:?}");
    assert!(showable.contains(&(plain.id(), Some(false))));
}

#[test]
fn a_reconnect_never_repeats_an_ask() {
    let path = start_server();
    let mut a = Client::connect_with(&path, SHOWS, None);
    let mut b = Client::connect_with(&path, SHOWS, None);
    let session = session_of(&mut a);
    ask(&mut a, session, ShowTo::Client(b.id())).expect("done");
    shown(&mut b);
    let cursor = b.cursor;
    drop(b);
    let mut again = Client::connect_with(&path, SHOWS, cursor);
    // Whatever replays, no SessionShown arrives before the next answer.
    let answer = again.answer(|req| Message::List { req });
    assert!(
        answer
            .iter()
            .all(|m| !matches!(m, Message::SessionShown { .. }))
    );
}
