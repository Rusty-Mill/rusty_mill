//! Reading the server's state without attaching: `list` (C06) and the
//! terminal block methods (C06, C18).

#![cfg(target_os = "linux")]

mod common;

use std::time::{Duration, Instant};

use common::{Client, block_for, new_block, start_server, window_for};
use rusty_mux_core::BlockId;
use rusty_mux_proto::schema::Value;
use rusty_mux_proto::{Code, Entry, Features, Message};

const INSPECT: Features = Features::CHECKPOINT_V3
    .union(Features::INSPECT)
    .union(Features::BLOCK_CALL);

fn block_call(block: BlockId, method: &str, args: &[(&str, Value)]) -> impl FnOnce(u64) -> Message {
    let method = method.to_owned();
    let args = args
        .iter()
        .map(|(n, v)| ((*n).to_owned(), v.clone()))
        .collect();
    move |req| Message::BlockCall {
        req,
        block,
        method,
        args,
    }
}

fn text(client: &mut Client, block: BlockId, method: &str, args: &[(&str, Value)]) -> String {
    match client.answer(block_call(block, method, args)).as_slice() {
        [Message::Text { text, .. }] => text.clone(),
        other => panic!("expected Text, got {other:?}"),
    }
}

fn capture(client: &mut Client, block: BlockId, history: u32) -> String {
    let args = [("history", Value::Lines(history))];
    text(client, block, "terminal.format", &args)
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
            ..
        },
        Entry::Block {
            block: b1,
            window: bw1,
            holder: Some(holder),
            size,
            title,
            name: bname,
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
    assert_eq!(bname, "");
    // Clients are listed only with the labels feature.
    assert_eq!(entries.len(), 4);
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
    let reason = refusal(plain.answer(block_call(block, "terminal.title", &[])));
    assert!(reason.contains("a block call requires"), "{reason}");

    let mut client = Client::connect_with(&path, INSPECT, None);
    let missing = BlockId::from_uuid(rusty_uuid::Uuid::new_v4());
    let reason = refusal(client.answer(block_call(missing, "terminal.title", &[])));
    assert!(reason.contains("unknown block"), "{reason}");
}

#[test]
fn terminal_methods_read_the_title_and_working_directory() {
    let path = start_server();
    let mut client = Client::connect_with(&path, INSPECT, None);
    let block = block_for(&mut client);
    // The shell starts in `/` and reports nothing, so pwd reads its process.
    assert_eq!(text(&mut client, block, "terminal.pwd", &[]), "/");
    assert_eq!(text(&mut client, block, "terminal.title", &[]), "");

    client.input(block, b"cd /tmp; echo CD$((1+1))\r");
    client.wait_for(block, "CD2");
    assert_eq!(text(&mut client, block, "terminal.pwd", &[]), "/tmp");

    // A report over OSC 7 wins, percent-decoded.
    client.input(
        block,
        b"printf '\\033]2;my title\\007\\033]7;file://host/a%%20b\\007'; echo OSC$((1+1))\r",
    );
    client.wait_for(block, "OSC2");
    assert_eq!(text(&mut client, block, "terminal.title", &[]), "my title");
    assert_eq!(text(&mut client, block, "terminal.pwd", &[]), "/a b");
}

#[test]
fn terminal_status_reads_the_programs_reports() {
    let path = start_server();
    let mut client = Client::connect_with(&path, INSPECT, None);
    let block = block_for(&mut client);
    assert_eq!(text(&mut client, block, "terminal.status", &[]), "");

    // `aGk=` is "hi"; the malformed report is dropped.
    client.input(
        block,
        b"printf '\\033]7501;state=blocked:kind=question:app=sh:msg=aGk=\\007\\033]7501;id=x\\007\\033]7501;state=done:id=job\\007'; echo ST$((1+1))\r",
    );
    client.wait_for(block, "ST2");
    assert_eq!(
        text(&mut client, block, "terminal.status", &[]),
        "state=blocked:kind=question:app=sh:msg=aGk=\nstate=done:id=job:app=sh"
    );
}

