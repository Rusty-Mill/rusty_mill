//! Clipboard reads (C54): a program's OSC 52 read is asked of the block's
//! input-lease holder alone, which answers by its own policy; the server
//! never answers for a client, and a reply from anyone else is refused.

#![cfg(target_os = "linux")]

mod common;

use common::{Client, block_for, start_server};
use rusty_mux_core::{BlockId, Selection};
use rusty_mux_proto::{Features, Message};

const READS: Features = Features::CHECKPOINT_V6
    .union(Features::EVENT_SEQ)
    .union(Features::INSPECT)
    .union(Features::SYNC)
    .union(Features::CLIPBOARD);

/// Runs a program that reads the clipboard, then waits for its answer.
fn ask(client: &mut Client, block: BlockId) {
    client.input(block, b"printf '\\033]52;c;?\\007'; read x\r");
}

/// Reads until a [`Message::ClipboardRead`] for `block` arrives.
fn next_read(client: &mut Client, block: BlockId) -> Selection {
    loop {
        if let Message::ClipboardRead {
            block: asked,
            selection,
        } = client.recv()
        {
            assert_eq!(asked, block);
            return selection;
        }
    }
}

#[test]
fn the_lease_holder_is_asked_and_its_answer_reaches_the_program() {
    let path = start_server();
    let mut holder = Client::connect_with(&path, READS, None);
    let block = block_for(&mut holder);
    let mut viewer = Client::connect_with(&path, READS, None);
    viewer.request(|req| Message::Attach { req, block });

    ask(&mut holder, block);
    assert_eq!(next_read(&mut holder, block), Selection::Clipboard);
    holder.send(&Message::ClipboardReply {
        req: 90,
        block,
        selection: Selection::Clipboard,
        data: Some(b"hi".to_vec()),
    });
    // The program's terminal echoes what it was sent: "hi" in base64.
    holder.wait_for(block, "52;c;aGk=");
    // The viewer was never asked: a read would arrive before the output
    // holding the query, so none may come before the answer's echo.
    while !viewer.saw(block, "52;c;aGk=") {
        assert!(!matches!(viewer.recv(), Message::ClipboardRead { .. }));
    }
}

#[test]
fn a_denial_is_an_empty_answer() {
    let path = start_server();
    let mut holder = Client::connect_with(&path, READS, None);
    let block = block_for(&mut holder);
    ask(&mut holder, block);
    next_read(&mut holder, block);
    holder.send(&Message::ClipboardReply {
        req: 90,
        block,
        selection: Selection::Clipboard,
        data: None,
    });
    holder.wait_for(block, "52;c;^G");
}

#[test]
fn a_reply_from_a_client_without_the_lease_is_refused() {
    let path = start_server();
    let mut holder = Client::connect_with(&path, READS, None);
    let block = block_for(&mut holder);
    let mut viewer = Client::connect_with(&path, READS, None);
    viewer.request(|req| Message::Attach { req, block });
    ask(&mut holder, block);
    next_read(&mut holder, block);

    let answer = viewer.answer(|req| Message::ClipboardReply {
        req,
        block,
        selection: Selection::Clipboard,
        data: Some(b"stolen".to_vec()),
    });
    assert!(
        matches!(answer.last(), Some(Message::Failed { reason, .. }) if reason.contains("does not hold the input lease")),
        "{answer:?}"
    );
}

#[test]
fn an_oversized_answer_is_refused() {
    let path = start_server();
    let mut holder = Client::connect_with(&path, READS, None);
    let block = block_for(&mut holder);
    let answer = holder.answer(|req| Message::ClipboardReply {
        req,
        block,
        selection: Selection::Clipboard,
        data: Some(vec![b'x'; (1 << 20) + 1]),
    });
    assert!(
        matches!(answer.last(), Some(Message::Failed { reason, .. }) if reason.contains("at most")),
        "{answer:?}"
    );
}

#[test]
fn a_holder_without_the_feature_is_not_asked() {
    let path = start_server();
    let without = Features::from_bits(READS.bits() & !Features::CLIPBOARD.bits());
    let mut holder = Client::connect_with(&path, without, None);
    assert!(!holder.features.contains(Features::CLIPBOARD));
    let block = block_for(&mut holder);
    holder.input(block, b"printf '\\033]52;c;?\\007'; echo DONE_$((1+1))\r");
    // The ask would come before the output holding the query.
    while !holder.saw(block, "DONE_2") {
        assert!(!matches!(holder.recv(), Message::ClipboardRead { .. }));
    }
}

#[test]
fn a_reply_after_the_lease_moved_is_refused() {
    let path = start_server();
    let mut holder = Client::connect_with(&path, READS, None);
    let block = block_for(&mut holder);
    ask(&mut holder, block);
    next_read(&mut holder, block);
    holder.request(|req| Message::ReleaseLease { req, block });

    let answer = holder.answer(|req| Message::ClipboardReply {
        req,
        block,
        selection: Selection::Clipboard,
        data: Some(b"late".to_vec()),
    });
    assert!(
        matches!(answer.last(), Some(Message::Failed { reason, .. }) if reason.contains("does not hold the input lease")),
        "{answer:?}"
    );
}

fn reply(client: &mut Client, block: BlockId, data: &[u8]) -> Vec<Message> {
    client.answer(|req| Message::ClipboardReply {
        req,
        block,
        selection: Selection::Clipboard,
        data: Some(data.to_vec()),
    })
}

fn refused(answer: &[Message], needle: &str) -> bool {
    matches!(answer.last(), Some(Message::Failed { reason, .. }) if reason.contains(needle))
}

#[test]
fn a_reply_with_no_read_waiting_is_refused() {
    let path = start_server();
    let mut holder = Client::connect_with(&path, READS, None);
    let block = block_for(&mut holder);
    // Nothing asked: a late or invented answer would be typed into
    // whatever runs next.
    assert!(refused(&reply(&mut holder, block, b"x"), "is waiting"));
}

#[test]
fn a_read_is_answered_once() {
    let path = start_server();
    let mut holder = Client::connect_with(&path, READS, None);
    let block = block_for(&mut holder);
    ask(&mut holder, block);
    next_read(&mut holder, block);
    holder.send(&Message::ClipboardReply {
        req: 90,
        block,
        selection: Selection::Clipboard,
        data: Some(b"hi".to_vec()),
    });
    holder.wait_for(block, "52;c;aGk=");
    assert!(refused(&reply(&mut holder, block, b"again"), "is waiting"));
}

#[test]
fn a_program_looping_on_the_query_asks_the_holder_once() {
    let path = start_server();
    let mut holder = Client::connect_with(&path, READS, None);
    let block = block_for(&mut holder);
    holder.input(
        block,
        b"for i in 1 2 3 4 5; do printf '\\033]52;c;?\\007'; done; echo DONE_$((1+1))\r",
    );
    let mut reads = 0;
    while !holder.saw(block, "DONE_2") {
        if let Message::ClipboardRead { .. } = holder.recv() {
            reads += 1;
        }
    }
    assert_eq!(reads, 1, "one outstanding read at a time");
}
