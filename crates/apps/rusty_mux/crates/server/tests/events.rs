//! The resumable event stream (C05): numbered events, and replay of the
//! events a client missed while disconnected.

#![cfg(target_os = "linux")]

mod common;

use common::{Client, block_for, start_server, window_for};
use rusty_mux_core::{BlockId, Event, ExitStatus, Progress, SessionId, Size};
use rusty_mux_proto::{Cursor, Features, Launch, Message, Program};
use rusty_uuid::Uuid;

const SEQ: Features = Features::EVENT_SEQ;

fn create_session(client: &mut Client, name: &str) -> SessionId {
    let id = client
        .request(|req| Message::CreateSession {
            req,
            name: name.into(),
        })
        .expect("id");
    SessionId::from_uuid(id)
}

/// Reads until the event announcing `session` arrives.
fn wait_for_session(client: &mut Client, session: SessionId) {
    let created = Event::SessionCreated { session };
    while !client.events.iter().any(|(_, e)| *e == created) {
        client.recv();
    }
}

fn numbers(client: &Client) -> Vec<u64> {
    client
        .events
        .iter()
        .map(|(seq, _)| seq.expect("numbered"))
        .collect()
}

fn last_seen(client: &Client) -> Cursor {
    let start = client.cursor.expect("cursor");
    Cursor {
        seq: numbers(client).last().copied().unwrap_or(start.seq),
        ..start
    }
}

#[test]
fn a_resumed_stream_has_no_gap_and_no_repeat() {
    let path = start_server();
    let mut a = Client::connect_with(&path, SEQ, None);
    assert_eq!(a.cursor.map(|c| c.seq), Some(0));
    let first = create_session(&mut a, "first");
    wait_for_session(&mut a, first);
    let seen = last_seen(&a);
    drop(a);

    // Changes made while A is away, by someone else.
    let mut b = Client::connect(&path);
    let missed = create_session(&mut b, "missed");

    let mut a = Client::connect_with(&path, SEQ, Some(seen));
    assert_eq!(a.cursor, Some(seen), "the server resumed the stream");
    let after = create_session(&mut a, "after");
    wait_for_session(&mut a, after);

    let resumed = numbers(&a);
    let expected: Vec<u64> = (seen.seq + 1..=seen.seq + resumed.len() as u64).collect();
    assert_eq!(resumed, expected, "contiguous from the resume point");
    assert!(
        a.events
            .iter()
            .any(|(_, e)| *e == Event::SessionCreated { session: missed }),
        "the missed change was replayed"
    );
}

#[test]
fn a_cursor_from_another_run_starts_afresh() {
    let path = start_server();
    let mut b = Client::connect(&path);
    create_session(&mut b, "before");

    let stale = Cursor {
        epoch: Uuid::from_bytes([7; 16]),
        seq: 1,
    };
    let mut a = Client::connect_with(&path, SEQ, Some(stale));
    let start = a.cursor.expect("cursor");
    assert_ne!(start, stale, "not resumed");
    let session = create_session(&mut a, "after");
    wait_for_session(&mut a, session);
    assert_eq!(
        numbers(&a).first(),
        Some(&(start.seq + 1)),
        "nothing replayed"
    );
}

#[test]
fn a_client_without_the_feature_gets_unnumbered_events() {
    let path = start_server();
    let mut a = Client::connect_with(&path, Features::NONE, None);
    assert_eq!(a.cursor, None);
    let session = create_session(&mut a, "s");
    wait_for_session(&mut a, session);
    assert!(a.events.iter().all(|(seq, _)| seq.is_none()));
}

#[test]
fn status_changes_reach_only_clients_that_asked() {
    let path = start_server();
    let wanted = SEQ.union(Features::STATUS).union(Features::CHECKPOINT_V6);
    let mut watcher = Client::connect_with(&path, wanted, None);
    // With layout events, so its only gap is the status event.
    let mut plain = Client::connect_with(&path, SEQ.union(Features::LAYOUT), None);
    let block = block_for(&mut watcher);
    watcher.input(block, b"printf '\\033]7501;state=working\\007'\r");
    let status = Event::BlockStatus { block };
    while !watcher.events.iter().any(|(_, e)| *e == status) {
        watcher.recv();
    }

    // Events reach every client in one order, so once the plain client sees
    // a later one it has missed the status event for good.
    let later = create_session(&mut watcher, "later");
    wait_for_session(&mut plain, later);
    assert!(!plain.events.iter().any(|(_, e)| *e == status));
    let gaps = numbers(&plain)
        .windows(2)
        .filter(|w| w[1] != w[0] + 1)
        .count();
    assert_eq!(gaps, 1, "its numbers skip the status event");
}