/// The shell is listed alone while idle; a job it runs comes first, above
/// it, once the job has the terminal.
#[test]
fn terminal_process_lists_the_foreground_and_its_ancestors() {
    let path = start_server();
    let mut client = Client::connect_with(&path, INSPECT, None);
    let block = block_for(&mut client);
    let idle = text(&mut client, block, "terminal.process", &[]);
    let shell: Vec<&str> = idle.split('\t').collect();
    assert_eq!(shell[1..], ["sh", "/bin/sh"], "{idle}");

    client.input(block, b"sleep 30\r");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let now = text(&mut client, block, "terminal.process", &[]);
        let rows: Vec<Vec<&str>> = now.lines().map(|l| l.split('\t').collect()).collect();
        if let [job, last] = rows.as_slice() {
            assert_eq!(job[1..], ["sleep", "sleep 30"], "{now}");
            assert_eq!(*last, shell, "{now}");
            break;
        }
        assert!(Instant::now() < deadline, "no job in {now:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
    client.input(block, b"\x03");
}

#[test]
fn block_calls_are_checked_against_the_catalog() {
    let path = start_server();
    let mut client = Client::connect_with(&path, INSPECT, None);
    let block = block_for(&mut client);
    for method in ["terminal.nope", "list", ""] {
        let answer = client.answer(block_call(block, method, &[]));
        assert!(
            matches!(
                answer.as_slice(),
                [Message::Failed {
                    code: Some(Code::MethodNotFound),
                    ..
                }]
            ),
            "{method}: {answer:?}"
        );
    }
    let cases: [&[(&str, Value)]; 3] = [
        &[],
        &[("history", Value::Text("1".into()))],
        &[("history", Value::Lines(1)), ("extra", Value::Lines(1))],
    ];
    for args in cases {
        let answer = client.answer(block_call(block, "terminal.format", args));
        assert!(
            matches!(answer.as_slice(), [Message::Failed { code: None, .. }]),
            "{args:?}: {answer:?}"
        );
    }
}

#[test]
fn an_oversized_listing_is_refused_and_the_connection_kept() {
    let path = start_server();
    let mut client = Client::connect_with(&path, INSPECT, None);
    // Names are short, but enough sessions exceed the bound on one reply
    // (4 MiB); the requests are sent at once, then confirmed by a sync.
    let name = "a".repeat(rusty_mux_core::MAX_NAME);
    for req in 0..16_000 {
        client.send(&Message::CreateSession {
            req: 1_000_000 + req,
            name: name.clone(),
        });
    }
    assert!(matches!(
        client.answer(|req| Message::Sync { req }).as_slice(),
        [Message::Done { .. }]
    ));
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
        Box::new(move |req| block_call(block, "terminal.title", &[])(req)),
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

const CAPTURE: Features = INSPECT.union(Features::CAPTURE);

fn send_keys(block: BlockId, keys: &[&str]) -> impl FnOnce(u64) -> Message {
    let keys = Value::Words(keys.iter().map(|k| (*k).to_owned()).collect());
    move |req| Message::BlockCall {
        req,
        block,
        method: "terminal.send-keys".into(),
        args: vec![("key".into(), keys)],
    }
}

#[test]
fn keys_reach_the_program_in_its_cursor_mode() {
    let path = start_server();
    let mut client = Client::connect_with(&path, INSPECT, None);
    let block = block_for(&mut client);
    // `cat -v` shows what it reads; DECCKM on turns Up into SS3 A.
    client.input(block, b"printf '\\033[?1h'; cat -v\r");
    client.wait_for(block, "cat -v\r\n");
    let pressed = send_keys(block, &["up", "ctrl+a", "x", "enter", "ctrl+d"]);
    client.send(&pressed(99));
    client.wait_for(block, "^[OA^Ax\r\n");
    let answer = client.answer(send_keys(block, &["up", "hyper+x"]));
    let reason = refusal(answer);
    assert!(reason.contains("`hyper+x` is not a key"), "{reason}");

    let mut other = Client::connect_with(&path, INSPECT, None);
    other.request(|req| Message::Attach { req, block });
    let reason = refusal(other.answer(send_keys(block, &["enter"])));
    assert!(reason.contains("lease"), "{reason}");
}

#[test]
fn capture_options_trim_unwrap_and_keep_styles() {
    let path = start_server();
    let mut client = Client::connect_with(&path, CAPTURE, None);
    let block = block_for(&mut client);
    client.input(
        block,
        b"printf '\\033[H\\033[2J'; printf '\\033[1mBOLD\\033[0m %0100d\\n' 0; echo END$((1+1))\r",
    );
    client.wait_for(block, "END2");
    let format = |client: &mut Client, flags: &[&str]| {
        let mut args = vec![("history", Value::Lines(0))];
        args.extend(flags.iter().map(|f| (*f, Value::Flag(true))));
        text(client, block, "terminal.format", &args)
    };
    let zeros = "0".repeat(100);

    let plain = format(&mut client, &[]);
    assert_eq!(plain.split('\n').count(), 24, "{plain:?}");
    assert!(!plain.contains(&zeros), "{plain:?}");

    let trimmed = format(&mut client, &["trim", "unwrap"]);
    let lines: Vec<&str> = trimmed.lines().collect();
    assert_eq!(lines[..2], [format!("BOLD {zeros}").as_str(), "END2"]);
    assert!(!trimmed.ends_with('\n'), "{trimmed:?}");

    let vt = format(&mut client, &["vt"]);
    assert!(vt.contains("\x1b[0;1mBOLD\x1b[0m 000"), "{vt:?}");

    let mut old = Client::connect_with(&path, INSPECT, None);
    let args = [("history", Value::Lines(0)), ("trim", Value::Flag(true))];
    let reason = refusal(old.answer(block_call(block, "terminal.format", &args)));
    assert!(reason.contains("requires feature bits"), "{reason}");
}
