//! Console input on Windows, as rusty_mux's attach client reads it.
//! `Terminal::poll_readable` reports readable only once a read returns a
//! byte: records that carry no character (key-ups, bare modifiers) are
//! dropped first, while the key that types NUL is kept. The client
//! re-checks its own state between waits, so a wake for a key-up would
//! leave it blocked in a read until the next key. `read_chunk` returns
//! UTF-8 whatever the input code page, since the client forwards the bytes
//! to a ConPTY, which reads UTF-8. Runs only on CI's Windows leg.
//!
//! Allocating a console acts on the whole process, so every test here is
//! serialized and starts from a known state, as `tests/console_acquisition.rs`
//! does.

#![cfg(windows)]
#![allow(unsafe_code)]

use std::time::Duration;

use platform::term::{ConsoleAcquisition, Terminal};
use platform_windows::ffi::win32_surface as w;
use platform_windows::WindowsTerminal;

static CONSOLE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock_console_tests() -> std::sync::MutexGuard<'static, ()> {
    CONSOLE_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

const WAIT: Option<Duration> = Some(Duration::from_millis(100));
const VK_A: u16 = 0x41;
const VK_SHIFT: u16 = 0x10;
const VK_MENU: u16 = 0x12;

/// A terminal on a fresh console of this process's own.
fn fresh_console() -> WindowsTerminal {
    let mut term = WindowsTerminal::new();
    let _ = term.free_console();
    term.alloc_console().expect("AllocConsole");
    term
}

fn key(down: bool, vk: u16, ch: char) -> w::INPUT_RECORD {
    unit(down, vk, ch as u16, 0)
}

fn unit(down: bool, vk: u16, ch: u16, state: u32) -> w::INPUT_RECORD {
    w::INPUT_RECORD {
        EventType: w::KEY_EVENT as u16,
        Event: w::INPUT_RECORD_0 {
            KeyEvent: w::KEY_EVENT_RECORD {
                bKeyDown: i32::from(down),
                wRepeatCount: 1,
                wVirtualKeyCode: vk,
                wVirtualScanCode: 0,
                uChar: w::KEY_EVENT_RECORD_0 { UnicodeChar: ch },
                dwControlKeyState: state,
            },
        },
    }
}

/// Queues `records` on the console's input.
fn queue(records: &[w::INPUT_RECORD]) {
    // SAFETY: plain slot constant.
    let hin = unsafe { w::GetStdHandle(w::STD_INPUT_HANDLE) };
    let mut written = 0u32;
    // SAFETY: `hin` is this process's console input; `records` outlives
    // the call and `written` is a valid out-pointer.
    let ok =
        unsafe { w::WriteConsoleInputW(hin, records.as_ptr(), records.len() as u32, &mut written) };
    assert_ne!(ok, 0, "WriteConsoleInputW failed");
    assert_eq!(written as usize, records.len());
}

#[test]
fn key_ups_and_bare_modifiers_are_not_readable() {
    let _guard = lock_console_tests();
    let mut term = fresh_console();
    // Raw, so the read below returns the key without waiting for Enter.
    term.enter_raw().expect("raw mode");
    queue(&[key(false, VK_A, 'a'), key(true, VK_SHIFT, '\0')]);
    assert!(
        !term.poll_readable(WAIT).expect("poll"),
        "a key-up and a bare Shift press reported readable"
    );
    queue(&[key(true, VK_A, 'a')]);
    assert!(term.poll_readable(WAIT).expect("poll"), "a key-down 'a'");
    let mut buf = [0u8; 8];
    let n = term.read_chunk(&mut buf).expect("read");
    assert_eq!(&buf[..n], b"a");
    term.leave_raw().expect("leave raw mode");
    term.free_console().expect("FreeConsole");
}

/// Alt+numpad entry delivers its character on Alt's release.
#[test]
fn an_alt_release_with_a_character_is_readable() {
    let _guard = lock_console_tests();
    let mut term = fresh_console();
    queue(&[key(false, VK_MENU, 'é')]);
    assert!(term.poll_readable(WAIT).expect("poll"));
    term.free_console().expect("FreeConsole");
}

#[test]
fn piped_stdin_keeps_the_plain_wait() {
    let _guard = lock_console_tests();
    let mut term = fresh_console();
    let (mut read, mut write) = (std::ptr::null_mut(), std::ptr::null_mut());
    // SAFETY: valid out-pointers; default security and buffer size.
    let ok = unsafe { w::CreatePipe(&mut read, &mut write, std::ptr::null(), 0) };
    assert_ne!(ok, 0, "CreatePipe failed");
    let mut written = 0u32;
    // SAFETY: `write` is the pipe's live write end; one byte from a
    // valid buffer.
    let ok = unsafe { w::WriteFile(write, b"x".as_ptr(), 1, &mut written, std::ptr::null_mut()) };
    assert_ne!(ok, 0, "WriteFile failed");
    // SAFETY: `read` is a live handle; the console's input is restored
    // below by freeing the console and its std handles.
    let ok = unsafe { w::SetStdHandle(w::STD_INPUT_HANDLE, read) };
    assert_ne!(ok, 0, "SetStdHandle failed");
    assert!(term.poll_readable(WAIT).expect("poll"), "a piped byte");
    term.free_console().expect("FreeConsole");
    // SAFETY: both ends are live handles owned by this test.
    unsafe {
        w::CloseHandle(read);
        w::CloseHandle(write);
    }
}

