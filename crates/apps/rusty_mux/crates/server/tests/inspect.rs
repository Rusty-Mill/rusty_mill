//! Reading the server's state without attaching: `list` (C06) and
//! `capture` (C18).

#![cfg(target_os = "linux")]

mod common;

use common::{Client, block_for, new_block, start_server, window_for};
use rusty_mux_core::BlockId;
use rusty_mux_proto::{Code, Entry, Features, Message};

const INSPECT: Features = Features::CHECKPOINT_V3.union(Features::INSPECT);

fn capture(client: &mut Client, block: BlockId, history: u32) -> String {
    match client
        .answer(|req| Message::Capture {
            req,
            block,
            history,
        })
        .as_slice()
    {
        [Message::Text { text, .. }] => text.clone(),
        other => panic!("expected Text, got {other:?}"),
    }
}

fn refusal(answer: Vec<Message>) -> String {
    match answer.as_slice() {
        [Message::Failed { reason, .. }] => reason.clone(),
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[test]
fn list_gives_each_parent_before_its_children() {
    let path = start_server();
    let mut client = Client::connect_with(&path, INSPECT, None);
    let window = window_for(&mut client);
    let leased = new_block(&mut client, window);
    client.request(|req| Message::Attach { req, block: leased });
    client.request(|req| Message::AcquireLease { req, block: leased });
    let idle = new_block(&mut client, window);
    let answer = client.answer(|req| Message::List { req });

    let (entries, done) = answer.split_at(answer.len() - 1);
    assert!(matches!(done, [Message::Done { created: None, .. }]));
    let entries: Vec<&Entry> = entries
        .iter()
        .map(|m| match m {
            Message::Entry { entry, .. } => entry,
            other => panic!("expected Entry, got {other:?}"),
        })
        .collect();
    let [
        Entry::Session { session, name },
        Entry::Window {
            window: w,
            session: ws,
            name: wname,
        },
        Entry::Block {
            block: b1,
            window: bw1,
            holder: Some(holder),
            size,
            title,
        },
        Entry::Block {
            block: b2,
            window: bw2,
            holder: None,
            ..
        },
    ] = entries.as_slice()
    else {
        panic!("unexpected listing: {entries:?}");
    };
    assert_eq!((name.as_str(), wname.as_str()), ("s", "w"));
    assert_eq!((ws, *w, *bw1, *bw2), (session, window, window, window));
    assert_eq!((*b1, *holder, *b2), (leased, client.id(), idle));
    assert_eq!((size.cols(), size.rows(), title.as_str()), (80, 24, ""));
}

#[test]
fn capture_reads_the_screen_after_the_history_asked_for() {
    let path = start_server();
    let mut client = Client::connect_with(&path, INSPECT, None);
    let block = block_for(&mut client);
    // 60 numbered lines scroll the first ones off a 24-row screen.
    client.input(block, b"seq 101 160; echo DONE$((1+1))\r");
    client.wait_for(block, "DONE2");

    let screen = capture(&mut client, block, 0);
    assert_eq!(screen.lines().count(), 24, "{screen}");
    assert!(screen.contains("\n160\n") && screen.contains("DONE2"));
    assert!(!screen.lines().any(|l| l == "101"));

    let more = capture(&mut client, block, 5);
    assert_eq!(more.lines().count(), 29);
    assert!(more.ends_with(&screen));

    let all = capture(&mut client, block, u32::MAX);
    assert!(all.lines().any(|l| l == "101"), "{all}");
    assert!(all.ends_with(&screen));
}

#[test]
fn inspecting_needs_the_feature_and_a_real_block() {
    let path = start_server();
    let mut plain = Client::connect(&path);
    let block = block_for(&mut plain);
    let reason = refusal(plain.answer(|req| Message::List { req }));
    assert!(reason.contains("list requires"), "{reason}");
    let reason = refusal(plain.answer(|req| Message::Capture {
        req,
        block,
        history: 0,
    }));
    assert!(reason.contains("capture requires"), "{reason}");

    let mut client = Client::connect_with(&path, INSPECT, None);
    let missing = BlockId::from_uuid(rusty_uuid::Uuid::new_v4());
    let reason = refusal(client.answer(|req| Message::Capture {
        req,
        block: missing,
        history: 0,
    }));
    assert!(reason.contains("unknown block"), "{reason}");
}

#[test]
fn an_oversized_listing_is_refused_and_the_connection_kept() {
    let path = start_server();
    let mut client = Client::connect_with(&path, INSPECT, None);
    // Each name fits one frame, but five exceed the bound on one reply.
    for n in 0..5u8 {
        client.request(|req| Message::CreateSession {
            req,
            name: char::from(b'a' + n).to_string().repeat(1 << 20),
        });
    }
    let reason = refusal(client.answer(|req| Message::List { req }));
    assert!(reason.contains("list reply exceeds"), "{reason}");
    assert!(matches!(
        client.answer(|req| Message::Sync { req }).as_slice(),
        [Message::Done { .. }]
    ));
}

#[test]
fn a_missing_entity_is_refused_as_not_found() {
    let path = start_server();
    let mut client = Client::connect_with(&path, INSPECT, None);
    let missing = rusty_uuid::Uuid::new_v4();
    let block = BlockId::from_uuid(missing);
    let session = rusty_mux_core::SessionId::from_uuid(missing);
    let requests: [Box<dyn Fn(u64) -> Message>; 3] = [
        Box::new(move |req| Message::CloseBlock { req, block }),
        Box::new(move |req| Message::Capture {
            req,
            block,
            history: 0,
        }),
        Box::new(move |req| Message::CreateWindow {
            req,
            session,
            name: "w".into(),
        }),
    ];
    for make in requests {
        let answer = client.answer(make);
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
    // A refusal for another reason carries no code.
    let block = block_for(&mut client);
    let mut other = Client::connect_with(&path, INSPECT, None);
    other.request(|req| Message::Attach { req, block });
    let answer = other.answer(|req| Message::AcquireLease { req, block });
    assert!(
        matches!(answer.as_slice(), [Message::Failed { code: None, .. }]),
        "{answer:?}"
    );
}

#[test]
fn a_block_program_knows_where_it_is() {
    let path = start_server();
    let mut client = Client::connect_with(&path, INSPECT, None);
    let block = block_for(&mut client);
    client.input(
        block,
        b"echo \"B=$RUSTY_MUX_BLOCK S=$RUSTY_MUX_SOCKET\" | tr a-z A-Z\r",
    );
    let expected = format!(
        "B={} S={}",
        block.to_string().to_uppercase(),
        std::path::absolute(&path)
            .expect("absolute")
            .display()
            .to_string()
            .to_uppercase()
    );
    client.wait_for(block, &expected);
}
