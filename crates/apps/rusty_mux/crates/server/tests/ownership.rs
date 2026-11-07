//! Block ownership (C46): the client that launches a block owned is the
//! only one that may change it; a peer of the same account gains nothing;
//! an owner gone for good leaves the block orphaned.

#![cfg(any(target_os = "linux", windows))]

mod common;

use common::{Client, start_server, window_for};
use rusty_mux_core::{Axis, BlockId, ClientId, EntityId, Share, Side, Size, WindowId};
use rusty_mux_proto::{Entry, Features, Launch, Message, Ownership, Takeover};

const BASE: Features = Features::CHECKPOINT_V6
    .union(Features::EVENT_SEQ)
    .union(Features::INSPECT)
    .union(Features::LABELS)
    .union(Features::LAUNCH)
    .union(Features::LAYOUT)
    .union(Features::LAYOUT_EDIT)
    .union(Features::FOCUS);
const OWNER: Features = BASE.union(Features::OWNERSHIP);
const RECONNECTING_OWNER: Features = OWNER.union(Features::TAKEOVER);

fn owned() -> Launch {
    Launch {
        own: true,
        ..Launch::default()
    }
}

fn launch_in(client: &mut Client, window: WindowId, launch: Launch) -> BlockId {
    let size = Size::new(80, 24).expect("size");
    let id = client.request(|req| Message::CreateBlock {
        req,
        window,
        size,
        launch,
    });
    BlockId::from_uuid(id.expect("a block id"))
}

fn owner_of(client: &mut Client, block: BlockId) -> Option<Ownership> {
    client
        .answer(|req| Message::List { req })
        .into_iter()
        .find_map(|m| match m {
            Message::Entry {
                entry: Entry::Block {
                    block: b, owner, ..
                },
                ..
            } if b == block => Some(owner),
            _ => None,
        })
        .expect("the block is listed")
}

fn refused(client: &mut Client, make: impl FnOnce(u64) -> Message) -> String {
    match client.answer(make).as_slice() {
        [Message::Failed { reason, .. }] => reason.clone(),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn succeeds(client: &mut Client, make: impl FnOnce(u64) -> Message) {
    match client.answer(make).as_slice() {
        [Message::Done { .. }] => {}
        other => panic!("expected success, got {other:?}"),
    }
}

/// Lists until `block`'s owner is `want`: a dropped connection is seen by
/// the server a moment later.
fn wait_owner(client: &mut Client, block: BlockId, want: Option<Ownership>) {
    let deadline = std::time::Instant::now() + common::PATIENCE;
    while owner_of(client, block) != want {
        assert!(
            std::time::Instant::now() < deadline,
            "owner never became {want:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn only_the_owner_may_change_an_owned_block() {
    let path = start_server();
    let mut owner = Client::connect_with(&path, OWNER, None);
    let mut peer = Client::connect_with(&path, OWNER, None);
    let window = window_for(&mut owner);
    let block = launch_in(&mut owner, window, owned());
    let other = launch_in(&mut peer, window, Launch::default());
    assert_eq!(
        owner_of(&mut peer, block),
        Some(Ownership::Client(owner.id()))
    );
    assert_eq!(owner_of(&mut peer, other), None);

    let target = EntityId::Block(block);
    let owner_id: ClientId = owner.id();
    let attempts: Vec<Box<dyn FnOnce(u64) -> Message>> = vec![
        Box::new(move |req| Message::Rename {
            req,
            target,
            name: "mine".into(),
        }),
        Box::new(move |req| Message::CloseBlock { req, block }),
        Box::new(move |req| Message::Split {
            req,
            block,
            side: Side::Right,
            launch: Launch::default(),
        }),
        Box::new(move |req| Message::ResizeSplit {
            req,
            block,
            axis: Axis::Columns,
            share: Share::new(30).expect("share"),
        }),
        Box::new(move |req| Message::SwapBlocks {
            req,
            block: other,
            other: block,
        }),
        Box::new(move |req| Message::MoveBlock {
            req,
            block: other,
            beside: Some(block),
            side: Side::Down,
        }),
        Box::new(move |req| Message::DetachBlock { req, block }),
        Box::new(move |req| Message::Disconnect {
            req,
            client: owner_id,
        }),
    ];
    for attempt in attempts {
        let reason = refused(&mut peer, attempt);
        assert!(reason.contains("own"), "{reason}");
    }
    // Looking is open to everyone.
    succeeds(&mut peer, |req| Message::Zoom { req, block });
    succeeds(&mut peer, |req| Message::FocusBlock { req, block });
    succeeds(&mut peer, |req| Message::Attach { req, block });

    succeeds(&mut owner, |req| Message::Rename {
        req,
        target,
        name: "agent".into(),
    });
    succeeds(&mut owner, |req| Message::SwapBlocks { req, block, other });
    succeeds(&mut owner, |req| Message::CloseBlock { req, block });
}

#[test]
fn an_owned_launch_needs_the_feature() {
    let path = start_server();
    let mut plain = Client::connect_with(&path, BASE, None);
    let window = window_for(&mut plain);
    let size = Size::new(80, 24).expect("size");
    let reason = refused(&mut plain, |req| Message::CreateBlock {
        req,
        window,
        size,
        launch: owned(),
    });
    assert!(reason.contains("feature bits"), "{reason}");
}

#[test]
fn an_owner_gone_for_good_orphans_its_block() {
    let path = start_server();
    // Without TAKEOVER nothing keeps the client once its connection ends.
    let mut owner = Client::connect_with(&path, OWNER, None);
    let mut peer = Client::connect_with(&path, OWNER, None);
    let window = window_for(&mut owner);
    let block = launch_in(&mut owner, window, owned());
    drop(owner);
    wait_owner(&mut peer, block, Some(Ownership::Orphaned));

    let target = EntityId::Block(block);
    let reason = refused(&mut peer, |req| Message::Rename {
        req,
        target,
        name: "taken".into(),
    });
    assert!(reason.contains("orphaned"), "{reason}");
    succeeds(&mut peer, |req| Message::Attach { req, block });
    let reason = refused(&mut peer, |req| Message::AcquireLease { req, block });
    assert!(reason.contains("orphaned"), "{reason}");
    // The owner's principal, the same account here, may clean it up.
    succeeds(&mut peer, |req| Message::CloseBlock { req, block });
}

#[test]
fn a_reconnect_within_the_grace_keeps_the_ownership() {
    let path = start_server();
    let mut owner = Client::connect_with(&path, RECONNECTING_OWNER, None);
    let window = window_for(&mut owner);
    let block = launch_in(&mut owner, window, owned());
    let takeover = Takeover {
        client: owner.id(),
        token: owner.token.expect("a token"),
    };
    drop(owner);
    let mut again = Client::taking_over(&path, RECONNECTING_OWNER, None, Some(takeover));
    assert_eq!(again.id(), takeover.client, "the same client");
    assert_eq!(
        owner_of(&mut again, block),
        Some(Ownership::Client(again.id()))
    );
    let target = EntityId::Block(block);
    succeeds(&mut again, |req| Message::Rename {
        req,
        target,
        name: "still mine".into(),
    });
}
