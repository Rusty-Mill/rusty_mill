//! `Terminal::poll_readable` on a console reports readable only once a
//! read returns a byte: records that carry no character (key-ups, bare
//! modifiers) are dropped first. rusty_mux's attach client relies on it:
//! it re-checks its own state between waits, and a wake for a key-up
//! would leave it blocked in a read until the next key. Runs only on CI's
//! Windows leg.
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
    w::INPUT_RECORD {
        EventType: w::KEY_EVENT as u16,
        Event: w::INPUT_RECORD_0 {
            KeyEvent: w::KEY_EVENT_RECORD {
                bKeyDown: i32::from(down),
                wRepeatCount: 1,
                wVirtualKeyCode: vk,
                wVirtualScanCode: 0,
                uChar: w::KEY_EVENT_RECORD_0 {
                    UnicodeChar: ch as u16,
                },
                dwControlKeyState: 0,
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
