//! End-to-end behavioral tests for the Windows raw-mode editor, driven via
//! synthetic console input (`rusty_win32::console::write_char_events`)
//! rather than a pseudo-terminal — there is no Windows equivalent of
//! `tests/pty.rs`'s Unix pty harness available here without ConPTY, which
//! `rusty_win32` deliberately doesn't build (it hosts a *child* process's
//! console session, not useful for testing this process's own reads — see
//! that crate's own reasoning). Instead, this drives [`Editor::read_line`]
//! in-process against a real console this test acquires and points the
//! process's own standard handles at, so `term_sys`'s Windows backend
//! (which reads/writes through `io::stdin()`/`io::stdout()`) resolves to
//! it — proven to work by `rusty_win32`'s own
//! `write_char_events_round_trips_through_raw_mode_read` test, verified on
//! real `windows-latest` CI before this file was written.
//!
//! All tests here share one process-wide console (real hardware/CI
//! consoles are a scarce, singular resource — there's no `openpty`-style
//! "make a fresh isolated one" for a normal console the way Unix pty.rs
//! gets one per test), so they run serially behind [`TEST_LOCK`] rather
//! than relying on `cargo test`'s default parallel threads, which would
//! otherwise race on the shared input buffer and std-handle slots.
#![cfg(windows)]

use rusty_lines::{Editor, NoHooks, ReadResult};
use rusty_win32::console;
use std::sync::Mutex;

/// Serializes every test in this file — see the module doc comment.
static TEST_LOCK: Mutex<()> = Mutex::new(());

#[link(name = "kernel32")]
unsafe extern "system" {
    fn AllocConsole() -> i32;
    fn GetStdHandle(std_handle: u32) -> rusty_win32::RawHandle;
    fn SetStdHandle(std_handle: u32, handle: rusty_win32::RawHandle) -> i32;
    fn CreateFileW(
        file_name: *const u16,
        desired_access: u32,
        share_mode: u32,
        security_attributes: *const core::ffi::c_void,
        creation_disposition: u32,
        flags_and_attributes: u32,
        template_file: rusty_win32::RawHandle,
    ) -> rusty_win32::RawHandle;
}

const STD_INPUT_HANDLE: u32 = -10i32 as u32;
const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_SHARE_READ: u32 = 0x0000_0001;
const FILE_SHARE_WRITE: u32 = 0x0000_0002;
const OPEN_EXISTING: u32 = 3;

/// Open a real handle to whatever console this process is attached to —
/// `CreateFileW("CONIN$"/"CONOUT$", …)`, not `GetStdHandle` (which can
/// still point at a redirected/closed handle from before this process
/// acquired or allocated its console — the same quirk `rusty_win32`'s own
/// tests found and worked around the same way).
fn open_console(name: &str) -> Option<rusty_win32::RawHandle> {
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: `wide` is a valid, NUL-terminated UTF-16 string naming a
    // well-known console pseudo-device; the other arguments are
    // documented-valid constants for opening an existing device for
    // read/write, shared.
    let h = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    if h.is_null() || h as isize == -1 {
        None
    } else {
        Some(h)
    }
}

/// Points this process's own std-input/std-output handles at a real
/// console for as long as it's alive, then restores the previous stdout
/// handle on drop.
///
/// The restore matters: the test harness itself prints each test's
/// `... ok`/`... FAILED` line (and the final summary) through this same
/// process-wide `io::stdout()`, so if it stayed pointed at the console for
/// the rest of the process's life, that output would silently vanish from
/// the CI-captured log instead of reporting which test failed and why.
struct ConsoleGuard {
    stdin: rusty_win32::RawHandle,
    prev_stdout: rusty_win32::RawHandle,
}

impl Drop for ConsoleGuard {
    fn drop(&mut self) {
        // SAFETY: `prev_stdout` is the handle `GetStdHandle` returned
        // before this guard overwrote it, valid for the lifetime of the
        // process.
        unsafe { SetStdHandle(STD_OUTPUT_HANDLE, self.prev_stdout) };
    }
}

