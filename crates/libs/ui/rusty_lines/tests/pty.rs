//! End-to-end tests: drive `examples/demo` under a real pseudo-terminal,
//! exercising raw mode, escape-sequence decoding, and the repaint path
//! that the unit tests in `src/lib.rs` can't reach. Unix-only, like the
//! raw editor itself.
#![cfg(unix)]

use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn example_path(name: &str) -> std::path::PathBuf {
    // target/debug/deps/pty-<hash> -> target/debug/examples/<name>
    let mut p = std::env::current_exe().unwrap();
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.push("examples");
    p.push(name);
    p
}

fn spawn_example(name: &str, args: &[&str]) -> (OwnedFd, Child) {
    let (master, _slave_for_test, child) = spawn_example_ext(name, args, 80, 24);
    (master, child)
}

/// [`spawn_example`], but with a caller-chosen pty size and a second fd
/// onto the same tty handed back for the test itself to use — e.g.
/// twiddling termios directly (an external `stty`, or a SIGTSTP+`fg`
/// cycle's cooked-mode aftermath) or reading the window size independent
/// of what the child believes it is.
fn spawn_example_ext(name: &str, args: &[&str], cols: u16, rows: u16) -> (OwnedFd, OwnedFd, Child) {
    let mut master: libc::c_int = 0;
    let mut slave: libc::c_int = 0;
    let mut ws = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let rc = unsafe {
        // *mut pointers throughout: macOS's binding takes *mut termios /
        // *mut winsize where Linux takes *const, and *mut coerces to both.
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &raw mut ws,
        )
    };
    assert_eq!(rc, 0, "openpty failed");
    let master = unsafe { OwnedFd::from_raw_fd(master) };
    let slave = unsafe { OwnedFd::from_raw_fd(slave) };
    let slave_for_test = slave.try_clone().unwrap();
    let child = Command::new(example_path(name))
        .args(args)
        .stdin(Stdio::from(slave.try_clone().unwrap()))
        .stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::from(slave))
        .spawn()
        .expect("spawn demo (is it built? cargo test builds examples)");
    (master, slave_for_test, child)
}

/// Whether `ECHO` is currently set on the tty `fd` refers to — polled
/// from the test side, independent of the child's own view of its
/// terminal.
fn termios_echo_on(fd: &OwnedFd) -> bool {
    let mut term: libc::termios = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::tcgetattr(fd.as_raw_fd(), &mut term) };
    assert_eq!(rc, 0, "tcgetattr failed");
    term.c_lflag & libc::ECHO != 0
}

/// Read whatever the pty has within `timeout_ms`, appending to `out`.
/// Returns false on EOF/EIO (slave side fully closed).
fn drain(master: &OwnedFd, out: &mut Vec<u8>, timeout_ms: i32) -> bool {
    let mut pfd = libc::pollfd {
        fd: master.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let n = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
    if n <= 0 || pfd.revents & libc::POLLIN == 0 {
        return true; // nothing available right now
    }
    let mut buf = [0u8; 4096];
    let r = unsafe { libc::read(master.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
    if r <= 0 {
        return false;
    }
    out.extend_from_slice(&buf[..r as usize]);
    true
}

/// Wait until the editor has painted a fresh, empty prompt. This is the
/// raw-mode synchronization point: the prompt is only painted once
/// read_line has the terminal set up, so keystrokes sent after it can't
/// be swallowed by the cooked-mode tty driver (on macOS the driver owns
/// C-w/C-y itself — VWERASE/VDSUSP — which corrupted unsynchronized runs).
fn wait_for_prompt(master: &OwnedFd, out: &mut Vec<u8>, deadline: Instant, prompt: &str) {
    loop {
        // The paint is `\r ESC[J <prompt> <buffer> \r ESC[<col>G`: with an
        // empty buffer, the text ends with the prompt + \r after
        // ANSI-stripping.
        if strip_ansi(out).trim_end_matches('\r').ends_with(prompt) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "no empty prompt; output so far:\n{}",
            strip_ansi(out)
        );
        drain(master, out, 100);
    }
}

/// Feed each chunk to the pty (whole, so escape sequences are never
/// split), waiting for the prompt before each chunk that starts a new
/// line; append Ctrl-D to exit the demo, and collect everything the
/// editor wrote until the child exits.
fn run_session(chunks: &[&[u8]]) -> String {
    run_session_in("demo", "demo> ", chunks)
}

fn run_session_in(example: &str, prompt: &str, chunks: &[&[u8]]) -> String {
    let (master, mut child) = spawn_example(example, &[]);
    let mut m = std::fs::File::from(master.try_clone().unwrap());
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut out = Vec::new();
    let mut at_line_start = true;
    for chunk in chunks {
        if at_line_start {
            wait_for_prompt(&master, &mut out, deadline, prompt);
        }
        m.write_all(chunk).unwrap();
        m.flush().unwrap();
        // Let the editor drain this chunk before the next one, so a
        // trailing ESC is never glued to the next chunk's bytes.
        std::thread::sleep(Duration::from_millis(100));
        drain(&master, &mut out, 0);
        // Enter and Ctrl-C both end the line and repaint a fresh prompt.
        at_line_start = matches!(chunk.last(), Some(b'\r' | b'\x03'));
    }
    wait_for_prompt(&master, &mut out, deadline, prompt);
    m.write_all(b"\x04").unwrap(); // Ctrl-D on the empty line: exit
    m.flush().unwrap();

    let mut exited = false;
    loop {
        if !drain(&master, &mut out, 200) {
            break; // EOF/EIO: slave side fully closed
        }
        if exited {
            break; // child gone and the pty has drained
        }
        if child.try_wait().unwrap().is_some() {
            exited = true;
        }
        assert!(
            Instant::now() < deadline,
            "demo did not exit; output so far:\n{out:?}"
        );
    }
    let status = child.wait().unwrap();
    assert!(status.success(), "demo exited with {status}");
    strip_ansi(&out)
}

/// Drop ESC-introduced sequences (CSI, OSC, two-byte) so assertions see
/// only printable text and \r\n structure.
fn strip_ansi(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    let mut out = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('[') => {
                // CSI: parameters, then a final byte in @..~
                for c in it.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                // OSC: terminated by BEL or ST
                while let Some(c) = it.next() {
                    if c == '\x07' {
                        break;
                    }
                    if c == '\x1b' && it.peek() == Some(&'\\') {
                        it.next();
                        break;
                    }
                }
            }
            _ => {} // two-byte sequence: drop the introducer + one char
        }
    }
    out
}

