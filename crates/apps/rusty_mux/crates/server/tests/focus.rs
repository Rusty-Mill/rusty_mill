//! Server-side focus (C41): each window's focused block, `focus-block`,
//! `move-focus`, the `FocusChanged` event and the window entry's focus.

#![cfg(any(target_os = "linux", windows))]

mod common;

use common::{Client, new_block, start_server, window_for};
use rusty_mux_core::{BlockId, Event, Side, Toward, WindowId};
use rusty_mux_proto::{Entry, Features, Message};

const PLAIN: Features = Features::CHECKPOINT_V6
    .union(Features::EVENT_SEQ)
    .union(Features::INSPECT)
    .union(Features::LAYOUT);
const FOCUS: Features = PLAIN.union(Features::FOCUS);

/// The window's focused block, as `list` reports it.
fn focused(client: &mut Client, window: WindowId) -> Option<BlockId> {
    client
        .answer(|req| Message::List { req })
        .into_iter()
        .find_map(|m| match m {
            Message::Entry {
                entry: Entry::Window {
                    window: w, focused, ..
                },
                ..
            } if w == window => Some(focused),
            _ => None,
        })
        .expect("the window")
}

fn wait_for_event(client: &mut Client, event: &Event) {
    while !client.events.iter().any(|(_, e)| e == event) {
        client.recv();
    }
}

#[test]
fn focus_moves_by_request_and_says_so() {
    let path = start_server();
    let mut client = Client::connect_with(&path, FOCUS, None);
    let window = window_for(&mut client);
    let a = new_block(&mut client, window);
    let b = new_block(&mut client, window);
    assert_eq!(focused(&mut client, window), Some(a), "the first block");
    wait_for_event(
        &mut client,
        &Event::FocusChanged {
            window,
            block: Some(a),
        },
    );

    client.request(|req| Message::MoveFocus {
        req,
        window,
        toward: Toward::Side(Side::Right),
    });
    assert_eq!(focused(&mut client, window), Some(b));
    client.request(|req| Message::FocusBlock { req, block: a });
    assert_eq!(focused(&mut client, window), Some(a));
    wait_for_event(
        &mut client,
        &Event::FocusChanged {
            window,
            block: Some(a),
        },
    );

    // Closing the focused block hands the focus to the one in its place.
    client.request(|req| Message::CloseBlock { req, block: a });
    assert_eq!(focused(&mut client, window), Some(b));

    let gone = BlockId::from_uuid(rusty_uuid::Uuid::new_v4());
    let answer = client.answer(|req| Message::FocusBlock { req, block: gone });
    assert!(
        matches!(answer.as_slice(), [Message::Failed { .. }]),
        "{answer:?}"
    );
}

#[test]
fn a_client_without_the_feature_can_not_focus_and_hears_nothing() {
    let path = start_server();
    let mut plain = Client::connect_with(&path, PLAIN, None);
    let window = window_for(&mut plain);
    let block = new_block(&mut plain, window);
    assert_eq!(
        focused(&mut plain, window),
        Some(block),
        "the listing carries it all the same"
    );
    let answer = plain.answer(|req| Message::FocusBlock { req, block });
    assert!(
        matches!(answer.as_slice(), [Message::Failed { reason, .. }] if reason.contains("feature")),
        "{answer:?}"
    );
    // Events reach clients in one order: once the plain client sees the
    // later block, it has missed the focus event for good.
    let later = new_block(&mut plain, window);
    wait_for_event(
        &mut plain,
        &Event::BlockCreated {
            block: later,
            window,
            size: rusty_mux_core::Size::new(80, 24).expect("size"),
        },
    );
    assert!(
        !plain
            .events
            .iter()
            .any(|(_, e)| matches!(e, Event::FocusChanged { .. }))
    );
}
