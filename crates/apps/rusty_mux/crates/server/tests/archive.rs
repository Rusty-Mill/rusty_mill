//! Archive of dismissed blocks (C84): `archive-block` saves a held block's
//! transcript under the state directory and closes it; `archive-list` and
//! `archive-show` read the records back. Unix only: the scripts are
//! `/bin/sh`.

#![cfg(unix)]

mod common;

use std::path::PathBuf;

use common::{Client, PATIENCE, shell, start_server, start_server_with, window_for};
use rusty_mux_core::{BlockId, Size};
use rusty_mux_proto::{Entry, Features, Launch, Message, Program};

const ARCHIVING: Features = Features::CHECKPOINT_V6
    .union(Features::LAUNCH)
    .union(Features::INSPECT)
    .union(Features::HOLD)
    .union(Features::ARCHIVE);

fn state(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rusty-mux-archive-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("state dir");
    dir
}

fn held(client: &mut Client, script: &str) -> BlockId {
    let window = window_for(client);
    let size = Size::new(80, 24).expect("size");
    let launch = Launch {
        program: Some(Program {
            command: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
        }),
        hold: true,
        ..Launch::default()
    };
    let answer = client.answer(|req| Message::CreateBlock {
        req,
        window,
        size,
        launch,
    });
    match answer.as_slice() {
        [
            ..,
            Message::Done {
                created: Some(id), ..
            },
        ] => BlockId::from_uuid(*id),
        other => panic!("not created: {other:?}"),
    }
}

fn exited(client: &mut Client, block: BlockId) -> bool {
    client
        .answer(|req| Message::List { req })
        .into_iter()
        .any(|m| {
            matches!(
                m,
                Message::Entry {
                    entry: Entry::Block { block: b, exited: true, .. },
                    ..
                } if b == block
            )
        })
}

fn wait_exited(client: &mut Client, block: BlockId) {
    let deadline = std::time::Instant::now() + PATIENCE;
    while !exited(client, block) {
        assert!(std::time::Instant::now() < deadline, "never exited");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

fn archive(client: &mut Client, block: BlockId, card: &str) -> Vec<Message> {
    let card = Some(card.to_owned());
    client.answer(|req| Message::ArchiveBlock {
        req,
        block,
        card,
        label: Some("good".into()),
        why: Some("card done".into()),
    })
}

fn text(answer: &[Message]) -> String {
    match answer {
        [Message::Text { text, .. }] => text.clone(),
        other => panic!("expected Text, got {other:?}"),
    }
}

fn failed(answer: &[Message]) -> String {
    match answer {
        [Message::Failed { reason, .. }] => reason.clone(),
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[test]
fn a_held_block_is_archived_listed_shown_and_closed() {
    let dir = state("flow");
    let socket = start_server_with(rusty_mux_server::Config {
        state_dir: Some(dir.clone()),
        ..shell()
    });
    let mut client = Client::connect_with(&socket, ARCHIVING, None);
    let block = held(&mut client, "echo ARCHIVED-OUT; exit 3");
    wait_exited(&mut client, block);

    let done = archive(&mut client, block, "card-7");
    assert!(
        matches!(done.as_slice(), [Message::Done { .. }]),
        "{done:?}"
    );
    assert!(!exited(&mut client, block), "the block closed");

    let all = text(&client.answer(|req| Message::ArchiveList { req, card: None }));
    let row: Vec<&str> = all.lines().next().expect("a row").split('\t').collect();
    assert_eq!(&row[2..], ["code:3", "no", "card-7", "good"], "{all}");
    let other = text(&client.answer(|req| Message::ArchiveList {
        req,
        card: Some("card-8".into()),
    }));
    assert_eq!(other, "");

    let id = row[0].to_owned();
    let shown = text(&client.answer(|req| Message::ArchiveShow { req, id }));
    assert!(shown.contains("why: card done"), "{shown}");
    assert!(shown.contains("\n\nARCHIVED-OUT"), "{shown}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_transcript_with_a_secret_marks_its_record_sensitive() {
    let dir = state("secret");
    let socket = start_server_with(rusty_mux_server::Config {
        state_dir: Some(dir.clone()),
        ..shell()
    });
    let mut client = Client::connect_with(&socket, ARCHIVING, None);
    let block = held(&mut client, "echo password=hunter2hunter2");
    wait_exited(&mut client, block);
    archive(&mut client, block, "c");
    let all = text(&client.answer(|req| Message::ArchiveList { req, card: None }));
    assert!(all.contains("\tyes\t"), "{all}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_running_block_is_not_archived_and_stays() {
    let dir = state("running");
    let socket = start_server_with(rusty_mux_server::Config {
        state_dir: Some(dir.clone()),
        ..shell()
    });
    let mut client = Client::connect_with(&socket, ARCHIVING, None);
    let block = held(&mut client, "sleep 30");
    let reason = failed(&archive(&mut client, block, "c"));
    assert!(reason.contains("still running"), "{reason}");
    assert!(!exited(&mut client, block));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_server_without_state_refuses_and_the_feature_is_needed() {
    let socket = start_server();
    let mut client = Client::connect_with(&socket, ARCHIVING, None);
    let block = held(&mut client, "exit 0");
    wait_exited(&mut client, block);
    let reason = failed(&archive(&mut client, block, "c"));
    assert!(reason.contains("no state directory"), "{reason}");

    let plain = Features::CHECKPOINT_V6.union(Features::INSPECT);
    let mut old = Client::connect_with(&socket, plain, None);
    let reason = failed(&old.answer(|req| Message::ArchiveList { req, card: None }));
    assert!(reason.contains("requires feature bits"), "{reason}");
}

#[test]
fn record_ids_cannot_name_other_files() {
    let dir = state("ids");
    let socket = start_server_with(rusty_mux_server::Config {
        state_dir: Some(dir.clone()),
        ..shell()
    });
    let mut client = Client::connect_with(&socket, ARCHIVING, None);
    let reason = failed(&client.answer(|req| Message::ArchiveShow {
        req,
        id: "../sessions".into(),
    }));
    assert!(reason.contains("not a record id"), "{reason}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_transcript_too_long_for_one_reply_is_refused_and_the_block_stays() {
    let dir = state("long");
    let socket = start_server_with(rusty_mux_server::Config {
        state_dir: Some(dir.clone()),
        ..shell()
    });
    let mut client = Client::connect_with(&socket, ARCHIVING, None);
    // About 4 MiB of output, over what `archive-show` can answer in one reply
    // and what the spool and screen hold between them.
    let block = held(
        &mut client,
        "yes 0123456789012345678901234567890123456789012345678901234567890123456789012345678 | head -c 4400000",
    );
    wait_exited(&mut client, block);
    let reason = failed(&archive(&mut client, block, "c"));
    assert!(
        reason.contains("too long") || reason.contains("lines"),
        "{reason}"
    );
    assert!(exited(&mut client, block), "the block must stay");
    let _ = std::fs::remove_dir_all(&dir);
}
