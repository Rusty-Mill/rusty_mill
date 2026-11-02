//! Changing and measuring a block's terminal (C40): `terminal.size`,
//! `terminal.reset` and `terminal.set-theme`.

#![cfg(target_os = "linux")]

mod common;

use common::{Client, block_for, start_server};
use rusty_mux_core::{BlockId, Size};
use rusty_mux_proto::schema::Value;
use rusty_mux_proto::{Features, Message};

const CONTROL: Features = Features::CHECKPOINT_V6
    .union(Features::INSPECT)
    .union(Features::BLOCK_CALL)
    .union(Features::TERMINAL_CONTROL);

/// What a reset feeds the terminal: RIS.
const RESET: &str = "\x1bc";

fn call(block: BlockId, method: &str, args: &[(&str, Value)]) -> impl FnOnce(u64) -> Message {
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

fn text(client: &mut Client, block: BlockId, method: &str) -> String {
    match client.answer(call(block, method, &[])).as_slice() {
        [Message::Text { text, .. }] => text.clone(),
        other => panic!("expected Text, got {other:?}"),
    }
}

fn screen(client: &mut Client, block: BlockId) -> String {
    let args = [("history", Value::Lines(1000))];
    match client
        .answer(call(block, "terminal.format", &args))
        .as_slice()
    {
        [Message::Text { text, .. }] => text.clone(),
        other => panic!("expected Text, got {other:?}"),
    }
}

fn done(client: &mut Client, block: BlockId, method: &str, args: &[(&str, Value)]) {
    match client.answer(call(block, method, args)).as_slice() {
        [Message::Done { created: None, .. }] => {}
        other => panic!("{method}: expected Done, got {other:?}"),
    }
}

fn refusal(client: &mut Client, block: BlockId, method: &str, args: &[(&str, Value)]) -> String {
    match client.answer(call(block, method, args)).as_slice() {
        [Message::Failed { reason, .. }] => reason.clone(),
        other => panic!("{method}: expected Failed, got {other:?}"),
    }
}

fn colour(name: &str, value: &str) -> (&'static str, Value) {
    let name = match name {
        "foreground" => "foreground",
        "background" => "background",
        "cursor" => "cursor",
        other => panic!("not a colour parameter: {other}"),
    };
    (name, Value::Text(value.to_owned()))
}

fn palette(entries: &[&str]) -> (&'static str, Value) {
    (
        "palette",
        Value::Words(entries.iter().map(|e| (*e).to_owned()).collect()),
    )
}

/// Leaves the block's terminal inside an OSC string that never ends, so it
/// swallows everything after: the raw output shows the marker, the screen
/// does not.
fn stick(client: &mut Client, block: BlockId) {
    client.input(block, b"printf '\\033]0;stuck'; echo HIDDEN$((1+1))\r");
    client.wait_for(block, "HIDDEN2");
    assert!(
        !screen(client, block).contains("HIDDEN2"),
        "the OSC string swallows later output"
    );
}

#[test]
fn size_follows_the_blocks_resizes() {
    let path = start_server();
    let mut client = Client::connect_with(&path, CONTROL, None);
    let block = block_for(&mut client);
    assert_eq!(text(&mut client, block, "terminal.size"), "80x24");
    let size = Size::new(100, 30).expect("size");
    client.request(|req| Message::Resize { req, block, size });
    assert_eq!(text(&mut client, block, "terminal.size"), "100x30");
}

#[test]
fn reset_clears_the_screen_and_modes_for_the_server_and_every_client() {
    let path = start_server();
    let mut client = Client::connect_with(&path, CONTROL, None);
    let block = block_for(&mut client);
    let mut watcher = Client::connect_with(&path, CONTROL, None);
    watcher.request(|req| Message::Attach { req, block });
    watcher.wait_ready(block);
    // DECCKM on, then a line only the output has (the echo shows `%s`).
    client.input(block, b"printf '\\033[?1hBEFORE%s\\n' $((1+1))\r");
    client.wait_for(block, "BEFORE2");
    watcher.wait_for(block, "BEFORE2");

    done(&mut client, block, "terminal.reset", &[]);

    assert!(!screen(&mut client, block).contains("BEFORE2"));
    watcher.wait_for(block, RESET);
    assert!(
        !watcher.text(block).contains("BEFORE2"),
        "the watcher's copy too"
    );
    // The server's own terminal left application cursor keys: Up is CSI A
    // again, not SS3 A.
    client.input(block, b"cat -v\r");
    client.wait_for(block, "cat -v\r\n");
    let keys = Value::Words(vec!["up".into(), "enter".into()]);
    client.send(&call(block, "terminal.send-keys", &[("key", keys)])(99));
    client.wait_for(block, "^[[A");
}

#[test]
fn reset_recovers_a_terminal_stuck_in_a_sequence() {
    // `printf` text for each state a program can leave the terminal in, and
    // the bytes it writes: strings, an ESC inside a string (the start of
    // its terminator), escapes waiting for a byte, a cut CSI and UTF-8.
    let cases = [
        ("\\033]0;stuck", "\x1b]0;stuck"),
        ("\\033]0;stuck\\033", "\x1b]0;stuck\x1b"),
        ("\\033P1$r", "\x1bP1$r"),
        ("\\033_Gstuck\\033", "\x1b_Gstuck\x1b"),
        ("\\033^stuck", "\x1b^stuck"),
        ("\\033", "\x1b"),
        ("\\033(", "\x1b("),
        ("\\033[3", "\x1b[3"),
    ];
    for (n, (printf, written)) in cases.into_iter().enumerate() {
        let path = start_server();
        let mut client = Client::connect_with(&path, CONTROL, None);
        let block = block_for(&mut client);
        // DECCKM on and a marker, then the unfinished sequence, then `cat`
        // writes nothing until it is interrupted.
        let before = format!("BEFORE{n}");
        let line = format!("printf '\\033[?1hBEFORE%s\\n' {n}; printf '{printf}'; cat\r");
        client.input(block, line.as_bytes());
        client.wait_for(block, &format!("{before}\r\n{written}"));
        assert!(screen(&mut client, block).contains(&before), "{printf}");

        done(&mut client, block, "terminal.reset", &[]);

        assert!(!screen(&mut client, block).contains(&before), "{printf}");
        assert!(
            !client.text(block).contains(&before),
            "{printf}: the client's copy"
        );
        // Out of application cursor keys: Up is CSI A, not SS3 A.
        client.input(block, b"\x03cat -v\r");
        client.wait_for(block, "cat -v\r\n");
        let keys = Value::Words(vec!["up".into(), "enter".into()]);
        client.send(&call(block, "terminal.send-keys", &[("key", keys)])(99));
        client.wait_for(block, "^[[A");
    }
}

#[test]
fn set_theme_sets_the_colours_programs_ask_for() {
    let path = start_server();
    let mut client = Client::connect_with(&path, CONTROL, None);
    let block = block_for(&mut client);
    let mut watcher = Client::connect_with(&path, CONTROL, None);
    watcher.request(|req| Message::Attach { req, block });
    watcher.wait_ready(block);

    let theme = [
        colour("foreground", "#c0c0c0"),
        colour("background", "#102030"),
        colour("cursor", "#FF8000"),
        palette(&["1=#ff0000", "200=#010203"]),
    ];
    done(&mut client, block, "terminal.set-theme", &theme);
    watcher.wait_for(block, "\x1b]11;#102030\x1b\\");

    // The tty echoes the terminal's answers, ESC as `^[`.
    client.input(
        block,
        b"printf '\\033]10;?\\033\\\\\\033]11;?\\033\\\\\\033]12;?\\033\\\\\\033]4;1;?\\033\\\\\\033]4;200;?\\033\\\\'\r",
    );
    client.wait_for(block, "]10;rgb:c0c0/c0c0/c0c0");
    client.wait_for(block, "]11;rgb:1010/2020/3030");
    client.wait_for(block, "]12;rgb:ffff/8080/0000");
    client.wait_for(block, "]4;1;rgb:ffff/0000/0000");
    client.wait_for(block, "]4;200;rgb:0101/0202/0303");

    // `default` puts a colour back to the terminal's own.
    let back = [colour("background", "default"), palette(&["1=default"])];
    done(&mut client, block, "terminal.set-theme", &back);
    client.input(
        block,
        b"\x15printf '\\033]11;?\\033\\\\\\033]4;1;?\\033\\\\'\r",
    );
    client.wait_for(block, "]11;rgb:0000/0000/0000");
    client.wait_for(block, "]4;1;rgb:8080/0000/0000");
}

#[test]
fn set_theme_refuses_what_it_cannot_apply_whole() {
    let path = start_server();
    let mut client = Client::connect_with(&path, CONTROL, None);
    let block = block_for(&mut client);
    for (args, why) in [
        (vec![], "at least one"),
        (vec![colour("background", "red")], "background"),
        (vec![colour("cursor", "#12345")], "cursor"),
        (vec![colour("foreground", "#1234567")], "foreground"),
        (vec![palette(&["256=#000000"])], "palette"),
        (vec![palette(&["1=#000000", "1=#ffffff"])], "palette"),
        (vec![palette(&["1"])], "palette"),
        (vec![palette(&["x=#000000"])], "palette"),
    ] {
        let reason = refusal(&mut client, block, "terminal.set-theme", &args);
        assert!(reason.contains(why), "{args:?}: {reason}");
    }

    // A sequence the program left unfinished would take the theme's bytes
    // as its own, so the theme waits for a whole one; a reset ends it.
    stick(&mut client, block);
    let theme = [colour("background", "#102030")];
    let reason = refusal(&mut client, block, "terminal.set-theme", &theme);
    assert!(reason.contains("unfinished"), "{reason}");
    done(&mut client, block, "terminal.reset", &[]);
    done(&mut client, block, "terminal.set-theme", &theme);
}

#[test]
fn reset_and_set_theme_need_the_lease_and_size_does_not() {
    let path = start_server();
    let mut client = Client::connect_with(&path, CONTROL, None);
    let block = block_for(&mut client);
    let mut other = Client::connect_with(&path, CONTROL, None);
    let theme = [colour("background", "#102030")];
    for attached in [false, true] {
        if attached {
            other.request(|req| Message::Attach { req, block });
        }
        for (method, args) in [
            ("terminal.reset", &[][..]),
            ("terminal.set-theme", &theme[..]),
        ] {
            let reason = refusal(&mut other, block, method, args);
            assert!(
                reason.contains("lease") || reason.contains("attached"),
                "{method}: {reason}"
            );
        }
        assert_eq!(text(&mut other, block, "terminal.size"), "80x24");
    }
}

#[test]
fn a_client_without_the_feature_is_refused() {
    let path = start_server();
    let older = Features::CHECKPOINT_V6
        .union(Features::INSPECT)
        .union(Features::BLOCK_CALL);
    let mut client = Client::connect_with(&path, older, None);
    let block = block_for(&mut client);
    for method in ["terminal.size", "terminal.reset"] {
        let reason = refusal(&mut client, block, method, &[]);
        assert!(
            reason.contains("requires feature bits"),
            "{method}: {reason}"
        );
    }
    let theme = [colour("background", "#102030")];
    let reason = refusal(&mut client, block, "terminal.set-theme", &theme);
    assert!(reason.contains("requires feature bits"), "{reason}");
}