/// An executed line is echoed by the demo at column 0, right after the
/// editor's own final "\r\n" — a signature repaints never produce.
fn echo(line: &str) -> String {
    format!("\r\n{line}\r\n")
}

#[test]
fn types_a_line_and_gets_it_echoed() {
    let out = run_session(&[b"hello\r"]);
    assert!(out.contains("demo> "), "prompt missing:\n{out}");
    assert!(out.contains(&echo("hello")), "echo missing:\n{out}");
}

#[test]
fn emacs_editing_home_and_delete() {
    // "xhello", C-a to column 0, C-d deletes the stray 'x'.
    let out = run_session(&[b"xhello", b"\x01", b"\x04", b"\r"]);
    assert!(out.contains(&echo("hello")), "edited line wrong:\n{out}");
}

#[test]
fn kill_word_then_yank() {
    // C-w kills "bar" into the kill ring, C-y yanks it back.
    let out = run_session(&[b"foo bar", b"\x17", b"\x19", b"\r"]);
    assert!(out.contains(&echo("foo bar")), "yank result wrong:\n{out}");
}

#[test]
fn up_arrow_recalls_history() {
    let out = run_session(&[b"one\r", b"two\r", b"\x1b[A\x1b[A", b"\r"]);
    let hits = out.matches(&echo("one")).count();
    assert!(
        hits >= 2,
        "expected 'one' echoed twice (typed + recalled):\n{out}"
    );
}

#[test]
fn ctrl_o_operate_and_get_next_replays_history() {
    // Recall "one" with two Ups, then C-o: it executes "one" and
    // pre-loads "two" on the next prompt, so a bare Enter replays it.
    let out = run_session(&[b"one\r", b"two\r", b"\x1b[A\x1b[A", b"\x0f", b"\r"]);
    assert!(
        out.matches(&echo("one")).count() >= 2,
        "C-o did not execute the recalled line:\n{out}"
    );
    assert!(
        out.matches(&echo("two")).count() >= 2,
        "next history entry not pre-loaded after C-o:\n{out}"
    );
}

#[test]
fn ctrl_c_interrupts_the_line() {
    let out = run_session(&[b"doomed", b"\x03"]);
    assert!(out.contains("^C"), "^C marker missing:\n{out}");
    assert!(
        !out.contains(&echo("doomed")),
        "interrupted line was executed:\n{out}"
    );
}

#[test]
fn ctrl_l_clears_screen_and_keeps_the_line() {
    // C-l mid-line: the screen clears but the buffer and cursor survive.
    let out = run_session(&[b"abc", b"\x0c", b"def", b"\r"]);
    assert!(
        out.contains(&echo("abcdef")),
        "line lost across C-l:\n{out}"
    );
}

#[test]
fn bracketed_paste_inserts_literally() {
    // A pasted tab must insert, not trigger completion.
    let out = run_session(&[b"\x1b[200~a\tb\x1b[201~", b"\r"]);
    assert!(out.contains(&echo("a\tb")), "paste not literal:\n{out}");
}

#[test]
fn tab_menu_cycles_candidates() {
    // "al" → Tab inserts the LCP "alpha"; Tab again lists and arms the
    // menu; further Tabs cycle: alpha (candidate 0), then alphabet.
    let out = run_session_in(
        "hooked",
        "hooked> ",
        &[b"al", b"\t", b"\t", b"\t", b"\t", b"\r"],
    );
    assert!(
        out.contains("alphanumeric"),
        "candidate list missing:\n{out}"
    );
    assert!(out.contains(&echo("alphabet")), "menu cycle wrong:\n{out}");
}

#[test]
fn right_arrow_accepts_full_hint() {
    let out = run_session_in(
        "hooked",
        "hooked> ",
        &[b"alpha beta\r", b"alp", b"\x1b[C", b"\r"],
    );
    let hits = out.matches(&echo("alpha beta")).count();
    assert!(hits >= 2, "hint not accepted (typed + hinted):\n{out}");
}

