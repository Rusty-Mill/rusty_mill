//! `terminal.prompts` and `terminal.find` (C55): the rows where the shell's
//! prompts started and the rows holding a text.

#![cfg(target_os = "linux")]

mod common;

use common::{Client, block_for, start_server};
use rusty_mux_core::BlockId;
use rusty_mux_proto::schema::Value;
use rusty_mux_proto::{Features, Message};

const FIND: Features = Features::CHECKPOINT_V6
    .union(Features::INSPECT)
    .union(Features::BLOCK_CALL)
    .union(Features::TERMINAL_FIND);

fn call(client: &mut Client, block: BlockId, method: &str, args: &[(&str, Value)]) -> Message {
    let method = method.to_owned();
    let args = args
        .iter()
        .map(|(n, v)| ((*n).to_owned(), v.clone()))
        .collect();
    let mut answer = client.answer(move |req| Message::BlockCall {
        req,
        block,
        method,
        args,
    });
    assert_eq!(answer.len(), 1, "{answer:?}");
    answer.remove(0)
}

fn text(client: &mut Client, block: BlockId, method: &str, args: &[(&str, Value)]) -> String {
    match call(client, block, method, args) {
        Message::Text { text, .. } => text,
        other => panic!("{method}: expected Text, got {other:?}"),
    }
}

/// Has the shell mark two prompts and print a line between them.
fn marked(client: &mut Client, block: BlockId) {
    client.input(
        block,
        b"printf '\\033]133;A\\007first prompt\\nNeed%s one\\n\\033]133;A\\007second prompt\\n' le; \
          echo DONE$((1+1))\r",
    );
    client.wait_for(block, "DONE2");
}

#[test]
fn prompts_and_find_read_the_blocks_scrollback_and_screen() {
    let path = start_server();
    let mut client = Client::connect_with(&path, FIND, None);
    let block = block_for(&mut client);
    assert_eq!(text(&mut client, block, "terminal.prompts", &[]), "");
    marked(&mut client, block);

    let prompts = text(&mut client, block, "terminal.prompts", &[]);
    let lines: Vec<&str> = prompts.lines().collect();
    assert_eq!(lines.len(), 2, "{prompts:?}");
    for (line, want) in lines.iter().zip(["first prompt", "second prompt"]) {
        let (row, row_text) = line.split_once('\t').expect(line);
        assert!(row.parse::<i64>().is_ok(), "{line}");
        assert_eq!(row_text, want);
    }

    let query = Value::Text("needle one".to_owned());
    let found = text(&mut client, block, "terminal.find", &[("query", query)]);
    let fields: Vec<&str> = found.split('\t').collect();
    assert_eq!(fields.len(), 3, "{found:?}");
    assert_eq!(fields[1], "0", "the match starts the row");
    assert_eq!(fields[2], "Needle one");
    // The row is the one after the first prompt's.
    let first: i64 = lines[0].split('\t').next().unwrap().parse().unwrap();
    assert_eq!(fields[0].parse::<i64>().unwrap(), first + 1);

    let limited = [
        ("query", Value::Text("o".to_owned())),
        ("limit", Value::Lines(1)),
    ];
    assert_eq!(
        text(&mut client, block, "terminal.find", &limited)
            .lines()
            .count(),
        1
    );
    let none = [("query", Value::Text("absent-text".to_owned()))];
    assert_eq!(text(&mut client, block, "terminal.find", &none), "");
}

#[test]
fn a_client_without_the_feature_is_refused() {
    let path = start_server();
    let older = Features::CHECKPOINT_V6
        .union(Features::INSPECT)
        .union(Features::BLOCK_CALL);
    let mut client = Client::connect_with(&path, older, None);
    let block = block_for(&mut client);
    for method in ["terminal.prompts", "terminal.find"] {
        let args = [("query", Value::Text("x".to_owned()))];
        let args = if method == "terminal.find" {
            &args[..]
        } else {
            &[]
        };
        match call(&mut client, block, method, args) {
            Message::Failed { reason, .. } => {
                assert!(
                    reason.contains("requires feature bits"),
                    "{method}: {reason}"
                );
            }
            other => panic!("{method}: expected Failed, got {other:?}"),
        }
    }
}
