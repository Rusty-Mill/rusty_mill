//! Layout editing (C41): resize a split, swap and move blocks, detach a
//! block from the layout while it runs, and reorder windows.

#![cfg(any(target_os = "linux", windows))]

mod common;

use common::{Client, new_block, start_server, window_for};
use rusty_mux_core::{Axis, BlockId, Event, Layout, SessionId, Share, Side, WindowId};
use rusty_mux_proto::{Code, Entry, Features, Message};

const PLAIN: Features = Features::CHECKPOINT_V6
    .union(Features::EVENT_SEQ)
    .union(Features::INSPECT)
    .union(Features::LAYOUT);
const EDIT: Features = PLAIN.union(Features::LAYOUT_EDIT);

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

/// The window's layout as text, as `list` reports it.
fn layout_text(client: &mut Client, window: WindowId) -> String {
    entries(client)
        .into_iter()
        .find_map(|e| match e {
            Entry::Window {
                window: w, layout, ..
            } if w == window => Some(layout.as_ref().map(Layout::to_string)),
            _ => None,
        })
        .expect("the window")
        .unwrap_or_default()
}

/// The session's windows in their order, as `list` reports them.
fn windows_of(client: &mut Client, session: SessionId) -> Vec<WindowId> {
    entries(client)
        .into_iter()
        .filter_map(|e| match e {
            Entry::Window {
                window, session: s, ..
            } if s == session => Some(window),
            _ => None,
        })
        .collect()
}

fn listed_block(client: &mut Client, block: BlockId) -> bool {
    entries(client)
        .iter()
        .any(|e| matches!(e, Entry::Block { block: b, .. } if *b == block))
}

fn session_of(client: &mut Client, window: WindowId) -> SessionId {
    entries(client)
        .into_iter()
        .find_map(|e| match e {
            Entry::Window {
                window: w, session, ..
            } if w == window => Some(session),
            _ => None,
        })
        .expect("the window")
}

fn refused(answer: &[Message], code: Option<Code>) -> bool {
    matches!(answer, [Message::Failed { code: c, .. }] if *c == code)
}

#[test]
fn blocks_resize_swap_move_and_detach() {
    let path = start_server();
    let mut client = Client::connect_with(&path, EDIT, None);
    let window = window_for(&mut client);
    // New blocks split the largest: a | b, then c below a.
    let a = new_block(&mut client, window);
    let b = new_block(&mut client, window);
    client.request(|req| Message::MoveBlock {
        req,
        block: b,
        beside: Some(a),
        side: Side::Right,
    });
    let c = new_block(&mut client, window);
    client.request(|req| Message::MoveBlock {
        req,
        block: c,
        beside: Some(a),
        side: Side::Down,
    });
    assert_eq!(
        layout_text(&mut client, window),
        format!("cols50(rows50({a},{c}),{b})")
    );

    client.request(|req| Message::ResizeSplit {
        req,
        block: b,
        axis: Axis::Columns,
        share: Share::new(30).expect("in range"),
    });
    assert_eq!(
        layout_text(&mut client, window),
        format!("cols70(rows50({a},{c}),{b})")
    );

    client.request(|req| Message::SwapBlocks {
        req,
        block: a,
        other: b,
    });
    assert_eq!(
        layout_text(&mut client, window),
        format!("cols70(rows50({b},{c}),{a})")
    );

    client.request(|req| Message::DetachBlock { req, block: c });
    let detached = format!("cols70({b},{a})");
    assert_eq!(layout_text(&mut client, window), detached);
    assert!(listed_block(&mut client, c), "a detached block runs on");
    let event = Event::LayoutChanged {
        window,
        layout: Some(detached.parse().expect("layout")),
        zoomed: None,
    };
    while !client.events.iter().any(|(_, e)| *e == event) {
        client.recv();
    }
    // Only a block in the layout can be shown alone.
    let answer = client.answer(|req| Message::Zoom { req, block: c });
    assert!(refused(&answer, None), "{answer:?}");
    // A detached block can be attached to, as any other.
    client.request(|req| Message::Attach { req, block: c });

    // Nothing goes beside a detached block; it comes back beside another.
    let answer = client.answer(|req| Message::MoveBlock {
        req,
        block: a,
        beside: Some(c),
        side: Side::Left,
    });
    assert!(refused(&answer, None), "{answer:?}");
    client.request(|req| Message::MoveBlock {
        req,
        block: c,
        beside: Some(a),
        side: Side::Up,
    });
    assert_eq!(
        layout_text(&mut client, window),
        format!("cols70({b},rows50({c},{a}))")
    );

    let answer = client.answer(|req| Message::ResizeSplit {
        req,
        block: b,
        axis: Axis::Rows,
        share: Share::HALF,
    });
    assert!(refused(&answer, None), "{answer:?}");
    let gone = BlockId::from_uuid(rusty_uuid::Uuid::new_v4());
    let answer = client.answer(|req| Message::DetachBlock { req, block: gone });
    assert!(refused(&answer, Some(Code::NotFound)), "{answer:?}");
}

#[test]
fn windows_move_within_their_session() {
    let path = start_server();
    let mut client = Client::connect_with(&path, EDIT, None);
    let first = window_for(&mut client);
    let session = session_of(&mut client, first);
    let second = WindowId::from_uuid(
        client
            .request(|req| Message::CreateWindow {
                req,
                session,
                name: "2".into(),
            })
            .expect("id"),
    );
    client.request(|req| Message::MoveWindow {
        req,
        window: second,
        index: 0,
    });
    assert_eq!(windows_of(&mut client, session), vec![second, first]);
    let moved = Event::WindowMoved {
        window: second,
        index: 0,
    };
    while !client.events.iter().any(|(_, e)| *e == moved) {
        client.recv();
    }
    let answer = client.answer(|req| Message::MoveWindow {
        req,
        window: first,
        index: 2,
    });
    assert!(refused(&answer, None), "{answer:?}");
}

#[test]
fn a_client_without_the_feature_can_not_edit_and_hears_no_moves() {
    let path = start_server();
    let mut plain = Client::connect_with(&path, PLAIN, None);
    let mut editor = Client::connect_with(&path, EDIT, None);
    let first = window_for(&mut plain);
    let block = new_block(&mut plain, first);
    let answer = plain.answer(|req| Message::DetachBlock { req, block });
    assert!(
        matches!(answer.as_slice(), [Message::Failed { reason, .. }] if reason.contains("feature")),
        "{answer:?}"
    );
    let session = session_of(&mut plain, first);
    let second = WindowId::from_uuid(
        plain
            .request(|req| Message::CreateWindow {
                req,
                session,
                name: "2".into(),
            })
            .expect("id"),
    );
    editor.request(|req| Message::MoveWindow {
        req,
        window: second,
        index: 0,
    });
    // Events reach clients in one order: once the plain client sees the
    // later window, it has missed the move for good.
    let later = WindowId::from_uuid(
        editor
            .request(|req| Message::CreateWindow {
                req,
                session,
                name: "3".into(),
            })
            .expect("id"),
    );
    let created = Event::WindowCreated {
        window: later,
        session,
    };
    while !plain.events.iter().any(|(_, e)| *e == created) {
        plain.recv();
    }
    assert!(
        !plain
            .events
            .iter()
            .any(|(_, e)| matches!(e, Event::WindowMoved { .. }))
    );
}