#[test]
fn alt_f_accepts_one_word_of_hint() {
    let out = run_session_in(
        "hooked",
        "hooked> ",
        &[b"alpha beta\r", b"al", b"\x1bf", b"\r"],
    );
    assert!(
        out.contains(&echo("alpha")),
        "partial hint accept wrong:\n{out}"
    );
}

#[test]
fn rebound_key_runs_the_new_action() {
    // hooked rebinds C-o to unix-line-discard: "abc" C-o "def" leaves
    // only "def".
    let out = run_session_in("hooked", "hooked> ", &[b"abc", b"\x0f", b"def", b"\r"]);
    assert!(out.contains(&echo("def")), "rebinding ignored:\n{out}");
    assert!(
        !out.contains(&echo("abcdef")),
        "rebound key did nothing:\n{out}"
    );
}

#[test]
fn host_binding_rewrites_the_line() {
    // hooked binds C-g to a host command that uppercases the line
    // (bash `bind -x`): the edited buffer must survive the raw-mode
    // suspend/resume round trip.
    let out = run_session_in("hooked", "hooked> ", &[b"abc", b"\x07", b"\r"]);
    assert!(out.contains(&echo("ABC")), "host binding missing:\n{out}");
}

#[test]
fn burst_ending_in_host_binding_paints_correctly() {
    // "abc" + C-g delivered in ONE write (one pty read on the editor's
    // side): the self-inserts coalesce their repaints, and the host
    // binding — reached via a call nested inside run_action, not the
    // main loop — must still see a freshly painted, uncorrupted
    // terminal when it suspends raw mode. A stale paint here would
    // desync `finish_line`'s cursor math and corrupt the display.
    let out = run_session_in("hooked", "hooked> ", &[b"abc\x07", b"\r"]);
    assert!(
        out.contains(&echo("ABC")),
        "burst-then-host-binding corrupted:\n{out}"
    );
}

#[test]
fn burst_ending_in_tab_completion_paints_correctly() {
    // "al" + Tab in one write: the completion candidate list is printed
    // by a function nested inside run_action (list_candidates), which
    // also must see a fresh paint, not a coalesced/stale one.
    let out = run_session_in("hooked", "hooked> ", &[b"al\t", b"\r"]);
    assert!(
        out.contains(&echo("alpha")),
        "burst-then-completion corrupted:\n{out}"
    );
}

#[test]
fn long_burst_into_enter_paints_correctly() {
    // A long run of coalesced self-inserts (many skipped repaints in a
    // row) immediately followed by Enter in the same write: the final
    // `finish_line` must flush the whole deferred run, not just the
    // last key, before computing its cursor-movement math.
    let line = "x".repeat(300);
    let chunk = format!("{line}\r");
    let out = run_session(&[chunk.as_bytes()]);
    assert!(
        out.contains(&echo(&line)),
        "long coalesced burst lost or corrupted text"
    );
}

#[test]
fn very_large_flood_completes_and_is_not_lost() {
    // A flood well past the coalescing cap (MAX_COALESCED_RUN = 200):
    // periodic forced paints must not corrupt the final state, and the
    // editor must not hang waiting on anything it shouldn't.
    let line = "z".repeat(3000);
    let chunk = format!("{line}\r");
    let out = run_session(&[chunk.as_bytes()]);
    assert!(
        out.contains(&echo(&line)),
        "flood past the coalescing cap lost or corrupted text"
    );
}

#[test]
fn burst_into_search_paints_correctly() {
    // Seed history with "one", then in a fresh line, burst two junk
    // characters immediately followed — in the SAME write, so the
    // editor reads them in one syscall — by C-r and a matching query.
    // Entering search mid-coalesced-burst takes a different render path
    // (search's own unconditional render, not the main loop's), which
    // must still leave `render_owed` correctly settled by the time
    // Enter accepts the match.
    let out = run_session(&[b"one\r", b"xy\x12on\r"]);
    let hits = out.matches(&echo("one")).count();
    assert!(
        hits >= 2,
        "expected 'one' echoed twice (typed + found via search):\n{out}"
    );
}

#[test]
fn character_search_moves_to_the_typed_char() {
    // C-a to column 0, C-] then 'l' jumps onto the first 'l', C-d
    // deletes it (readline character-search).
    let out = run_session(&[b"hello", b"\x01", b"\x1dl", b"\x04", b"\r"]);
    assert!(
        out.contains(&echo("helo")),
        "character-search wrong:\n{out}"
    );
}

#[test]
fn possible_completions_lists_without_editing() {
    // M-? prints the candidate list but leaves the buffer untouched
    // (no LCP insertion, unlike Tab).
    let out = run_session_in("hooked", "hooked> ", &[b"al", b"\x1b?", b"\r"]);
    assert!(
        out.contains("alphanumeric"),
        "candidate list missing:\n{out}"
    );
    assert!(
        out.contains(&echo("al")),
        "buffer was modified by M-?:\n{out}"
    );
}