/// The key-down records that type `text`, one per UTF-16 unit.
fn typed(text: &str) -> Vec<w::INPUT_RECORD> {
    text.encode_utf16().map(|u| unit(true, 0, u, 0)).collect()
}

/// Reads until `want` bytes arrived, polling before each read, with
/// `chunk`-byte reads.
fn read_polled(term: &WindowsTerminal, chunk: usize, want: usize) -> Vec<u8> {
    let mut got = Vec::new();
    let mut buf = vec![0u8; chunk];
    while got.len() < want {
        assert!(
            term.poll_readable(WAIT).expect("poll"),
            "not readable after {} of {want} bytes",
            got.len()
        );
        let n = term.read_chunk(&mut buf).expect("read");
        assert!(n > 0, "end of input after {} of {want} bytes", got.len());
        got.extend_from_slice(&buf[..n]);
    }
    got
}

/// Raw mode on a fresh console whose input code page is `cp`.
fn raw_console(cp: u32) -> WindowsTerminal {
    let mut term = fresh_console();
    // SAFETY: plain value; this process's console.
    assert_ne!(unsafe { w::SetConsoleCP(cp) }, 0, "SetConsoleCP({cp})");
    // SAFETY: no arguments.
    assert_eq!(unsafe { w::GetConsoleCP() }, cp);
    term.enter_raw().expect("raw mode");
    term
}

fn close(mut term: WindowsTerminal) {
    term.leave_raw().expect("leave raw mode");
    term.free_console().expect("FreeConsole");
}

/// The key-down that types NUL is input; its key-up is not.
#[test]
fn the_key_that_types_nul_is_readable() {
    let _guard = lock_console_tests();
    let term = raw_console(437);
    // SAFETY: no pointers.
    let scan = unsafe { w::VkKeyScanW(0) };
    let scan = u16::try_from(scan).expect("the layout types NUL with some key");
    let mut state = 0;
    if scan & 0x100 != 0 {
        state |= w::SHIFT_PRESSED;
    }
    if scan & 0x200 != 0 {
        state |= w::LEFT_CTRL_PRESSED;
    }
    if scan & 0x400 != 0 {
        state |= w::LEFT_ALT_PRESSED;
    }
    let vk = scan & 0xff;
    queue(&[unit(false, vk, 0, state)]);
    assert!(!term.poll_readable(WAIT).expect("poll"), "NUL's key-up");
    queue(&[unit(true, vk, 0, state)]);
    assert_eq!(read_polled(&term, 8, 1), [0]);
    close(term);
}

/// Text arrives as UTF-8 under an OEM code page, including characters
/// that page cannot represent.
#[test]
fn text_is_utf8_under_a_non_utf8_code_page() {
    let _guard = lock_console_tests();
    let term = raw_console(437);
    let text = "é€😀x";
    queue(&typed(text));
    assert_eq!(read_polled(&term, 64, text.len()), text.as_bytes());
    close(term);
}

/// A character whose UTF-8 form straddles a read's end arrives whole
/// over the next reads, with a poll between each.
#[test]
fn a_character_split_across_reads_arrives_whole() {
    let _guard = lock_console_tests();
    let term = raw_console(437);
    let text = format!("{}é😀x", "a".repeat(4095));
    queue(&typed(&text));
    assert_eq!(read_polled(&term, 4096, text.len()), text.as_bytes());
    let small = "é😀x";
    queue(&typed(small));
    assert_eq!(read_polled(&term, 1, small.len()), small.as_bytes());
    queue(&typed(small));
    assert_eq!(read_polled(&term, 3, small.len()), small.as_bytes());
    close(term);
}

/// Text a small read left behind belongs to its console: freeing it and
/// allocating another leaves nothing readable.
#[test]
fn pending_text_does_not_outlive_its_console() {
    let _guard = lock_console_tests();
    let term = raw_console(437);
    queue(&typed("é"));
    assert_eq!(read_polled(&term, 1, 1), [0xC3]);
    close(term);
    let term = raw_console(437);
    assert!(
        !term.poll_readable(WAIT).expect("poll"),
        "the first console's pending byte reached the second"
    );
    queue(&typed("x"));
    assert_eq!(read_polled(&term, 8, 1), b"x");
    close(term);
}
