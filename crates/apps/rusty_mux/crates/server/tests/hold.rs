//! Hold on exit (C78): a block launched with `hold` stays once its program
//! exits, read-only with its last screen, listed as exited, until it is
//! closed. Unix only: the scripts are `/bin/sh`.

#![cfg(unix)]

mod common;

use common::{Client, PATIENCE, start_server, window_for};
use rusty_mux_core::{BlockId, Size};
use rusty_mux_proto::schema::Value;
use rusty_mux_proto::{Entry, Features, Launch, Message, Program};

const HOLDING: Features = Features::CHECKPOINT_V6
    .union(Features::LAUNCH)
    .union(Features::INSPECT)
    .union(Features::BLOCK_CALL)
    .union(Features::TERMINAL_SHELL)
    .union(Features::HOLD);

fn launch(script: &str, hold: bool) -> Launch {
    Launch {
        program: Some(Program {
            command: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
        }),
        hold,
        ..Launch::default()
    }
}

fn create(client: &mut Client, launch: Launch) -> Vec<Message> {
    let window = window_for(client);
    let size = Size::new(80, 24).expect("size");
    client.answer(|req| Message::CreateBlock {
        req,
        window,
        size,
        launch,
    })
}

fn created(answer: &[Message]) -> BlockId {
    match answer {
        [
            ..,
            Message::Done {
                created: Some(id), ..
            },
        ] => BlockId::from_uuid(*id),
        other => panic!("not created: {other:?}"),
    }
}

/// The block's listing entry's exited mark; `None` once it is gone.
fn exited(client: &mut Client, block: BlockId) -> Option<bool> {
    client
        .answer(|req| Message::List { req })
        .into_iter()
        .find_map(|m| match m {
            Message::Entry {
                entry: Entry::Block {
                    block: b, exited, ..
                },
                ..
            } if b == block => Some(exited),
            _ => None,
        })
}