/// Ensure this process has a real console (allocating one if needed — this
/// sandbox can't confirm ahead of time whether `cargo test`'s
/// `windows-latest` runner already provides one), and point this process's
/// own std-input/std-output handles at it via `SetStdHandle`. Required
/// because `term_sys`'s Windows backend reads/writes through
/// `io::stdin()`/`io::stdout()`, which resolve `GetStdHandle` fresh on
/// every call — without this, they'd keep resolving to whatever (possibly
/// redirected/invalid) handle the process started with, never seeing the
/// console this test synthesizes input into.
fn ensure_console() -> ConsoleGuard {
    // SAFETY: `STD_OUTPUT_HANDLE` is a well-known pseudo-handle constant;
    // this just reads the process's current stdout handle.
    let prev_stdout = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
    let stdin = match (open_console("CONIN$"), open_console("CONOUT$")) {
        (Some(i), Some(o)) => {
            // SAFETY: both handles are real, valid, currently-open console
            // handles just opened above.
            unsafe {
                SetStdHandle(STD_INPUT_HANDLE, i);
                SetStdHandle(STD_OUTPUT_HANDLE, o);
            }
            i
        }
        _ => {
            // SAFETY: `AllocConsole` has no precondition.
            unsafe { AllocConsole() };
            let i = open_console("CONIN$").expect("AllocConsole should provide CONIN$");
            let o = open_console("CONOUT$").expect("AllocConsole should provide CONOUT$");
            // SAFETY: both handles are real, valid, freshly allocated.
            unsafe {
                SetStdHandle(STD_INPUT_HANDLE, i);
                SetStdHandle(STD_OUTPUT_HANDLE, o);
            }
            i
        }
    };
    ConsoleGuard { stdin, prev_stdout }
}

/// Drain and discard any input already queued (e.g. leftover from a
/// previous test sharing this process's one console) so each test starts
/// from a clean input buffer.
fn flush_pending_input(stdin: rusty_win32::RawHandle) {
    let mut buf = [0u8; 256];
    // SAFETY: `stdin` is a valid, real console input handle.
    while matches!(unsafe { console::wait_readable(stdin, 0) }, Ok(true)) {
        // SAFETY: same handle, valid buffer.
        if unsafe { console::read(stdin, &mut buf) }.unwrap_or(0) == 0 {
            break;
        }
    }
}

/// Type `text` into the shared test console, then read one line with a
/// fresh `Editor`. `text` should end with the byte sequence that finishes
/// the read (`"\r"` for Enter, `"\x03"` for Ctrl-C, …) so the read doesn't
/// block waiting for more input that never arrives.
fn read_line_after_typing(text: &str) -> ReadResult {
    let _lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let guard = ensure_console();
    let stdin = guard.stdin;
    flush_pending_input(stdin);
    // SAFETY: `stdin` is a valid, real console input handle this test just
    // acquired/allocated.
    unsafe { console::write_char_events(stdin, text) }.expect("WriteConsoleInputW should succeed");
    let mut ed = Editor::new();
    ed.read_line("prompt> ", "", &NoHooks)
        .expect("read_line should succeed")
}

#[test]
fn types_a_line_and_gets_it_echoed() {
    assert_eq!(
        read_line_after_typing("hello\r"),
        ReadResult::Line("hello".to_string())
    );
}

#[test]
fn backspace_edits_the_line() {
    // "helloo" then Backspace (0x08) removes the extra 'o'.
    assert_eq!(
        read_line_after_typing("helloo\x08\r"),
        ReadResult::Line("hello".to_string())
    );
}

#[test]
fn ctrl_c_interrupts_the_line() {
    assert_eq!(
        read_line_after_typing("partial line\x03"),
        ReadResult::Interrupted
    );
}

#[test]
fn ctrl_d_on_empty_line_is_eof() {
    assert_eq!(read_line_after_typing("\x04"), ReadResult::Eof);
}

#[test]
fn history_recall_via_ctrl_p() {
    let _lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let guard = ensure_console();
    let stdin = guard.stdin;
    flush_pending_input(stdin);
    let mut ed = Editor::new();

    // First line, to seed history.
    // SAFETY: `stdin` is a valid, real console input handle.
    unsafe { console::write_char_events(stdin, "first\r") }
        .expect("WriteConsoleInputW should succeed");
    let first = ed
        .read_line("prompt> ", "", &NoHooks)
        .expect("read_line should succeed");
    assert_eq!(first, ReadResult::Line("first".to_string()));
    ed.add_history_entry("first");

    // Second read: Ctrl-P (0x10) recalls the previous entry, then Enter
    // accepts it as-is.
    // SAFETY: `stdin` is the same valid handle.
    unsafe { console::write_char_events(stdin, "\x10\r") }
        .expect("WriteConsoleInputW should succeed");
    let second = ed
        .read_line("prompt> ", "", &NoHooks)
        .expect("read_line should succeed");
    assert_eq!(second, ReadResult::Line("first".to_string()));
}
