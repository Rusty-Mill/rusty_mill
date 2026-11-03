//! Window layouts: split, zoom, close and layout events (C12).

#![cfg(any(target_os = "linux", windows))]

mod common;

use common::{Client, block_for, new_block, start_server, window_for};
use rusty_mux_core::{BlockId, Event, Layout, Side, Size, WindowId};
use rusty_mux_proto::{Code, Entry, Features, Launch, Message};

const LAYOUT: Features = Features::CHECKPOINT_V6
    .union(Features::EVENT_SEQ)
    .union(Features::INSPECT)
    .union(Features::LAYOUT);
const PLAIN: Features = Features::CHECKPOINT_V6
    .union(Features::EVENT_SEQ)
    .union(Features::INSPECT);

fn split(block: BlockId, side: Side) -> impl FnOnce(u64) -> Message {
    move |req| Message::Split {
        req,
        block,
        side,
        launch: Launch::default(),
    }
}

/// The window's layout and zoom, as `list` reports them.
fn listed(client: &mut Client, window: WindowId) -> (Option<Layout>, Option<BlockId>) {
    client
        .answer(|req| Message::List { req })
        .into_iter()
        .find_map(|m| match m {
            Message::Entry {
                entry:
                    Entry::Window {
                        window: w,
                        layout,
                        zoomed,
                        ..
                    },
                ..
            } if w == window => Some((layout, zoomed)),
            _ => None,
        })
        .expect("the window")
}

fn size_of(client: &mut Client, block: BlockId) -> Size {
    client
        .answer(|req| Message::List { req })
        .into_iter()
        .find_map(|m| match m {
            Message::Entry {
                entry: Entry::Block { block: b, size, .. },
                ..
            } if b == block => Some(size),
            _ => None,
        })
        .expect("the block")
}

/// Reads until `event` arrives.
fn wait_for_event(client: &mut Client, event: &Event) {
    while !client.events.iter().any(|(_, e)| e == event) {
        client.recv();
    }
}

fn failed(answer: &[Message]) -> (&str, Option<Code>) {
    match answer {
        [Message::Failed { reason, code, .. }] => (reason, *code),
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[test]
fn a_split_places_a_new_block_beside_its_target() {
    let path = start_server();
    let mut client = Client::connect_with(&path, LAYOUT, None);
    let window = window_for(&mut client);
    let a = new_block(&mut client, window);
    let b = BlockId::from_uuid(client.request(split(a, Side::Right)).expect("an id"));
    // 80 columns: 40, a divider, 39 for the new block.
    assert_eq!(size_of(&mut client, b), Size::new(39, 24).unwrap());
    let expected: Layout = format!("cols50({a},{b})").parse().unwrap();
    wait_for_event(
        &mut client,
        &Event::LayoutChanged {
            window,
            layout: Some(expected.clone()),
            zoomed: None,
        },
    );
    let created = client
        .events
        .iter()
        .position(|(_, e)| matches!(e, Event::BlockCreated { block, .. } if *block == b));
    let laid_out = client.events.iter().position(
        |(_, e)| matches!(e, Event::LayoutChanged { layout: Some(l), .. } if *l == expected),
    );
    assert!(created < laid_out, "{:?}", client.events);
    assert_eq!(listed(&mut client, window), (Some(expected), None));

    // A block from create-block goes beside the largest block.
    let c = new_block(&mut client, window);
    let (Some(layout), _) = listed(&mut client, window) else {
        panic!("a layout");
    };
    assert_eq!(layout.blocks(), vec![a, c, b]);
}

#[test]
fn zoom_lasts_until_unzoom_split_or_close() {
    let path = start_server();
    let mut client = Client::connect_with(&path, LAYOUT, None);
    let window = window_for(&mut client);
    let a = new_block(&mut client, window);
    let b = BlockId::from_uuid(client.request(split(a, Side::Down)).expect("an id"));
    client.request(|req| Message::Zoom { req, block: b });
    assert_eq!(listed(&mut client, window).1, Some(b));
    client.request(|req| Message::Unzoom { req, window });
    assert_eq!(listed(&mut client, window).1, None);

    client.request(|req| Message::Zoom { req, block: a });
    let c = BlockId::from_uuid(client.request(split(b, Side::Left)).expect("an id"));
    assert_eq!(listed(&mut client, window).1, None, "a split unzooms");

    client.request(|req| Message::Zoom { req, block: c });
    client.request(|req| Message::CloseBlock { req, block: c });
    let expected: Layout = format!("rows50({a},{b})").parse().unwrap();
    assert_eq!(listed(&mut client, window), (Some(expected), None));
}

#[test]
fn refused_splits_and_zooms_say_why() {
    let path = start_server();
    let mut client = Client::connect_with(&path, LAYOUT, None);
    let block = block_for(&mut client);
    let size = Size::new(2, 24).unwrap();
    client.request(|req| Message::Resize { req, block, size });
    let answer = client.answer(split(block, Side::Right));
    assert!(failed(&answer).0.contains("too small"), "{answer:?}");
    // Stacked still fits.
    client.request(split(block, Side::Up));

    let gone = BlockId::from_uuid(rusty_uuid::Uuid::new_v4());
    let answer = client.answer(split(gone, Side::Down));
    assert_eq!(failed(&answer).1, Some(Code::NotFound));
    let answer = client.answer(|req| Message::Zoom { req, block: gone });
    assert_eq!(failed(&answer).1, Some(Code::NotFound));
}

#[test]
fn a_client_without_the_feature_gets_no_layout_events_or_requests() {
    let path = start_server();
    let mut plain = Client::connect_with(&path, PLAIN, None);
    let window = window_for(&mut plain);
    let block = new_block(&mut plain, window);
    for answer in [
        plain.answer(split(block, Side::Right)),
        plain.answer(|req| Message::Zoom { req, block }),
        plain.answer(|req| Message::Unzoom { req, window }),
    ] {
        assert!(failed(&answer).0.contains("feature"), "{answer:?}");
    }
    // The listing carries the layout all the same.
    assert_eq!(listed(&mut plain, window).0, Some(Layout::Block(block)));
    plain.request(|req| Message::CloseBlock { req, block });
    wait_for_event(&mut plain, &Event::BlockClosed { block });
    plain.request(|req| Message::List { req });
    assert!(
        !plain
            .events
            .iter()
            .any(|(_, e)| matches!(e, Event::LayoutChanged { .. })),
        "{:?}",
        plain.events
    );
}
