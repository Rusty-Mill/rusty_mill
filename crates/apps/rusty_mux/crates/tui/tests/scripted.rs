//! The client leaves the local terminal usable however the attach ends:
//! the block closing, the server going away (with no lease, so no key would
//! notice), or Ctrl-] d after the block's program set terminal modes; and
//! it draws a window of several blocks side by side. Each runs the real
//! binary against a scripted server: on a pty on Linux, on our own console
//! host on Windows. The tests against a real server through a stand-in
//! `ssh` stay in `lifecycle.rs`, Linux only: `--ssh` is Unix only.

#![cfg(any(target_os = "linux", windows))]

mod common;

use std::time::{Duration, Instant};

use common::*;
use platform::pty::PtyMaster;
use rusty_mux_core::Size;
use rusty_mux_proto::Features;
use rusty_mux_tui::RESTORE;

#[test]
fn a_view_only_client_exits_and_resets_when_the_server_goes_away() {
    let (socket, server) = scripted_server(false, End::Eof);
    let tui = Tui::start(&socket);
    let screen = tui.exit();
    server.join().expect("server");
    assert_restored(&screen, "lost the server");
}

#[test]
fn a_client_exits_and_resets_when_its_block_closes() {
    let (socket, server) = scripted_server(true, End::Closed);
    let tui = Tui::start(&socket);
    let screen = tui.exit();
    server.join().expect("server");
    assert_restored(&screen, "closed");
}

#[test]
fn detaching_after_the_program_set_modes_resets_the_terminal() {
    let (socket, server) = scripted_server(true, End::Open);
    let tui = Tui::start(&socket);
    tui.wait_for(b"REMOTE");
    tui.master.write(b"\x1dd").expect("ctrl-] d");
    let screen = tui.exit();
    server.join().expect("server");
    assert_restored(&screen, "detached from block");
}

/// The local terminal must keep sending plain keys, or Ctrl-] arrives
/// win32-encoded and passes to the block: the client drops the program's
/// request for the mode.
#[test]
fn win32_input_mode_never_reaches_the_local_terminal() {
    let (socket, server) = scripted_server(true, End::Open);
    let tui = Tui::start(&socket);
    tui.wait_for(SHOWN);
    tui.master.write(b"\x1dd").expect("ctrl-] d");
    let screen = tui.exit();
    server.join().expect("server");
    let asked = screen.windows(8).any(|w| w == b"\x1b[?9001h");
    assert!(!asked, "{:?}", String::from_utf8_lossy(&screen));
}

#[test]
fn the_keylog_records_each_keyboard_read_in_hex() {
    let log = keylog_path("new");
    let _ = std::fs::remove_file(&log);
    let (socket, server) = scripted_server(true, End::Open);
    let tui = Tui::spawn(&socket, BLOCK, keylog_env(&log));
    tui.wait_for(b"REMOTE");
    tui.master.write(b"\x1dd").expect("ctrl-] d");
    let screen = tui.exit();
    server.join().expect("server");
    let logged = std::fs::read_to_string(&log).expect("keylog");
    assert_eq!(logged, "1d 64\n", "{logged:?}");
    // Windows' owner-only DACL is checked by `create_private`'s own test.
    #[cfg(unix)]
    {
        let mode = std::os::unix::fs::PermissionsExt::mode(
            &std::fs::metadata(&log).expect("stat").permissions(),
        );
        assert_eq!(mode & 0o777, 0o600, "{mode:o}");
    }
    let warned = String::from_utf8_lossy(&screen).contains("passwords included");
    assert!(warned, "{:?}", String::from_utf8_lossy(&screen));
    std::fs::remove_file(&log).expect("cleanup");
}

/// The keys must never land in a file another user may already read.
#[test]
fn the_keylog_refuses_an_existing_file() {
    let log = keylog_path("existing");
    std::fs::write(&log, "kept\n").expect("existing log");
    let (socket, _server) = scripted_server(true, End::Open);
    let screen = Tui::spawn(&socket, BLOCK, keylog_env(&log)).exit();
    let text = String::from_utf8_lossy(&screen);
    assert!(text.contains("RUSTY_MUX_KEYLOG"), "{text:?}");
    assert_eq!(std::fs::read_to_string(&log).expect("log"), "kept\n");
    std::fs::remove_file(&log).expect("cleanup");
}

/// `--ssh` runs the forward, connects through it, and on detach stops it
/// and removes its socket's directory. The stand-in `ssh` records its
/// arguments and pid, then "forwards" by linking the local path to the
/// server's socket.
#[test]
fn a_window_of_two_blocks_is_drawn_side_by_side() {
    let (socket, server) = layout_server(Script {
        both: true,
        ..Script::default()
    });
    let tui = Tui::start(&socket);
    tui.wait_for(b"LEFT");
    tui.wait_for(b"RIGHT");
    tui.wait_for("│".as_bytes());
    let text = detach(tui);
    // Only the focused block, which holds the lease, fits its half.
    let resized = server.log.lock().expect("lock").resized.clone();
    assert_eq!(resized, vec![(id(BLOCK), Size::new(40, 24).expect("size"))]);
    let find = |needle: &str| text.rfind(needle);
    let alternate = find("\x1b[?1049h").unwrap_or_else(|| panic!("no alternate screen: {text:?}"));
    let restore = find(std::str::from_utf8(RESTORE).expect("utf-8"))
        .unwrap_or_else(|| panic!("terminal not reset: {text:?}"));
    let said = find("detached from block").unwrap_or_else(|| panic!("no detach: {text:?}"));
    assert!(alternate < restore && restore < said, "{text:?}");
}

