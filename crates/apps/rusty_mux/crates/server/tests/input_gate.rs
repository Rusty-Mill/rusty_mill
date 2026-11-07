//! The input gate (C77): an owner-only block takes input from its owner
//! and from clients the owner granted, and every byte typed into it is
//! reported to the owner, or refused when the owner can't be told. A
//! viewer may ask the owner for a grant (Q26); the owner decides.

#![cfg(any(target_os = "linux", windows))]

mod common;

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use common::{Client, start_server, window_for};
use rusty_mux_core::{BlockId, ClientId, Event, InputMode, Size, WindowId};
use rusty_mux_proto::{Entry, Features, Launch, Message, Takeover};

const BASE: Features = Features::CHECKPOINT_V6
    .union(Features::EVENT_SEQ)
    .union(Features::INSPECT)
    .union(Features::LAUNCH)
    .union(Features::SYNC);
const OWNER: Features = BASE
    .union(Features::OWNERSHIP)
    .union(Features::INPUT_GATE)
    .union(Features::TAKEOVER);
const ASKING: Features = OWNER.union(Features::LEASE_REQUESTS);

fn now_ms() -> u64 {
    let since = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock");
    u64::try_from(since.as_millis()).expect("ms")
}

fn gated() -> Launch {
    Launch {
        own: true,
        input: InputMode::OwnerOnly,
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

fn attach(client: &mut Client, block: BlockId) {
    client.request(|req| Message::Attach { req, block });
}

fn refused(client: &mut Client, make: impl FnOnce(u64) -> Message) -> String {
    match client.answer(make).as_slice() {
        [Message::Failed { reason, .. }] => reason.clone(),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn grant(owner: &mut Client, block: BlockId, client: ClientId) {
    let until = now_ms() + 60_000;
    owner.request(|req| Message::GrantLease {
        req,
        block,
        client,
        until,
    });
}

/// Writes `data` to `block`: `Err` with the reason if it was refused.
fn write(client: &mut Client, block: BlockId, data: &[u8]) -> Result<(), String> {
    let input = client.input(block, data);
    let sync = client.input_sync();
    loop {
        match client.recv() {
            Message::Failed { req, reason, .. } if req == input => return Err(reason),
            Message::Done { req, .. } if req == sync => return Ok(()),
            _ => {}
        }
    }
}

/// The next input `owner` is told of: who typed it, its number and bytes.
fn next_batch(owner: &mut Client) -> (ClientId, u64, Vec<u8>) {
    loop {
        if let Message::InputBatch {
            client, seq, data, ..
        } = owner.recv()
        {
            return (client, seq, data);
        }
    }
}

fn input_mode(client: &mut Client, block: BlockId) -> InputMode {
    client
        .answer(|req| Message::List { req })
        .into_iter()
        .find_map(|m| match m {
            Message::Entry {
                entry: Entry::Block {
                    block: b, input, ..
                },
                ..
            } if b == block => Some(input),
            _ => None,
        })
        .expect("the block is listed")
}

#[test]
fn only_a_granted_client_may_type_and_the_owner_hears_every_byte() {
    let path = start_server();
    let mut owner = Client::connect_with(&path, OWNER, None);
    let mut peer = Client::connect_with(&path, OWNER, None);
    let window = window_for(&mut owner);
    let block = launch_in(&mut owner, window, gated());
    assert_eq!(input_mode(&mut peer, block), InputMode::OwnerOnly);
    attach(&mut peer, block);

    let reason = refused(&mut peer, |req| Message::AcquireLease { req, block });
    assert!(reason.contains("input_gated"), "{reason}");

    grant(&mut owner, block, peer.id());
    peer.request(|req| Message::AcquireLease { req, block });
    write(&mut peer, block, b"true\r").expect("granted input");
    assert_eq!(next_batch(&mut owner), (peer.id(), 1, b"true\r".to_vec()));
    write(&mut peer, block, b"x").expect("granted input");
    assert_eq!(next_batch(&mut owner), (peer.id(), 2, b"x".to_vec()));
    assert!(
        owner.events.iter().any(|(_, e)| matches!(
            e,
            Event::GrantChanged { client, until: Some(_), .. } if *client == peer.id()
        )),
        "the owner heard of its grant"
    );

    owner.request(|req| Message::RevokeLease {
        req,
        block,
        client: peer.id(),
    });
    let reason = write(&mut peer, block, b"y").expect_err("revoked");
    assert!(reason.contains("lease"), "{reason}");
    let reason = refused(&mut peer, |req| Message::AcquireLease { req, block });
    assert!(reason.contains("input_gated"), "{reason}");
}

#[test]
fn only_the_owner_grants_and_only_for_a_while() {
    let path = start_server();
    let mut owner = Client::connect_with(&path, OWNER, None);
    let mut peer = Client::connect_with(&path, OWNER, None);
    let window = window_for(&mut owner);
    let block = launch_in(&mut owner, window, gated());
    let open = launch_in(
        &mut owner,
        window,
        Launch {
            own: true,
            ..Launch::default()
        },
    );
    let peer_id = peer.id();
    let reason = refused(&mut peer, |req| Message::GrantLease {
        req,
        block,
        client: peer_id,
        until: now_ms() + 1_000,
    });
    assert!(reason.contains("own"), "{reason}");
    for until in [now_ms() - 1, now_ms() + 2 * 24 * 3600 * 1000] {
        let reason = refused(&mut owner, |req| Message::GrantLease {
            req,
            block,
            client: peer_id,
            until,
        });
        assert!(reason.contains("grant"), "{until}: {reason}");
    }
    // An open block takes input from anyone, unreported.
    attach(&mut peer, open);
    peer.request(|req| Message::AcquireLease { req, block: open });
    write(&mut peer, open, b"true\r").expect("open input");
}

#[test]
fn a_gated_launch_needs_the_feature_and_an_owner() {
    let path = start_server();
    let mut plain = Client::connect_with(&path, BASE.union(Features::OWNERSHIP), None);
    let window = window_for(&mut plain);
    let size = Size::new(80, 24).expect("size");
    let reason = refused(&mut plain, |req| Message::CreateBlock {
        req,
        window,
        size,
        launch: gated(),
    });
    assert!(reason.contains("feature bits"), "{reason}");

    let mut gate = Client::connect_with(&path, OWNER, None);
    let reason = refused(&mut gate, |req| Message::CreateBlock {
        req,
        window,
        size,
        launch: Launch {
            input: InputMode::OwnerOnly,
            ..Launch::default()
        },
    });
    assert!(reason.contains("own"), "{reason}");
}

#[test]
fn input_waits_for_an_owner_in_its_grace_and_its_grants_survive() {
    let path = start_server();
    let mut owner = Client::connect_with(&path, OWNER, None);
    let mut peer = Client::connect_with(&path, OWNER, None);
    let window = window_for(&mut owner);
    let block = launch_in(&mut owner, window, gated());
    attach(&mut peer, block);
    grant(&mut owner, block, peer.id());
    peer.request(|req| Message::AcquireLease { req, block });
    write(&mut peer, block, b"a").expect("granted input");
    assert_eq!(next_batch(&mut owner).1, 1);

    let takeover = Takeover {
        client: owner.id(),
        token: owner.token.expect("a token"),
    };
    drop(owner);
    // Nobody can be told of input until the owner is back.
    let deadline = Instant::now() + common::PATIENCE;
    let reason = loop {
        match write(&mut peer, block, b"b") {
            Err(reason) => break reason,
            Ok(()) => {
                assert!(Instant::now() < deadline, "input was never refused");
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    assert!(reason.contains("input_unrecorded"), "{reason}");

    let mut again = Client::taking_over(&path, OWNER, None, Some(takeover));
    assert_eq!(again.id(), takeover.client, "the same client");
    write(&mut peer, block, b"c").expect("the grant survived");
    // Input written before the server saw the drop may come first.
    let (from, seq) = loop {
        let (from, seq, data) = next_batch(&mut again);
        if data == b"c" {
            break (from, seq);
        }
    };
    assert_eq!(from, peer.id());
    assert!(seq >= 2, "numbered after the first: {seq}");
}

/// The next lease request `owner` is told of: who asks, for which block.
fn next_ask(owner: &mut Client) -> (BlockId, ClientId, String) {
    loop {
        if let Message::LeaseRequested {
            block,
            client,
            principal,
        } = owner.recv()
        {
            return (block, client, principal.id().to_owned());
        }
    }
}

#[test]
fn a_viewer_asks_the_owner_and_the_owner_grants() {
    let path = start_server();
    let mut owner = Client::connect_with(&path, ASKING, None);
    let mut viewer = Client::connect_with(&path, ASKING, None);
    let window = window_for(&mut owner);
    let block = launch_in(&mut owner, window, gated());
    attach(&mut viewer, block);

    viewer.request(|req| Message::RequestLease { req, block });
    let (asked, who, principal) = next_ask(&mut owner);
    assert_eq!((asked, who), (block, viewer.id()));
    assert!(!principal.is_empty(), "the verified principal comes along");

    grant(&mut owner, block, viewer.id());
    viewer.request(|req| Message::AcquireLease { req, block });
    write(&mut viewer, block, b"true\r").expect("granted input");
    // Giving back is the viewer's release; the grant stays until revoked.
    viewer.request(|req| Message::ReleaseLease { req, block });
    viewer.request(|req| Message::AcquireLease { req, block });
}

#[test]
fn a_request_is_refused_when_it_cannot_reach_an_owner_or_needs_none() {
    let path = start_server();
    let mut owner = Client::connect_with(&path, ASKING, None);
    let mut viewer = Client::connect_with(&path, ASKING, None);
    let window = window_for(&mut owner);
    let block = launch_in(&mut owner, window, gated());
    let open = launch_in(
        &mut owner,
        window,
        Launch {
            own: true,
            ..Launch::default()
        },
    );
    let reason = refused(&mut owner, |req| Message::RequestLease { req, block });
    assert!(reason.contains("yours"), "{reason}");
    let reason = refused(&mut viewer, |req| Message::RequestLease {
        req,
        block: open,
    });
    assert!(reason.contains("anyone"), "{reason}");

    // An owner that can't hear requests is never asked.
    let mut deaf = Client::connect_with(&path, OWNER, None);
    let quiet = launch_in(&mut deaf, window, gated());
    let reason = refused(&mut viewer, |req| Message::RequestLease {
        req,
        block: quiet,
    });
    assert!(reason.contains("owner_unreachable"), "{reason}");

    // Nor is an owner in its grace.
    drop(owner);
    let deadline = Instant::now() + common::PATIENCE;
    loop {
        let reason = match viewer
            .answer(|req| Message::RequestLease { req, block })
            .as_slice()
        {
            [Message::Failed { reason, .. }] => reason.clone(),
            _ => String::new(),
        };
        if reason.contains("owner_unreachable") {
            break;
        }
        assert!(Instant::now() < deadline, "the request was never refused");
        std::thread::sleep(Duration::from_millis(20));
    }

    let mut plain = Client::connect_with(&path, OWNER, None);
    let reason = refused(&mut plain, |req| Message::RequestLease {
        req,
        block: quiet,
    });
    assert!(reason.contains("feature bits"), "{reason}");
}