#[test]
fn wide_char_editing_keeps_cursor_math() {
    // Double-width CJK through the whole pipeline: UTF-8 input assembly,
    // width math, and editing at the start of the line. C-a then C-d
    // deletes the first wide character.
    let out = run_session(&["日本ab".as_bytes(), b"\x01", b"\x04", b"\r"]);
    assert!(out.contains(&echo("本ab")), "wide-char edit wrong:\n{out}");
    // And word ops over multibyte text: C-w kills the accented word.
    let out = run_session(&["héllo wörld".as_bytes(), b"\x17", b"\r"]);
    assert!(
        out.contains(&echo("héllo ")),
        "multibyte word kill wrong:\n{out}"
    );
}

#[test]
fn history_edits_survive_navigation() {
    // Recall "one" (two Ups), append "X", go Up to "two"... wait — Up
    // from the older entry goes older; use Down then Up instead: edit
    // "one", Down to "two", Up again — the edit must still be there
    // (zsh keeps in-session edits until accept).
    let out = run_session(&[
        b"one\r",
        b"two\r",
        b"\x1b[A\x1b[A", // Up Up -> "one"
        b"X",            // edit it -> "oneX"
        b"\x1b[B\x1b[A", // Down to "two", back Up
        b"\r",
    ]);
    assert!(
        out.contains(&echo("oneX")),
        "history edit lost on navigation:\n{out}"
    );
}

#[test]
fn vi_normal_mode_edits_and_shows_the_mode() {
    // "xhello", Esc into normal mode, `0` to column 0, `x` deletes the
    // stray character, Enter accepts from normal mode.
    let out = run_session_in("vi", "vi> ", &[b"xhello", b"\x1b", b"0x", b"\r"]);
    assert!(out.contains(&echo("hello")), "vi edit wrong:\n{out}");
    assert!(
        out.contains("(cmd)"),
        "mode indicator missing after Esc:\n{out}"
    );
    assert!(
        out.contains("(ins)"),
        "insert-mode indicator missing:\n{out}"
    );
}

#[test]
fn vi_daw_deletes_a_word_object() {
    // Esc leaves the cursor on the last char of "three"; `bb` walks back
    // to "two"; `daw` deletes it and its trailing space.
    let out = run_session_in(
        "vi",
        "vi> ",
        &[b"one two three", b"\x1b", b"bb", b"daw", b"\r"],
    );
    assert!(out.contains(&echo("one three")), "daw wrong:\n{out}");
}

#[test]
fn vi_count_replace() {
    // `3rx` replaces three characters with xxx (vim count semantics).
    let out = run_session_in("vi", "vi> ", &[b"abc", b"\x1b", b"0", b"3rx", b"\r"]);
    assert!(out.contains(&echo("xxx")), "3rx wrong:\n{out}");
}

#[test]
fn multiline_prompt_prefix_paints_once_per_region() {
    // The vi example's prompt is "vi demo\nvi> ". The prefix line must
    // paint once per region — not once per keystroke, which is what the
    // old row accounting degenerated to with a '\n' in the prompt.
    let out = run_session_in("vi", "vi> ", &[b"abcde", b"\r"]);
    assert!(out.contains(&echo("abcde")), "line lost:\n{out}");
    let prefixes = out.matches("vi demo").count();
    assert!(
        prefixes <= 3,
        "prompt prefix repainted per keystroke ({prefixes} times):\n{out}"
    );
}

#[test]
fn read_line_with_initial_seeds_buffer_and_cursor() {
    // The initial example pre-seeds "hello " ∥ "world" with the cursor at
    // the split; typing X then Enter must yield "hello Xworld".
    let (master, mut child) = spawn_example("initial", &[]);
    let mut m = std::fs::File::from(master.try_clone().unwrap());
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut out = Vec::new();
    loop {
        if strip_ansi(&out).contains("hello world") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "seeded line never painted:\n{}",
            strip_ansi(&out)
        );
        drain(&master, &mut out, 100);
    }
    m.write_all(b"X\r").unwrap();
    m.flush().unwrap();
    loop {
        if !drain(&master, &mut out, 200) || child.try_wait().unwrap().is_some() {
            break;
        }
        assert!(Instant::now() < deadline, "initial demo did not exit");
    }
    child.wait().unwrap();
    drain(&master, &mut out, 200);
    let out = strip_ansi(&out);
    assert!(
        out.contains(&echo("hello Xworld")),
        "cursor not at the seam:\n{out}"
    );
}

#[test]
fn rprompt_paints_and_hides_when_the_line_grows_into_it() {
    // The rprompt example paints "RIGHT" at the row's right edge while
    // the line is short, and hides it (zsh-style) once the buffer grows
    // into it: at 80 columns, prompt "rp> " (4) + 72 chars + gap leaves
    // no room for the 5-wide rprompt.
    let (master, mut child) = spawn_example("rprompt", &[]);
    let mut m = std::fs::File::from(master.try_clone().unwrap());
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut out = Vec::new();
    loop {
        if strip_ansi(&out).contains("RIGHT") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "rprompt never painted:\n{}",
            strip_ansi(&out)
        );
        drain(&master, &mut out, 100);
    }
    let long = "a".repeat(72);
    m.write_all(long.as_bytes()).unwrap();
    m.write_all(b"\r").unwrap();
    m.flush().unwrap();
    loop {
        if !drain(&master, &mut out, 200) || child.try_wait().unwrap().is_some() {
            break;
        }
        assert!(Instant::now() < deadline, "rprompt demo did not exit");
    }
    child.wait().unwrap();
    drain(&master, &mut out, 200);
    let out = strip_ansi(&out);
    assert!(out.contains(&echo(&long)), "line mangled:\n{out}");
    // Repaint rows are separated by \r; any row holding the full-length
    // buffer must no longer carry the rprompt.
    let full_rows: Vec<&str> = out.split('\r').filter(|seg| seg.contains(&long)).collect();
    assert!(
        !full_rows.is_empty(),
        "no full-length repaint captured:\n{out}"
    );
    for row in full_rows {
        assert!(
            !row.contains("RIGHT"),
            "rprompt not hidden on a full row: {row:?}"
        );
    }
}

