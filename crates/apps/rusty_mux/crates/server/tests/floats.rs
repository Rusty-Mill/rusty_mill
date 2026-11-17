//! Floating layers (C42): float a block over its window, move, resize and
//! stack it, and list it; a client without the feature sees none of it.

#![cfg(any(target_os = "linux", windows))]

mod common;

use common::{Client, new_block, start_server, window_for};
use rusty_mux_core::Event;
use rusty_mux_core::{Float, FloatRect, WindowId};
use rusty_mux_proto::{Entry, Features, Message};

const PLAIN: Features = Features::CHECKPOINT_V6
    .union(Features::EVENT_SEQ)
    .union(Features::INSPECT)
    .union(Features::LAYOUT)
    .union(Features::LAYOUT_EDIT);
const FLOATING: Features = PLAIN.union(Features::FLOATS);

fn rect(x: u8, y: u8, w: u8, h: u8) -> FloatRect {
    FloatRect::new(x, y, w, h).expect("a float rect")
}

fn listed(client: &mut Client, window: WindowId) -> Vec<Float> {
    client
        .answer(|req| Message::List { req })
        .into_iter()
        .find_map(|m| match m {
            Message::Entry {
                entry: Entry::Floats { window: w, floats },
                ..
            } if w == window => Some(floats),
            _ => None,
        })
        .unwrap_or_default()
}

#[test]
fn a_block_floats_moves_stacks_and_is_listed() {
    let path = start_server();
    let mut client = Client::connect_with(&path, FLOATING, None);
    let window = window_for(&mut client);
    let a = new_block(&mut client, window);
    let b = new_block(&mut client, window);
    for (block, at) in [(a, rect(5, 5, 40, 40)), (b, rect(30, 30, 50, 50))] {
        client.request(|req| Message::PlaceFloat { req, block, at });
    }
    let order = |floats: &[Float]| floats.iter().map(|f| f.block).collect::<Vec<_>>();
    assert_eq!(order(&listed(&mut client, window)), vec![a, b]);
    client.request(|req| Message::StackFloat {
        req,
        block: a,
        top: true,
    });
    client.request(|req| Message::PlaceFloat {
        req,
        block: a,
        at: rect(0, 0, 20, 20),
    });
    let floats = listed(&mut client, window);
    assert_eq!(order(&floats), vec![b, a]);
    assert_eq!(floats[1].at, rect(0, 0, 20, 20));
    assert!(client.events.iter().any(
        |(_, e)| matches!(e, Event::FloatsChanged { window: w, floats } if *w == window && floats.len() == 2)
    ));
    // Detaching ends the float; stacking what does not float is refused.
    client.request(|req| Message::DetachBlock { req, block: a });
    assert_eq!(order(&listed(&mut client, window)), vec![b]);
    let answer = client.answer(|req| Message::StackFloat {
        req,
        block: a,
        top: false,
    });
    assert!(
        matches!(answer.as_slice(), [Message::Failed { .. }]),
        "{answer:?}"
    );
}

#[test]
fn a_client_without_the_feature_can_not_float_and_hears_none() {
    let path = start_server();
    let mut plain = Client::connect_with(&path, PLAIN, None);
    let mut floating = Client::connect_with(&path, FLOATING, None);
    let window = window_for(&mut plain);
    let block = new_block(&mut plain, window);
    let answer = plain.answer(|req| Message::PlaceFloat {
        req,
        block,
        at: rect(0, 0, 50, 50),
    });
    assert!(
        matches!(answer.as_slice(), [Message::Failed { reason, .. }] if reason.contains("feature")),
        "{answer:?}"
    );
    floating.request(|req| Message::PlaceFloat {
        req,
        block,
        at: rect(0, 0, 50, 50),
    });
    assert!(listed(&mut plain, window).is_empty());
    assert!(
        plain
            .events
            .iter()
            .all(|(_, e)| !matches!(e, Event::FloatsChanged { .. }))
    );
    assert_eq!(listed(&mut floating, window).len(), 1);
}

#[test]
fn floating_needs_layout_editing_too() {
    let path = start_server();
    let mut client = Client::connect_with(&path, PLAIN.union(Features::FLOATS), None);
    let mut no_edit = Client::connect_with(
        &path,
        Features::from_bits(FLOATING.bits() & !Features::LAYOUT_EDIT.bits()),
        None,
    );
    let window = window_for(&mut client);
    let block = new_block(&mut client, window);
    let answer = no_edit.answer(|req| Message::PlaceFloat {
        req,
        block,
        at: rect(0, 0, 50, 50),
    });
    assert!(
    // The refusal names both bits, so it is the layout one that is missing.
    let both = format!("{:#x}", Features::FLOATS.union(Features::LAYOUT_EDIT).bits());
    assert!(
        matches!(answer.as_slice(), [Message::Failed { reason, .. }] if reason.contains(&both)),
        "{answer:?}"
    );
}

#[test]
fn a_client_without_the_bit_is_never_told_focus_is_on_a_float() {
    let path = start_server();
    let mut plain = Client::connect_with(&path, PLAIN.union(Features::FOCUS), None);
    let mut floating = Client::connect_with(&path, FLOATING.union(Features::FOCUS), None);
    let window = window_for(&mut plain);
    let tile = new_block(&mut plain, window);
    let float = new_block(&mut plain, window);
    floating.request(|req| Message::PlaceFloat {
        req,
        block: float,
        at: rect(10, 10, 40, 40),
    });
    floating.request(|req| Message::FocusBlock { req, block: float });
    let listed_focus = |client: &mut Client| {
        client
            .answer(|req| Message::List { req })
            .into_iter()
            .find_map(|m| match m {
                Message::Entry {
                    entry: Entry::Window { focused, .. },
                    ..
                } => Some(focused),
                _ => None,
            })
            .expect("the window")
    };
    assert_eq!(listed_focus(&mut plain), Some(tile));
    assert_eq!(listed_focus(&mut floating), Some(float));
    let to_float = |(_, e): &(_, Event)| matches!(e, Event::FocusChanged { block: Some(b), .. } if *b == float);
    assert!(!plain.events.iter().any(to_float));
    assert!(floating.events.iter().any(to_float));
}
