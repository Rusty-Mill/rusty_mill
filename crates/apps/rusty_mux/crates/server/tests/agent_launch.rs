//! Agent launch (C75): a block's program can be given exactly the
//! environment its launch names, and the descriptors its launch frame
//! passed, at 3, 4, and on. The server keeps no copy of them, and closes
//! them when it refuses. Linux only: descriptors travel as `SCM_RIGHTS`.

#![cfg(target_os = "linux")]

mod common;

use std::io::{PipeReader, PipeWriter, Read};
use std::os::fd::AsRawFd;

use common::{Client, start_server, window_for};
use rusty_mux_core::{BlockId, Size, WindowId};
use rusty_mux_proto::{Features, Launch, Message, Program};

const AGENT: Features = Features::CHECKPOINT_V6
    .union(Features::LAUNCH)
    .union(Features::AGENT_LAUNCH);

/// `/bin/sh -c script`, which waits for a line before running it.
fn script(script: &str) -> Launch {
    Launch {
        program: Some(Program {
            command: "/bin/sh".into(),
            args: vec!["-c".into(), format!("read go; {script}; sleep 30")],
        }),
        ..Launch::default()
    }
}

fn create(window: WindowId, launch: Launch) -> impl FnOnce(u64) -> Message {
    let size = Size::new(200, 24).expect("size");
    move |req| Message::CreateBlock {
        req,
        window,
        size,
        launch,
    }
}

/// The new block's id, or why it was refused.
fn launched(answer: &[Message]) -> Result<BlockId, String> {
    match answer {
        [
            ..,
            Message::Done {
                created: Some(id), ..
            },
        ] => Ok(BlockId::from_uuid(*id)),
        [.., Message::Failed { reason, .. }] => Err(reason.clone()),
        other => panic!("unexpected answer {other:?}"),
    }
}

/// Attaches, takes the lease and lets the script run.
fn go(client: &mut Client, block: BlockId) {
    client.request(|req| Message::Attach { req, block });
    client.request(|req| Message::AcquireLease { req, block });
    client.input(block, b"\r");
}

/// What a pipe's reader gets until every write end has closed: proof the
/// server holds no copy of a passed one.
fn drain(mut reader: PipeReader) -> String {
    let mut out = String::new();
    reader.read_to_string(&mut out).expect("read the pipe");
    out
}

fn pipe() -> (PipeReader, PipeWriter) {
    std::io::pipe().expect("pipe")
}

#[test]
fn the_environment_is_exactly_the_one_given() {
    let socket = start_server();
    let mut client = Client::connect_with(&socket, AGENT, None);
    let window = window_for(&mut client);
    let mut launch = script("env | sort | tr '\\n' ' '; echo END");
    launch.env = Some(vec![
        ("PATH".into(), "/usr/bin:/bin".into()),
        ("AGENT".into(), "a=b".into()),
    ]);
    let block = launched(&client.answer(create(window, launch))).expect("launched");
    go(&mut client, block);
    client.wait_for(block, "END");
    let out = String::from_utf8_lossy(client.output(block)).into_owned();
    assert!(out.contains("AGENT=a=b "), "{out}");
    assert!(
        !out.contains("RUSTY_MUX_"),
        "nothing of the server's: {out}"
    );
    assert!(!out.contains("HOME="), "nothing inherited: {out}");
}

#[test]
fn passed_descriptors_reach_the_program_in_order_and_nowhere_else() {
    let socket = start_server();
    let mut client = Client::connect_with(&socket, AGENT, None);
    let window = window_for(&mut client);
    let (first, first_w) = pipe();
    let (second, second_w) = pipe();
    let mut launch = script(
        "echo one >&3; echo two >&4; ls /proc/$$/fd | sort -n | tr '\\n' ' '; echo END; exec 3>&- 4>&-",
    );
    launch.fds = 2;
    let fds = [first_w.as_raw_fd(), second_w.as_raw_fd()];
    let answer = client.answer_passing(create(window, launch), &fds);
    drop((first_w, second_w));
    let block = launched(&answer).expect("launched");
    go(&mut client, block);
    client.wait_for(block, "END");
    let out = String::from_utf8_lossy(client.output(block)).into_owned();
    assert!(
        out.contains("0 1 2 3 4 END"),
        "only the pty and the two: {out}"
    );
    // EOF once the program closes them: the server kept no copy.
    assert_eq!(drain(first), "one\n");
    assert_eq!(drain(second), "two\n");
}

#[test]
fn launches_that_do_not_match_their_descriptors_are_refused_and_closed() {
    let socket = start_server();
    let mut client = Client::connect_with(&socket, AGENT, None);
    let window = window_for(&mut client);
    let cases: [(u8, usize, &str); 3] = [
        (2, 1, "asks for 2 and its frame passed 1"),
        (0, 1, "asks for 0 and its frame passed 1"),
        (1, 0, "asks for 1 and its frame passed 0"),
    ];
    for (want, sent, why) in cases {
        let pipes: Vec<_> = (0..sent).map(|_| pipe()).collect();
        let fds: Vec<i32> = pipes.iter().map(|(_, w)| w.as_raw_fd()).collect();
        let mut launch = script("true");
        launch.fds = want;
        let reason =
            launched(&client.answer_passing(create(window, launch), &fds)).expect_err("refused");
        assert!(
            reason.starts_with("bad_launch(fds)") && reason.contains(why),
            "{reason}"
        );
        for (reader, writer) in pipes {
            drop(writer);
            assert_eq!(drain(reader), "", "closed by the server");
        }
    }
}

#[test]
fn descriptors_on_another_request_are_closed() {
    let socket = start_server();
    let mut client = Client::connect_with(&socket, AGENT, None);
    let (reader, writer) = pipe();
    let create = |req| Message::CreateSession {
        req,
        name: "s".into(),
    };
    let answer = client.answer_passing(create, &[writer.as_raw_fd()]);
    assert!(
        matches!(answer.last(), Some(Message::Done { .. })),
        "{answer:?}"
    );
    drop(writer);
    assert_eq!(drain(reader), "");
}

#[test]
fn the_server_variables_and_unnegotiated_launches_are_refused() {
    let socket = start_server();
    let mut client = Client::connect_with(&socket, AGENT, None);
    let window = window_for(&mut client);
    let mut launch = script("true");
    launch.env = Some(vec![("RUSTY_MUX_SOCKET".into(), "/tmp/x".into())]);
    let reason = launched(&client.answer(create(window, launch))).expect_err("refused");
    assert!(reason.starts_with("bad_launch(env)"), "{reason}");

    let plain = Features::CHECKPOINT_V6.union(Features::LAUNCH);
    let mut older = Client::connect_with(&socket, plain, None);
    let window = window_for(&mut older);
    let mut launch = script("true");
    launch.env = Some(Vec::new());
    let reason = launched(&older.answer(create(window, launch))).expect_err("refused");
    assert!(reason.contains("requires feature bits"), "{reason}");
}