#[test]
fn read_line_timeout_expires_at_the_prompt() {
    // Sit at the timeout example's prompt without typing: the 2s deadline
    // must fire, print bash's message, and exit cleanly.
    let (master, mut child) = spawn_example("timeout", &[]);
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut out = Vec::new();
    wait_for_prompt(&master, &mut out, deadline, "timeout> ");
    loop {
        if !drain(&master, &mut out, 200) || child.try_wait().unwrap().is_some() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "timeout demo did not exit:\n{}",
            strip_ansi(&out)
        );
    }
    let status = child.wait().unwrap();
    assert!(status.success(), "timeout demo exited with {status}");
    drain(&master, &mut out, 200);
    let out = strip_ansi(&out);
    assert!(
        out.contains("timed out waiting for input"),
        "timeout message missing:\n{out}"
    );
}

#[test]
fn resize_while_idle_repaints_and_keeps_the_line() {
    // Type half a line, shrink the terminal while the editor idles at
    // the prompt (no keystroke pending), then finish the line: the idle
    // poll tick must notice the new width and nothing may be lost.
    let (master, mut child) = spawn_example("demo", &[]);
    let mut m = std::fs::File::from(master.try_clone().unwrap());
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut out = Vec::new();
    wait_for_prompt(&master, &mut out, deadline, "demo> ");
    m.write_all(b"abc").unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let ws = libc::winsize {
        ws_row: 24,
        ws_col: 40,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let rc = unsafe { libc::ioctl(master.as_raw_fd(), libc::TIOCSWINSZ, &ws) };
    assert_eq!(rc, 0, "TIOCSWINSZ failed");
    std::thread::sleep(Duration::from_millis(500)); // > one idle tick
    m.write_all(b"def\r").unwrap();
    std::thread::sleep(Duration::from_millis(100));
    wait_for_prompt(&master, &mut out, deadline, "demo> ");
    m.write_all(b"\x04").unwrap();
    loop {
        if !drain(&master, &mut out, 200) || child.try_wait().unwrap().is_some() {
            break;
        }
        assert!(Instant::now() < deadline, "demo did not exit");
    }
    child.wait().unwrap();
    let out = strip_ansi(&out);
    assert!(
        out.contains(&echo("abcdef")),
        "line lost across resize:\n{out}"
    );
}

#[test]
fn prepare_external_output_prints_cleanly_while_idle_and_keeps_the_line() {
    // Type half a line, then — without pressing Enter — let a hook print a
    // notice via `prepare_external_output` from the idle tick: the same
    // "idle at the prompt, an external event needs to interrupt the
    // paint" shape `resize_while_idle_repaints_and_keeps_the_line` covers
    // for a resize, exercised here for a hook's own printed output
    // instead. Proves both that the notice lands on its own line (not
    // glued onto the in-progress buffer) and that the buffer itself
    // survives intact once the line is finished.
    let trigger =
        std::env::temp_dir().join(format!("rusty_lines_notify_trigger_{}", std::process::id()));
    let _ = std::fs::remove_file(&trigger);
    let (master, mut child) = spawn_example("notify", &[trigger.to_str().unwrap()]);
    let mut m = std::fs::File::from(master.try_clone().unwrap());
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut out = Vec::new();
    wait_for_prompt(&master, &mut out, deadline, "notify> ");
    m.write_all(b"abc").unwrap();
    std::thread::sleep(Duration::from_millis(100));
    std::fs::write(&trigger, b"").unwrap(); // fires the hook's next idle tick
    std::thread::sleep(Duration::from_millis(500)); // > one idle tick
    m.write_all(b"def\r").unwrap();
    std::thread::sleep(Duration::from_millis(100));
    wait_for_prompt(&master, &mut out, deadline, "notify> ");
    m.write_all(b"\x04").unwrap();
    loop {
        if !drain(&master, &mut out, 200) || child.try_wait().unwrap().is_some() {
            break;
        }
        assert!(Instant::now() < deadline, "notify did not exit");
    }
    child.wait().unwrap();
    let _ = std::fs::remove_file(&trigger);
    let out = strip_ansi(&out);
    assert!(
        out.contains("\r\n[1]+  Done\tsleep 5\r\n"),
        "notice missing or not on its own line:\n{out}"
    );
    assert!(
        out.contains(&echo("abcdef")),
        "line lost around external output:\n{out}"
    );
}

#[test]
fn quoted_insert_and_edit_in_editor_pty_smoke() {
    // quoted_insert (C-v/C-q) itself is unit-tested in src/lib.rs against
    // synthetic stdin; this just confirms it also works end-to-end under
    // a real pty: C-v C-a inserts the literal ^A byte instead of running
    // beginning-of-line, and the render shows it ^X-style.
    let out = run_session_in("demo", "demo> ", &[b"x\x16\x01y", b"\r"]);
    assert!(
        out.contains("x^Ay"),
        "quoted-insert did not render the literal control byte ^X-style:\n{out}"
    );
}

#[test]
fn edit_in_editor_rewrites_the_line_via_external_editor() {
    // hooked's external_editor hook deterministically overwrites the
    // tempfile; C-x C-e must hand the buffer to it and accept whatever
    // comes back, exactly like accepting the line normally.
    let out = run_session_in("hooked", "hooked> ", &[b"orig", b"\x18\x05"]);
    assert!(
        out.contains(&echo("EDITED-VIA-EXTERNAL")),
        "external editor's rewrite was not accepted as the line:\n{out}"
    );
}

#[test]
fn read_line_with_initial_timeout_preserves_seeded_and_edited_buffer() {
    // The combined seed+deadline variant: type into the seeded buffer,
    // then let the deadline expire without pressing Enter. TimedOut
    // itself carries no buffer (see ReadResult::TimedOut), but the
    // seeded-then-edited text must still be the last thing painted.
    let (master, mut child) = spawn_example("initial_timeout", &[]);
    let mut m = std::fs::File::from(master.try_clone().unwrap());
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut out = Vec::new();
    loop {
        if strip_ansi(&out).contains("hello world") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "seeded line never painted:\n{}",
            strip_ansi(&out)
        );
        drain(&master, &mut out, 100);
    }
    m.write_all(b"X").unwrap(); // edit the seeded buffer; never press Enter
    m.flush().unwrap();
    loop {
        if strip_ansi(&out).contains("hello Xworld") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "edit never painted:\n{}",
            strip_ansi(&out)
        );
        drain(&master, &mut out, 100);
    }
    loop {
        if !drain(&master, &mut out, 200) || child.try_wait().unwrap().is_some() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "initial_timeout demo did not exit:\n{}",
            strip_ansi(&out)
        );
    }
    let status = child.wait().unwrap();
    assert!(
        status.success(),
        "initial_timeout demo exited with {status}"
    );
    drain(&master, &mut out, 200);
    let out = strip_ansi(&out);
    assert!(
        out.contains("hello Xworld"),
        "seeded+edited buffer not preserved on screen at the deadline:\n{out}"
    );
    assert!(
        out.contains("timed out waiting for input"),
        "TimedOut message missing:\n{out}"
    );
}

