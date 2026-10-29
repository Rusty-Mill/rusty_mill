//! Taking a client over from a new connection: a reconnecting client keeps
//! its id, name, attachment and input lease, and the old connection is
//! closed.

#![cfg(any(target_os = "linux", windows))]

mod common;

use common::{Client, block_for, run_and_wait, start_server};
use rusty_mux_core::EntityId;
use rusty_mux_proto::{Entry, Features, Message, Takeover};
use rusty_uuid::Uuid;

const TAKEOVER: Features = Features::CHECKPOINT_V6
    .union(Features::INSPECT)
    .union(Features::LABELS)
    .union(Features::TAKEOVER);

fn takeover_of(client: &Client) -> Takeover {
    Takeover {
        client: client.id(),
        token: client.token.expect("a token"),
    }
}

#[test]
fn a_reconnect_keeps_the_client_its_screen_and_its_lease() {
    let path = start_server();
    let mut old = Client::connect_with(&path, TAKEOVER, None);
    let block = block_for(&mut old);
    let seen = run_and_wait(&mut old, block, 1);
    let target = EntityId::Client(old.id());
    old.request(|req| Message::Rename {
        req,
        target,
        name: "laptop".into(),
    });

    // The old path is still open, as after a network change with no FIN.
    let mut new = Client::taking_over(&path, TAKEOVER, None, Some(takeover_of(&old)));
    assert_eq!(new.id(), old.id(), "the same client");
    assert_ne!(new.token, old.token, "a fresh token");
    new.wait_ready(block);
    assert!(new.text(block).contains(&seen), "the screen came along");

    old.wait_closed();
    let names: Vec<_> = new
        .answer(|req| Message::List { req })
        .into_iter()
        .filter_map(|m| match m {
            Message::Entry {
                entry: Entry::Client { client, name, .. },
                ..
            } => Some((client, name)),
            _ => None,
        })
        .collect();
    assert_eq!(
        names,
        [(new.id(), "laptop".to_owned())],
        "one client, named"
    );
    // Typing needs the lease: it was kept, and the old connection's end did
    // not drop the client.
    run_and_wait(&mut new, block, 2);
}

#[test]
fn a_wrong_or_used_token_starts_a_new_client() {
    let path = start_server();
    let mut first = Client::connect_with(&path, TAKEOVER, None);
    let block = block_for(&mut first);
    let used = takeover_of(&first);

    let wrong = Takeover {
        token: Uuid::new_v4(),
        ..used
    };
    let stranger = Client::taking_over(&path, TAKEOVER, None, Some(wrong));
    assert_ne!(stranger.id(), first.id());
    run_and_wait(&mut first, block, 1);

    let mut second = Client::taking_over(&path, TAKEOVER, None, Some(used));
    assert_eq!(second.id(), first.id());
    let replay = Client::taking_over(&path, TAKEOVER, None, Some(used));
    assert_ne!(replay.id(), first.id(), "a token works once");
    second.wait_ready(block);
    run_and_wait(&mut second, block, 2);
}

#[test]
fn a_takeover_with_other_features_starts_a_new_client() {
    let path = start_server();
    let mut old = Client::connect_with(&path, TAKEOVER, None);
    let block = block_for(&mut old);
    let token = takeover_of(&old);

    // Without `CHECKPOINT_V6` it could not take a checkpoint of `block`.
    let fewer = Features::INSPECT
        .union(Features::LABELS)
        .union(Features::TAKEOVER);
    let mut downgraded = Client::taking_over(&path, fewer, None, Some(token));
    assert_ne!(downgraded.id(), old.id());
    downgraded.answer(|req| Message::List { req });
    assert!(
        !downgraded.has_checkpoint(block),
        "no checkpoint it can't read"
    );

    // The original keeps its connection, lease and token.
    run_and_wait(&mut old, block, 1);
    let mut new = Client::taking_over(&path, TAKEOVER, None, Some(token));
    assert_eq!(new.id(), old.id());
    new.wait_ready(block);
    run_and_wait(&mut new, block, 2);
}

#[test]
fn without_the_feature_there_is_no_token_and_no_takeover() {
    let path = start_server();
    let first = Client::connect_with(&path, TAKEOVER, None);
    let plain = Client::connect_with(&path, Features::CHECKPOINT_V6, None);
    assert_eq!(plain.token, None);
    let ignored = Client::taking_over(
        &path,
        Features::CHECKPOINT_V6,
        None,
        Some(takeover_of(&first)),
    );
    assert_ne!(ignored.id(), first.id());
}
