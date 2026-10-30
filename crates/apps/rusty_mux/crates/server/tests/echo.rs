//! Echo acks and modes on a real PTY: an ack follows whatever the program
//! echoed within the grace, and the mode tells who echoes, ahead of the
//! output printed after it changed. Linux only: ConPTY has no termios.

#![cfg(target_os = "linux")]

mod common;

use common::{Client, PATIENCE, start_server, window_for};
use rusty_mux_core::{BlockId, Size};
use rusty_mux_proto::{Echo, Features, Launch, Message, Program};

const ECHO: Features = Features::CHECKPOINT_V6
    .union(Features::LAUNCH)
    .union(Features::ECHO_ACK);

/// Starts `/bin/sh -c script` in a new block, attached and leased.
fn running(client: &mut Client, script: &str) -> BlockId {
    let window = window_for(client);
    let launch = Launch {
        program: Some(Program {
            command: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
        }),
        cwd: None,
    };
    let size = Size::new(80, 24).expect("size");
    let block = BlockId::from_uuid(
        client
            .request(|req| Message::CreateBlock {
                req,
                window,
                size,
                launch,
            })
            .expect("id"),
    );
    client.request(|req| Message::Attach { req, block });
    client.request(|req| Message::AcquireLease { req, block });
    block
}

/// Reads until an ack covering `req`; how much output preceded it.
fn ack(client: &mut Client, req: u64) -> usize {
    let deadline = std::time::Instant::now() + PATIENCE;
    loop {
        let found = client.echoes.iter().find_map(|(at, m)| match m {
            Message::EchoAck { req: r, .. } if *r >= req => Some(*at),
            _ => None,
        });
        if let Some(at) = found {
            return at;
        }
        assert!(std::time::Instant::now() < deadline, "no ack for {req}");
        client.recv();
    }
}

/// Reads until the newest mode is `mode`; how much output preceded it.
fn mode(client: &mut Client, mode: Echo) -> usize {
    let deadline = std::time::Instant::now() + PATIENCE;
    loop {
        let last = client.echoes.iter().rev().find_map(|(at, m)| match m {
            Message::EchoMode { mode, .. } => Some((*at, *mode)),
            _ => None,
        });
        if let Some((at, seen)) = last
            && seen == mode
        {
            return at;
        }
        assert!(std::time::Instant::now() < deadline, "never {mode:?}");
        client.recv();
    }
}

/// Reads until `block`'s screen shows `needle`, which may have been printed
/// before the attach.
fn wait_screen(client: &mut Client, block: BlockId, needle: &str) {
    let deadline = std::time::Instant::now() + PATIENCE;
    while !client.text(block).contains(needle) {
        assert!(
            std::time::Instant::now() < deadline,
            "never shown: {needle}"
        );
        client.recv();
    }
}

/// Where `needle` first appears in `block`'s output.
fn position(client: &Client, block: BlockId, needle: &[u8]) -> Option<usize> {
    client
        .output(block)
        .windows(needle.len())
        .position(|w| w == needle)
}

/// A program that echoes each key itself, `delay` seconds after reading
/// it, once it has printed `READY`.
fn echoing_after(delay: &str) -> String {
    format!(
        "stty -icanon -echo; printf READY; \
         while c=$(dd bs=1 count=1 2>/dev/null); do sleep {delay}; printf %s \"$c\"; done"
    )
}

#[test]
fn attaching_announces_the_mode_and_a_kernel_echo_precedes_its_ack() {
    let path = start_server();
    let mut client = Client::connect_with(&path, ECHO, None);
    assert!(client.features.contains(Features::ECHO_ACK));
    let block = running(&mut client, "exec cat");
    assert!(
        client
            .echoes
            .iter()
            .any(|(_, m)| matches!(m, Message::EchoMode { .. })),
        "a mode right after attach"
    );
    mode(&mut client, Echo::Kernel);
    let req = client.input(block, b"k");
    let at = ack(&mut client, req);
    assert!(
        position(&client, block, b"k").is_some_and(|p| p < at),
        "the kernel's echo is before the ack"
    );
}

#[test]
fn a_program_echo_within_the_grace_precedes_its_ack() {
    let path = start_server();
    let mut client = Client::connect_with(&path, ECHO, None);
    let block = running(&mut client, &echoing_after("0.01"));
    wait_screen(&mut client, block, "READY");
    mode(&mut client, Echo::Program);
    let req = client.input(block, b"x");
    let at = ack(&mut client, req);
    assert!(
        position(&client, block, b"x").is_some_and(|p| p < at),
        "an echo 10 ms late is before the ack"
    );
}

#[test]
fn a_program_echo_after_the_grace_follows_its_ack() {
    let path = start_server();
    let mut client = Client::connect_with(&path, ECHO, None);
    let block = running(&mut client, &echoing_after("0.2"));
    wait_screen(&mut client, block, "READY");
    mode(&mut client, Echo::Program);
    let req = client.input(block, b"x");
    let at = ack(&mut client, req);
    assert_eq!(
        position(&client, block, b"x"),
        None,
        "no echo yet at the ack"
    );
    client.wait_for(block, "x");
    assert!(position(&client, block, b"x").is_some_and(|p| p >= at));
}

#[test]
fn echo_turned_off_is_announced_before_the_output_after_it() {
    let path = start_server();
    let mut client = Client::connect_with(&path, ECHO, None);
    // It waits for a line, so that what follows comes after the attach.
    let block = running(
        &mut client,
        "printf READY; read go; stty -echo; printf X$((1+1)); exec cat",
    );
    wait_screen(&mut client, block, "READY");
    client.input(block, b"\r");
    client.wait_for(block, "X2");
    let at = mode(&mut client, Echo::Hidden);
    let printed = position(&client, block, b"X2").expect("printed");
    assert!(at <= printed, "Hidden at {at}, after output to {printed}");
}

#[test]
fn a_hidden_read_is_announced_and_its_unechoed_keys_are_acked() {
    if !std::path::Path::new("/bin/bash").exists() {
        eprintln!("skipped: no /bin/bash for `read -s`");
        return;
    }
    let path = start_server();
    let mut client = Client::connect_with(&path, ECHO, None);
    let block = running(
        &mut client,
        "printf READY; exec /bin/bash --norc --noprofile -c 'read -s x; echo got$x'",
    );
    wait_screen(&mut client, block, "READY");
    mode(&mut client, Echo::Hidden);
    let req = client.input(block, b"pw");
    let at = ack(&mut client, req);
    assert_eq!(position(&client, block, b"pw"), None, "nothing echoed");
    assert_eq!(at, client.output(block).len(), "acked with no output");
    client.input(block, b"\r");
    client.wait_for(block, "gotpw");
}

#[test]
fn a_client_without_the_feature_gets_neither() {
    let path = start_server();
    let mut client =
        Client::connect_with(&path, Features::CHECKPOINT_V6.union(Features::LAUNCH), None);
    let block = running(&mut client, "exec cat");
    client.input(block, b"q\r");
    client.wait_for(block, "q\r\n");
    std::thread::sleep(rusty_mux_proto::ECHO_GRACE * 2);
    // Anything already sent arrives before the reply to this.
    client.request(|req| Message::Detach { req });
    assert!(client.echoes.is_empty(), "{:?}", client.echoes);
}