#[test]
fn history_past_end_rings_the_default_audible_bell() {
    // BellStyle::Audible is the default; Up-arrow before anything is
    // typed hits history_prev's "nothing to recall" bell. A bare BEL
    // isn't ESC-introduced, so it survives ANSI-stripping untouched —
    // this also exercises BellStyle::Audible for item 11(c).
    let (master, mut child) = spawn_example("demo", &[]);
    let mut m = std::fs::File::from(master.try_clone().unwrap());
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut out = Vec::new();
    wait_for_prompt(&master, &mut out, deadline, "demo> ");
    m.write_all(b"\x1b[A").unwrap();
    std::thread::sleep(Duration::from_millis(150));
    drain(&master, &mut out, 0);
    assert!(
        strip_ansi(&out).contains('\x07'),
        "no audible BEL byte on history past-end:\n{}",
        strip_ansi(&out)
    );
    m.write_all(b"\x04").unwrap();
    loop {
        if !drain(&master, &mut out, 200) || child.try_wait().unwrap().is_some() {
            break;
        }
        assert!(Instant::now() < deadline, "demo did not exit");
    }
    child.wait().unwrap();
}

#[test]
fn bell_style_visible_flashes_reverse_video() {
    // BellStyle::Visible writes CSI ?5h / ?5l (a reverse-video flash) —
    // both get eaten by ANSI-stripping, so this reads the pty's raw
    // bytes directly instead of going through strip_ansi.
    let (master, mut child) = spawn_example("bell_visible", &[]);
    let mut m = std::fs::File::from(master.try_clone().unwrap());
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut out = Vec::new();
    wait_for_prompt(&master, &mut out, deadline, "bell> ");
    m.write_all(b"\x1b[A").unwrap();
    std::thread::sleep(Duration::from_millis(250)); // the flash itself holds ~80ms
    drain(&master, &mut out, 0);
    let raw = String::from_utf8_lossy(&out);
    assert!(
        raw.contains("\x1b[?5h"),
        "reverse-video set sequence missing:\n{raw}"
    );
    assert!(
        raw.contains("\x1b[?5l"),
        "reverse-video unset sequence missing:\n{raw}"
    );
    m.write_all(b"\x04").unwrap();
    loop {
        if !drain(&master, &mut out, 200) || child.try_wait().unwrap().is_some() {
            break;
        }
        assert!(Instant::now() < deadline, "bell_visible demo did not exit");
    }
    child.wait().unwrap();
}