/// Waits until the listing says the block's program has exited.
fn wait_exited(client: &mut Client, block: BlockId) {
    let deadline = std::time::Instant::now() + PATIENCE;
    while exited(client, block) != Some(true) {
        assert!(std::time::Instant::now() < deadline, "never exited");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

fn call(client: &mut Client, block: BlockId, method: &str) -> Vec<Message> {
    let args: Vec<(String, Value)> = match method {
        "terminal.format" => vec![("history".into(), Value::Lines(0))],
        _ => Vec::new(),
    };
    let method = method.to_owned();
    client.answer(move |req| Message::BlockCall {
        req,
        block,
        method,
        args,
    })
}

fn text(answer: &[Message]) -> String {
    match answer {
        [Message::Text { text, .. }] => text.clone(),
        other => panic!("expected Text, got {other:?}"),
    }
}

#[test]
fn a_held_block_keeps_its_screen_after_its_program_exits() {
    let socket = start_server();
    let mut client = Client::connect_with(&socket, HOLDING, None);
    let block = created(&create(&mut client, launch("echo HELD-OUT; exit 3", true)));
    wait_exited(&mut client, block);

    let screen = text(&call(&mut client, block, "terminal.format"));
    assert!(screen.contains("HELD-OUT"), "{screen}");
    let shell = text(&call(&mut client, block, "terminal.shell"));
    assert!(shell.contains("activity\texited"), "{shell}");

    client.request(|req| Message::Attach { req, block });
    client.wait_ready(block);
    assert!(client.text(block).contains("HELD-OUT"), "attach shows it");
    client.request(|req| Message::AcquireLease { req, block });
    client.send(&Message::Input {
        req: 900,
        block,
        data: b"x".to_vec(),
    });
    let refused = loop {
        match client.recv() {
            Message::Failed {
                req: 900, reason, ..
            } => break reason,
            Message::Done { req: 900, .. } => panic!("input to an exited block taken"),
            _ => {}
        }
    };
    assert!(refused.contains("its program has exited"), "{refused}");

    client.request(|req| Message::CloseBlock { req, block });
    assert_eq!(exited(&mut client, block), None, "closed");
}

#[test]
fn an_unheld_block_still_closes_and_a_running_one_is_not_exited() {
    let socket = start_server();
    let mut client = Client::connect_with(&socket, HOLDING, None);
    let running = created(&create(&mut client, launch("sleep 30", true)));
    assert_eq!(exited(&mut client, running), Some(false));
    let gone = created(&create(&mut client, launch("exit 0", false)));
    let deadline = std::time::Instant::now() + PATIENCE;
    while exited(&mut client, gone).is_some() {
        assert!(std::time::Instant::now() < deadline, "never closed");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn a_held_launch_needs_the_feature() {
    let socket = start_server();
    let plain = Features::CHECKPOINT_V6.union(Features::LAUNCH);
    let mut client = Client::connect_with(&socket, plain, None);
    let answer = create(&mut client, launch("true", true));
    assert!(
        matches!(answer.as_slice(), [Message::Failed { reason, .. }] if reason.contains("requires feature bits")),
        "{answer:?}"
    );
}

fn rerun(client: &mut Client, block: BlockId) -> Vec<Message> {
    client.answer(|req| Message::BlockCall {
        req,
        block,
        method: "terminal.rerun".into(),
        args: Vec::new(),
    })
}

fn wait_screen(client: &mut Client, block: BlockId, want: &str) -> String {
    let deadline = std::time::Instant::now() + PATIENCE;
    loop {
        let screen = text(&call(client, block, "terminal.format"));
        if screen.contains(want) {
            return screen;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "never showed {want}: {screen}"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn a_held_block_is_re_run_under_the_lease_and_keeps_its_screen() {
    let socket = start_server();
    let mut client = Client::connect_with(&socket, HOLDING, None);
    let block = created(&create(&mut client, launch("echo FIRST-RUN; exit 0", true)));
    wait_exited(&mut client, block);
    client.request(|req| Message::Attach { req, block });
    client.wait_ready(block);

    // No lease, no re-run.
    let refused = rerun(&mut client, block);
    assert!(
        matches!(refused.as_slice(), [Message::Failed { reason, .. }] if !reason.contains("launched") && !reason.contains("running")),
        "{refused:?}"
    );

    client.request(|req| Message::AcquireLease { req, block });
    let done = rerun(&mut client, block);
    assert!(
        matches!(done.as_slice(), [Message::Done { .. }]),
        "{done:?}"
    );
    // The command ran again; the first run's output and a line stay.
    let screen = wait_screen(&mut client, block, "-- re-run at");
    assert!(screen.contains("FIRST-RUN"), "{screen}");
    wait_exited(&mut client, block);
    let shown = text(&call(&mut client, block, "terminal.format"));
    assert_eq!(shown.matches("FIRST-RUN").count(), 2, "{shown}");
}

/// The block starts again in the directory it was launched in, which its
/// workspace holds, even if that path now names another directory.
#[cfg(target_os = "linux")]
#[test]
fn a_re_run_starts_in_the_launch_directory_the_workspace_holds() {
    let base = std::env::temp_dir().join(format!("rusty-mux-rerun-cwd-{}", std::process::id()));
    let (orig, moved) = (base.join("orig"), base.join("moved"));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&orig).unwrap();
    let socket = start_server();
    let mut client = Client::connect_with(&socket, HOLDING, None);
    let mut spec = launch("echo ran >> marker; exit 0", true);
    spec.cwd = Some(orig.to_string_lossy().into_owned());
    let block = created(&create(&mut client, spec));
    wait_exited(&mut client, block);
    client.request(|req| Message::Attach { req, block });
    client.wait_ready(block);
    client.request(|req| Message::AcquireLease { req, block });

    // The launch path now names a different directory.
    std::fs::rename(&orig, &moved).unwrap();
    std::fs::create_dir(&orig).unwrap();
    let done = rerun(&mut client, block);
    assert!(
        matches!(done.as_slice(), [Message::Done { .. }]),
        "{done:?}"
    );
    wait_screen(&mut client, block, "-- re-run at");
    wait_exited(&mut client, block);
    let marked = std::fs::read_to_string(moved.join("marker")).unwrap_or_default();
    assert_eq!(marked.lines().count(), 2, "{marked:?}");
    assert!(!orig.join("marker").exists());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_running_block_is_not_re_run() {
    let socket = start_server();
    let mut client = Client::connect_with(&socket, HOLDING, None);
    let block = created(&create(&mut client, launch("sleep 30", true)));
    client.request(|req| Message::Attach { req, block });
    client.wait_ready(block);
    client.request(|req| Message::AcquireLease { req, block });
    let answer = rerun(&mut client, block);
    assert!(
        matches!(answer.as_slice(), [Message::Failed { reason, .. }] if reason.contains("still running")),
        "{answer:?}"
    );
}

#[test]
fn a_block_whose_output_ended_before_its_program_is_not_re_run_until_reaped() {
    let socket = start_server();
    let mut client = Client::connect_with(&socket, HOLDING, None);
    // Closing its terminal ends the block's output (the hang-up) while the
    // program lives on: the window between hang-up and reap.
    let block = created(&create(
        &mut client,
        launch("trap '' HUP; exec >&- 2>&- <&-; sleep 2", true),
    ));
    client.request(|req| Message::Attach { req, block });
    client.wait_ready(block);
    client.request(|req| Message::AcquireLease { req, block });
    let deadline = std::time::Instant::now() + PATIENCE;
    loop {
        let answer = rerun(&mut client, block);
        match answer.as_slice() {
            [Message::Failed { reason, .. }] if reason.contains("not finished ending") => break,
            [Message::Failed { reason, .. }] if reason.contains("still running") => {}
            other => panic!("expected a refusal naming the unfinished run: {other:?}"),
        }
        assert!(std::time::Instant::now() < deadline, "never hung up");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    // Once reaped and announced, the same call goes through.
    let deadline = std::time::Instant::now() + PATIENCE;
    loop {
        let answer = rerun(&mut client, block);
        match answer.as_slice() {
            [Message::Done { .. }] => break,
            [Message::Failed { reason, .. }] if reason.contains("not finished ending") => {}
            other => panic!("{other:?}"),
        }
        assert!(std::time::Instant::now() < deadline, "never reaped");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn a_block_launched_with_its_own_environment_is_not_re_run_whatever_it_says_it_is() {
    let socket = start_server();
    let mut client = Client::connect_with(&socket, HOLDING.union(Features::AGENT_LAUNCH), None);
    // No `agent` metadata: the launch's environment is what marks it.
    let mut own = launch("exit 0", true);
    own.env = Some(vec![("PATH".into(), "/usr/bin:/bin".into())]);
    let block = created(&create(&mut client, own));
    wait_exited(&mut client, block);
    client.request(|req| Message::Attach { req, block });
    client.wait_ready(block);
    client.request(|req| Message::AcquireLease { req, block });
    let answer = rerun(&mut client, block);
    assert!(
        matches!(answer.as_slice(), [Message::Failed { reason, .. }] if reason.contains("re-run through its launcher")),
        "{answer:?}"
    );
}

#[test]
fn a_client_without_hold_cannot_re_run() {
    let socket = start_server();
    let mut holder = Client::connect_with(&socket, HOLDING, None);
    let block = created(&create(&mut holder, launch("exit 0", true)));
    wait_exited(&mut holder, block);
    let mut plain = Client::connect_with(
        &socket,
        Features::CHECKPOINT_V6
            .union(Features::BLOCK_CALL)
            .union(Features::INSPECT),
        None,
    );
    let answer = rerun(&mut plain, block);
    assert!(
        matches!(answer.as_slice(), [Message::Failed { .. }]),
        "{answer:?}"
    );
}

#[test]
fn a_re_run_takes_no_arguments() {
    let socket = start_server();
    let mut client = Client::connect_with(&socket, HOLDING, None);
    let block = created(&create(&mut client, launch("exit 0", true)));
    wait_exited(&mut client, block);
    let answer = client.answer(|req| Message::BlockCall {
        req,
        block,
        method: "terminal.rerun".into(),
        args: vec![("x".into(), Value::Flag(true))],
    });
    assert!(
        matches!(answer.as_slice(), [Message::Failed { reason, .. }] if reason.contains("no arguments")),
        "{answer:?}"
    );
}