#[test]
fn a_lease_granted_after_the_focus_moved_is_given_back() {
    let (socket, server) = layout_server(Script {
        both: true,
        late_grant: true,
        ..Script::default()
    });
    let tui = Tui::start(&socket);
    tui.wait_for(b"RIGHT");
    // Focus the right block while the left one's lease is still asked for.
    tui.master.write(b"\x1dl").expect("ctrl-] l");
    let deadline = Instant::now() + PATIENCE;
    while !server.released().contains(&id(BLOCK)) {
        assert!(Instant::now() < deadline, "the left lease was kept");
        std::thread::sleep(Duration::from_millis(20));
    }
    let text = detach(tui);
    assert!(text.contains("detached from block"), "{text:?}");
}

#[test]
fn a_block_hidden_and_shown_again_at_once_keeps_the_client() {
    let (socket, _server) = layout_server(Script {
        both: true,
        bounce: true,
        ..Script::default()
    });
    let tui = Tui::start(&socket);
    tui.wait_for(b"RIGHT");
    // The right block's first connection ends after its second opens.
    let from = tui.written();
    tui.master.write(b"\x1dz").expect("ctrl-] z");
    let deadline = Instant::now() + PATIENCE;
    while tui.since(from).windows(5).filter(|w| w == b"RIGHT").count() == 0 {
        assert!(
            Instant::now() < deadline,
            "the right block was not shown again"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(300));
    let text = detach(tui);
    assert!(text.contains("detached from block"), "{text:?}");
}

#[test]
fn a_closed_window_shows_its_blocks_last_output_first() {
    // Passed through alone, and composed beside a block that never closes.
    for both in [false, true] {
        let (socket, _server) = layout_server(Script {
            both,
            close: true,
            ..Script::default()
        });
        let tui = Tui::start(&socket);
        let text = String::from_utf8_lossy(&tui.exit()).into_owned();
        let restore = text
            .rfind(std::str::from_utf8(RESTORE).expect("utf-8"))
            .unwrap_or_else(|| panic!("terminal not reset: {text:?}"));
        let last = text
            .rfind("FINAL")
            .unwrap_or_else(|| panic!("no last output ({both}): {text:?}"));
        let said = text
            .rfind("closed")
            .unwrap_or_else(|| panic!("no close: {text:?}"));
        assert!(last < restore && restore < said, "{both}: {text:?}");
    }
}

#[test]
fn a_block_dropped_from_the_layout_shows_its_last_output_first() {
    let (socket, _server) = layout_server(Script {
        both: true,
        close_right: true,
        ..Script::default()
    });
    let tui = Tui::start(&socket);
    // The layout drops the right block before its last output arrives.
    tui.wait_for(b"RIGHTBYE");
    let text = detach(tui);
    assert!(text.contains("detached from block"), "{text:?}");
}

#[test]
fn a_block_passed_through_again_gets_its_modes_back() {
    let (socket, _server) = layout_server(Script {
        both: true,
        ..Script::default()
    });
    let tui = Tui::start(&socket);
    tui.wait_for(b"RIGHT");
    // Composing reset the left block's modes; zooming it passes it again.
    let from = tui.written();
    tui.master.write(b"\x1dz").expect("ctrl-] z");
    let deadline = Instant::now() + PATIENCE;
    let passed = loop {
        let since = String::from_utf8_lossy(&tui.since(from)).into_owned();
        if let Some(at) = since.find("\x1b[1;24r") {
            break since[at..].to_owned();
        }
        assert!(Instant::now() < deadline, "not passed through: {since:?}");
        std::thread::sleep(Duration::from_millis(20));
    };
    detach(tui);
    assert!(
        passed.contains("\x1b[?1h") && passed.contains("\x1b[?2004h"),
        "{passed:?}"
    );
}

#[test]
fn a_server_offering_only_v3_checkpoints_is_refused() {
    // An older server's RTCP v3 has no colour provenance; the client
    // refuses it rather than restore its default colours baked in.
    let (socket, listener) = listen();
    let server = std::thread::spawn(move || welcome(&listener, Features::CHECKPOINT_V3));
    let tui = Tui::start(&socket);
    let screen = tui.exit();
    drop(server.join().expect("server"));
    let text = String::from_utf8_lossy(&screen);
    assert!(text.contains("cannot send checkpoints"), "{text:?}");
}