#[test]
fn completion_query_items_prompts_above_threshold_and_y_shows_the_list() {
    let out = run_session_in("query_items", "qi> ", &[b"\t", b"y", b"\r"]);
    assert!(
        out.contains("Display all 3 possibilities? (y or n)"),
        "completion-query-items prompt missing:\n{out}"
    );
    assert!(
        out.contains("alpha") && out.contains("beta") && out.contains("gamma"),
        "candidate list missing after answering y:\n{out}"
    );
}

#[test]
fn completion_query_items_declining_hides_the_list() {
    let out = run_session_in("query_items", "qi> ", &[b"\t", b"n", b"\r"]);
    assert!(
        out.contains("Display all 3 possibilities? (y or n)"),
        "completion-query-items prompt missing:\n{out}"
    );
    assert!(
        !out.contains("alpha"),
        "candidate list printed despite declining:\n{out}"
    );
}

#[test]
fn candidate_columns_print_column_major_not_row_major() {
    // 8 same-width, no-common-prefix candidates wrap into 4 columns x 2
    // rows at 80 columns. Column-major order is 0,2,4,6,1,3,5,7 — so "g"
    // (index 6) must appear before "b" (index 1) in the printed text;
    // row-major would print "b" (row 0) before "g" (row 1).
    let out = run_session_in("columns", "cols> ", &[b"\t", b"\r"]);
    let pos_b = out
        .find("b-marker-item-01")
        .expect("candidate b missing from the list");
    let pos_g = out
        .find("g-marker-item-06")
        .expect("candidate g missing from the list");
    assert!(
        pos_g < pos_b,
        "candidates were not laid out column-major:\n{out}"
    );
}

#[test]
fn bracketed_paste_multiline_end_to_end() {
    // A multi-line bracketed paste: the embedded newline must render as
    // ⏎ live (not break the edit region), the whole block inserts as one
    // literal unit (not two executed lines), and — once accepted —
    // recalling it from history shows the "; "-joined form (src/lib.rs's
    // multi-line history entries).
    let (master, mut child) = spawn_example("demo", &[]);
    let mut m = std::fs::File::from(master.try_clone().unwrap());
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut out = Vec::new();
    wait_for_prompt(&master, &mut out, deadline, "demo> ");

    m.write_all(b"\x1b[200~line1\nline2\x1b[201~").unwrap();
    m.flush().unwrap();
    std::thread::sleep(Duration::from_millis(150));
    drain(&master, &mut out, 0);
    let live = strip_ansi(&out);
    assert!(
        live.contains('⏎'),
        "pasted newline not shown as ⏎ in the live display:\n{live}"
    );
    assert!(
        !live.contains(&echo("line1")),
        "paste executed line-by-line instead of inserting as one literal unit:\n{live}"
    );

    m.write_all(b"\r").unwrap();
    m.flush().unwrap();
    std::thread::sleep(Duration::from_millis(150));
    wait_for_prompt(&master, &mut out, deadline, "demo> ");
    let after_accept = strip_ansi(&out);
    let idx1 = after_accept
        .find("line1")
        .expect("line1 missing after accept");
    let idx2 = after_accept
        .find("line2")
        .expect("line2 missing after accept");
    assert!(idx2 > idx1, "pasted lines out of order:\n{after_accept}");
    assert!(
        !after_accept[idx1..idx2].contains("demo> "),
        "a prompt reappeared between the two pasted lines — executed separately:\n{after_accept}"
    );

    // Recall it: the in-memory history entry is joined with "; ".
    m.write_all(b"\x1b[A\r").unwrap();
    m.flush().unwrap();
    std::thread::sleep(Duration::from_millis(150));
    wait_for_prompt(&master, &mut out, deadline, "demo> ");
    m.write_all(b"\x04").unwrap();
    loop {
        if !drain(&master, &mut out, 200) || child.try_wait().unwrap().is_some() {
            break;
        }
        assert!(Instant::now() < deadline, "demo did not exit");
    }
    child.wait().unwrap();
    let out = strip_ansi(&out);
    assert!(
        out.contains(&echo("line1; line2")),
        "recalled multi-line entry not joined with \"; \":\n{out}"
    );
}

#[test]
fn self_heals_raw_mode_after_external_cooked_mode() {
    // Approximates SIGTSTP + `fg` (the parent shell restores its own
    // cooked termios on continue) or a trap that ran `stty`: something
    // outside the editor leaves the terminal cooked while it idles at
    // the prompt. The idle poll tick must notice and re-assert raw mode
    // before the next keystroke would otherwise be swallowed by the
    // kernel's line-buffered/echoing canonical mode.
    let (master, slave, mut child) = spawn_example_ext("demo", &[], 80, 24);
    let mut m = std::fs::File::from(master.try_clone().unwrap());
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut out = Vec::new();
    wait_for_prompt(&master, &mut out, deadline, "demo> ");
    m.write_all(b"abc").unwrap();
    std::thread::sleep(Duration::from_millis(100));

    let mut term: libc::termios = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::tcgetattr(slave.as_raw_fd(), &mut term) };
    assert_eq!(rc, 0, "tcgetattr failed");
    term.c_lflag |= libc::ICANON | libc::ECHO;
    let rc = unsafe { libc::tcsetattr(slave.as_raw_fd(), libc::TCSANOW, &term) };
    assert_eq!(rc, 0, "tcsetattr failed");

    std::thread::sleep(Duration::from_millis(500)); // > one idle tick
    m.write_all(b"def\r").unwrap();
    std::thread::sleep(Duration::from_millis(100));
    wait_for_prompt(&master, &mut out, deadline, "demo> ");
    m.write_all(b"\x04").unwrap();
    loop {
        if !drain(&master, &mut out, 200) || child.try_wait().unwrap().is_some() {
            break;
        }
        assert!(Instant::now() < deadline, "demo did not exit");
    }
    child.wait().unwrap();
    let out = strip_ansi(&out);
    assert!(
        out.contains(&echo("abcdef")),
        "line lost or corrupted after external cooked-mode reassertion:\n{out}"
    );
}