#[test]
fn replay_sends_status_events_only_to_clients_that_asked() {
    let path = start_server();
    let status = SEQ.union(Features::STATUS);
    let mut away = Vec::new();
    for features in [SEQ, status] {
        let mut client = Client::connect_with(&path, features, None);
        let session = create_session(&mut client, "before");
        wait_for_session(&mut client, session);
        away.push((features, last_seen(&client)));
    }

    // While both are away: a status change, then a later event.
    let mut writer = Client::connect_with(&path, status.union(Features::CHECKPOINT_V6), None);
    let block = block_for(&mut writer);
    writer.input(block, b"printf '\\033]7501;state=working\\007'\r");
    let changed = Event::BlockStatus { block };
    while !writer.events.iter().any(|(_, e)| *e == changed) {
        writer.recv();
    }
    let after = create_session(&mut writer, "after");

    for (features, seen) in away {
        let mut client = Client::connect_with(&path, features, Some(seen));
        assert_eq!(client.cursor, Some(seen), "resumed");
        wait_for_session(&mut client, after);
        let replayed = client.events.iter().any(|(_, e)| *e == changed);
        assert_eq!(
            replayed,
            features.contains(Features::STATUS),
            "{features:?}"
        );
    }
}

#[test]
fn pwd_and_title_changes_reach_only_clients_that_asked() {
    let path = start_server();
    let wanted = SEQ
        .union(Features::TERMINAL_EVENTS)
        .union(Features::CHECKPOINT_V6);
    let mut watcher = Client::connect_with(&path, wanted, None);
    let mut plain = Client::connect_with(&path, SEQ.union(Features::STATUS), None);
    let block = block_for(&mut watcher);
    watcher.input(
        block,
        b"printf '\\033]7;file://host/tmp\\007\\033]2;job\\007'\r",
    );
    let changes = [Event::PwdChanged { block }, Event::TitleChanged { block }];
    while !changes
        .iter()
        .all(|c| watcher.events.iter().any(|(_, e)| e == c))
    {
        watcher.recv();
    }

    let later = create_session(&mut watcher, "later");
    wait_for_session(&mut plain, later);
    assert!(!plain.events.iter().any(|(_, e)| changes.contains(e)));
}

#[test]
fn program_events_reach_only_clients_that_asked() {
    let path = start_server();
    let wanted = SEQ
        .union(Features::PROGRAM_EVENTS)
        .union(Features::CHECKPOINT_V6);
    let mut watcher = Client::connect_with(&path, wanted, None);
    let mut plain = Client::connect_with(&path, SEQ.union(Features::TERMINAL_EVENTS), None);
    let block = block_for(&mut watcher);
    watcher.input(
        block,
        b"printf '\\a\\033]9;4;1;40\\a\\033]777;notify;T;B\\a'; exit 3\r",
    );
    let closed = Event::BlockClosed { block };
    while !watcher.events.iter().any(|(_, e)| *e == closed) {
        watcher.recv();
    }
    let program: Vec<&Event> = watcher
        .events
        .iter()
        .map(|(_, e)| e)
        .skip_while(|e| !matches!(e, Event::Bell { .. }))
        .collect();
    // Closing the block then lets go of its lease and clients.
    assert_eq!(program.last(), Some(&&closed));
    assert_eq!(
        program[..4],
        [
            &Event::Bell { block },
            &Event::ProgressChanged {
                block,
                progress: Progress::Normal(40)
            },
            &Event::Notification {
                block,
                title: "T".into(),
                body: "B".into()
            },
            &Event::ProgramExited {
                block,
                status: ExitStatus::Code(3)
            },
        ]
    );

    while !plain.events.iter().any(|(_, e)| *e == closed) {
        plain.recv();
    }
    assert!(!plain.events.iter().any(|(_, e)| matches!(
        e,
        Event::Bell { .. }
            | Event::ProgressChanged { .. }
            | Event::Notification { .. }
            | Event::ProgramExited { .. }
    )));
}

/// A job that starts in the foreground without writing anything is seen by
/// the poll, and the shell taking the terminal back by its prompt; only
/// clients that asked hear of either.
#[test]
fn process_changes_reach_only_clients_that_asked() {
    let path = start_server();
    let wanted = SEQ.union(Features::PROCESS).union(Features::CHECKPOINT_V6);
    let mut watcher = Client::connect_with(&path, wanted, None);
    let mut plain = Client::connect_with(&path, SEQ.union(Features::PROGRAM_EVENTS), None);
    let block = block_for(&mut watcher);
    let changed_to = |client: &Client, program: &str| {
        client.events.iter().any(|(_, e)| {
            matches!(e, Event::ProcessChanged { block: b, name, .. } if *b == block && name == program)
        })
    };
    watcher.input(block, b"sleep 30\r");
    while !changed_to(&watcher, "sleep") {
        watcher.recv();
    }
    watcher.input(block, b"\x03");
    while !changed_to(&watcher, "sh") {
        watcher.recv();
    }

    let later = create_session(&mut watcher, "later");
    wait_for_session(&mut plain, later);
    assert!(
        !plain
            .events
            .iter()
            .any(|(_, e)| matches!(e, Event::ProcessChanged { .. }))
    );
}

