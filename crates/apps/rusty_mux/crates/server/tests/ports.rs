//! `terminal.ports` (C80): the TCP ports a block's programs listen on.

#![cfg(target_os = "linux")]

mod common;

use common::{Client, block_for, start_server};
use rusty_mux_core::BlockId;
use rusty_mux_proto::{Features, Message};

const PORTS: Features = Features::CHECKPOINT_V6
    .union(Features::INSPECT)
    .union(Features::BLOCK_CALL)
    .union(Features::PORTS);

fn ports(client: &mut Client, block: BlockId) -> String {
    let mut answer = client.answer(move |req| Message::BlockCall {
        req,
        block,
        method: "terminal.ports".to_owned(),
        args: Vec::new(),
    });
    assert_eq!(answer.len(), 1, "{answer:?}");
    match answer.remove(0) {
        Message::Text { text, .. } => text,
        other => panic!("expected Text, got {other:?}"),
    }
}

fn have_python() -> bool {
    std::process::Command::new("python3")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn a_block_lists_the_ports_its_programs_listen_on_and_stops_when_they_stop() {
    if !have_python() {
        eprintln!("skipped: no python3 to listen with");
        return;
    }
    let path = start_server();
    let mut client = Client::connect_with(&path, PORTS, None);
    let block = block_for(&mut client);
    assert_eq!(ports(&mut client, block), "", "nothing listens yet");

    // A job of the shell listens on a free loopback port and says so. The
    // word is built in the program so the typed command does not hold it.
    client.input(
        block,
        b"python3 -c \"import socket,time;s=socket.socket();s.bind(('127.0.0.1',0));\
          s.listen();print('LISTEN'+'ING',s.getsockname()[1]);time.sleep(60)\" &\r",
    );
    client.wait_for(block, "LISTENING");

    let listed = ports(&mut client, block);
    let lines: Vec<&str> = listed.lines().collect();
    assert_eq!(lines.len(), 1, "{listed:?}");
    let fields: Vec<&str> = lines[0].split('\t').collect();
    assert_eq!(fields.len(), 3, "{listed:?}");
    assert!(fields[0].parse::<u16>().is_ok_and(|p| p > 0), "{listed:?}");
    assert!(fields[1].parse::<u32>().is_ok(), "{listed:?}");
    assert!(fields[2].starts_with("python"), "{listed:?}");

    client.input(block, b"kill %1; wait; echo GONE$((1+1))\r");
    client.wait_for(block, "GONE2");
    assert_eq!(ports(&mut client, block), "", "the listener is gone");
}