/// A fresh, empty directory for [`terminal_facilities`]'s trigger-file
/// handshake — a caller-controlled substitute for a fixed sleep, so the
/// test dictates exactly when the example advances to its next phase
/// regardless of how the test binary happens to be scheduled.
fn trigger_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rusty_lines_trig_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Create `dir`'s trigger file and block until `terminal_facilities` has
/// picked it up (deletes it once seen) — the send half of the handshake.
fn release(dir: &std::path::Path, name: &str, deadline: Instant) {
    let path = dir.join(name);
    std::fs::write(&path, b"").unwrap();
    while path.exists() {
        assert!(Instant::now() < deadline, "example never consumed {name}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn terminal_size_reports_the_pty_dimensions() {
    let dir = trigger_dir("size");
    let (master, _slave, mut child) =
        spawn_example_ext("terminal_facilities", &[dir.to_str().unwrap()], 97, 31);
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut out = Vec::new();
    loop {
        if strip_ansi(&out).contains("size:") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "terminal_facilities never reported a size:\n{}",
            strip_ansi(&out)
        );
        drain(&master, &mut out, 100);
    }
    // This test only cares about the reported size; release the example
    // through both of its (unconditional) trigger gates so it can exit.
    release(&dir, "go1", deadline);
    release(&dir, "go2", deadline);
    loop {
        if !drain(&master, &mut out, 200) || child.try_wait().unwrap().is_some() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "terminal_facilities did not exit"
        );
    }
    let status = child.wait().unwrap();
    assert!(status.success(), "terminal_facilities exited with {status}");
    let out = strip_ansi(&out);
    assert!(
        out.contains("size:97x31"),
        "terminal_size() did not report the configured pty dimensions:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn with_echo_disabled_clears_and_restores_echo_including_after_a_panic() {
    let dir = trigger_dir("echo");
    let (master, slave, mut child) = spawn_example_ext(
        "terminal_facilities",
        &[dir.to_str().unwrap(), "--panic"],
        80,
        24,
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut out = Vec::new();

    // Baseline: before the guard runs (the example is blocked waiting
    // for "go1"), the pty's ECHO flag is on.
    loop {
        if strip_ansi(&out).contains("before-disable") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "example never reached before-disable:\n{}",
            strip_ansi(&out)
        );
        drain(&master, &mut out, 100);
    }
    assert!(
        termios_echo_on(&slave),
        "ECHO expected on before the guard runs"
    );

    // Mid-guard: release it into with_echo_disabled's closure, which
    // prints then blocks on "go2" — still inside the guard's scope.
    release(&dir, "go1", deadline);
    loop {
        if strip_ansi(&out).contains("during-disable") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "example never reached during-disable:\n{}",
            strip_ansi(&out)
        );
        drain(&master, &mut out, 100);
    }
    assert!(
        !termios_echo_on(&slave),
        "ECHO not cleared during with_echo_disabled"
    );

    // Release it out of the closure: the guard drops, restoring echo,
    // before "after-disable" prints and it blocks on "go3".
    release(&dir, "go2", deadline);
    loop {
        if strip_ansi(&out).contains("after-disable") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "example never reached after-disable:\n{}",
            strip_ansi(&out)
        );
        drain(&master, &mut out, 100);
    }
    assert!(
        termios_echo_on(&slave),
        "ECHO not restored after with_echo_disabled returns"
    );

    // Mid-guard again, but this time the closure panics instead of
    // returning normally: release into it, then wait for it to print and
    // block on "go4" before panicking.
    release(&dir, "go3", deadline);
    loop {
        if strip_ansi(&out).contains("during-panic") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "example never reached during-panic:\n{}",
            strip_ansi(&out)
        );
        drain(&master, &mut out, 100);
    }
    assert!(
        !termios_echo_on(&slave),
        "ECHO not cleared during the panicking closure"
    );

    release(&dir, "go4", deadline);
    loop {
        if strip_ansi(&out).contains("panicked:true") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "panic was not caught:\n{}",
            strip_ansi(&out)
        );
        drain(&master, &mut out, 100);
    }
    assert!(
        termios_echo_on(&slave),
        "ECHO not restored after the closure panicked (panic-safety broken)"
    );

    loop {
        if !drain(&master, &mut out, 200) || child.try_wait().unwrap().is_some() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "terminal_facilities --panic did not exit"
        );
    }
    let status = child.wait().unwrap();
    assert!(
        status.success(),
        "terminal_facilities --panic exited with {status}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