#[test]
fn a_closed_block_reports_no_exit() {
    let path = start_server();
    let wanted = SEQ
        .union(Features::PROGRAM_EVENTS)
        .union(Features::CHECKPOINT_V6);
    let mut client = Client::connect_with(&path, wanted, None);
    let block = block_for(&mut client);
    client.request(|req| Message::CloseBlock { req, block });
    let closed = Event::BlockClosed { block };
    while !client.events.iter().any(|(_, e)| *e == closed) {
        client.recv();
    }
    // Anything after the close would follow it on the stream.
    let later = create_session(&mut client, "later");
    wait_for_session(&mut client, later);
    assert!(
        !client
            .events
            .iter()
            .any(|(_, e)| matches!(e, Event::ProgramExited { .. }))
    );
}

/// The answer and DA1's, in the order the program asked, even when both
/// questions arrive in one write.
#[test]
fn a_status_query_is_answered_in_order() {
    let path = start_server();
    let mut client = Client::connect(&path);
    let block = block_for(&mut client);
    // Each round ends by printing its own marker after `stty sane`, and the
    // next is typed only once that marker is out: a line typed while the
    // terminal is still raw keeps its `\r`, so the shell never sees it end.
    let ask = |first: &str, second: &str, round: u32| {
        format!(
            "stty raw -echo; printf '{first}{second}'; \
             dd bs=1 count=19 2>/dev/null | od -An -tx1; stty sane; echo END$(({round}*2))\r"
        )
    };
    let (query, da1) = ("\\033]7501;?\\033\\\\", "\\033[c");
    client.input(block, ask(query, da1, 1).as_bytes());
    client.wait_for(block, "END2");
    client.wait_for(block, "1b 5d 37 35 30 31 3b 3f 1b 5c 1b 5b 3f 31 3b 32");
    client.input(block, ask(da1, query, 2).as_bytes());
    client.wait_for(block, "END4");
    client.wait_for(block, "1b 5b 3f 31 3b 32 3b 34 63 1b 5d 37 35 30 31 3b");
}

/// A block can run a given program in a given directory instead of the
/// shell, and its exit code comes back as an event.
#[test]
fn a_launched_program_reports_its_exit_code() {
    let path = start_server();
    let wanted = SEQ.union(Features::PROGRAM_EVENTS);
    let mut client = Client::connect_with(&path, wanted.union(Features::LAUNCH), None);
    let window = window_for(&mut client);
    let launch = Launch {
        program: Some(Program {
            command: "sh".into(),
            args: vec!["-c".into(), r#"[ "$(pwd)" = /tmp ] && exit 3"#.into()],
        }),
        cwd: Some("/tmp".into()),
    };
    let size = Size::new(80, 24).expect("size");
    let block = BlockId::from_uuid(
        client
            .request(|req| Message::CreateBlock {
                req,
                window,
                size,
                launch: launch.clone(),
            })
            .expect("id"),
    );
    let exited = Event::ProgramExited {
        block,
        status: ExitStatus::Code(3),
    };
    while !client.events.iter().any(|(_, e)| *e == exited) {
        client.recv();
    }

    for (cwd, why) in [
        ("tmp", "not an absolute path"),
        ("/no/such/dir", "not a directory"),
    ] {
        let launch = Launch {
            cwd: Some(cwd.into()),
            ..launch.clone()
        };
        let answer = client.answer(|req| Message::CreateBlock {
            req,
            window,
            size,
            launch,
        });
        assert!(
            matches!(answer.last(), Some(Message::Failed { reason, .. }) if reason.contains(why)),
            "{cwd}: {answer:?}"
        );
    }
}

/// A launch on a connection that did not negotiate it is refused before
/// anything starts; a plain block still works there.
#[test]
fn a_launch_needs_the_negotiated_feature() {
    let path = start_server();
    let mut client = Client::connect_with(&path, SEQ, None);
    let window = window_for(&mut client);
    let size = Size::new(80, 24).expect("size");
    let launches = [
        Launch {
            program: Some(Program {
                command: "true".into(),
                args: Vec::new(),
            }),
            cwd: None,
        },
        Launch {
            program: None,
            cwd: Some("/tmp".into()),
        },
    ];
    for launch in launches {
        let answer = client.answer(|req| Message::CreateBlock {
            req,
            window,
            size,
            launch: launch.clone(),
        });
        assert!(
            matches!(answer.last(), Some(Message::Failed { reason, .. }) if reason.contains("a launch requires")),
            "{launch:?}: {answer:?}"
        );
    }
    let plain = BlockId::from_uuid(
        client
            .request(|req| Message::CreateBlock {
                req,
                window,
                size,
                launch: Launch::default(),
            })
            .expect("id"),
    );
    // Events arrive in order: the plain block's is the only one created.
    while !client
        .events
        .iter()
        .any(|(_, e)| matches!(e, Event::BlockCreated { block, .. } if *block == plain))
    {
        client.recv();
    }
    let created = client
        .events
        .iter()
        .filter(|(_, e)| matches!(e, Event::BlockCreated { .. }))
        .count();
    assert_eq!(created, 1);
}
