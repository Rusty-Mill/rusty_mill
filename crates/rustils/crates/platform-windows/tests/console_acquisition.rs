//! Live console-acquisition test (`platform::term::ConsoleAcquisition`,
//! D9's `rusty_naner` facet): exercises real `AllocConsole`/
//! `AttachConsole`/`FreeConsole` calls against this test process's own
//! console, no mock — the same live-verification bar `tests/pty.rs`
//! holds ConPTY to. Only actually executes on CI's `windows-latest`
//! leg; this crate's whole backend is developed from a Linux host
//! against `cargo check --target x86_64-pc-windows-gnu`
//! (`crates/platform-windows/src/lib.rs`'s own module doc), so nothing
//! here has run outside CI — flagged here rather than hidden, the same
//! discipline `docs/design-discussion-console.md` records for this
//! whole slice.
//!
//! `AllocConsole`/`AttachConsole`/`FreeConsole` act on the **whole
//! process**, not on any one `WindowsTerminal` instance — closer to
//! `tests/pty.rs`'s own `PTY_TEST_LOCK` rationale than to an ordinary
//! unit test's isolation. Every test below is serialized through
//! [`lock_console_tests`] and leaves the process's console state
//! explicitly known (via its own `free_console()` cleanup) rather than
//! assuming what the previous test, or the CI runner's own launch
//! shape, left behind — rusty_win32's own test suite hit exactly this
//! uncertainty ("GitHub Actions' `windows-latest` runner's exact
//! console-attachment state for a `cargo test` process isn't something
//! [a sandboxed session] can verify ahead of time") and resolved it the
//! same way: never assume, always establish the starting state first.

#![cfg(windows)]

use platform::term::{ConsoleAcquisition, ConsoleState, TermStream, Terminal};
use platform_windows::WindowsTerminal;

/// Serializes every test in this file — see the module doc.
static CONSOLE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock_console_tests() -> std::sync::MutexGuard<'static, ()> {
    CONSOLE_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[test]
fn alloc_then_free_round_trips() {
    let _guard = lock_console_tests();
    let mut term = WindowsTerminal::new();
    // Establish a known starting state: no console attached, regardless
    // of whatever this test process inherited or a previous test left
    // behind.
    let _ = term.free_console();
    assert_eq!(term.console_state(), ConsoleState::None);

    term.alloc_console()
        .expect("AllocConsole should succeed once no console is attached");
    assert_eq!(term.console_state(), ConsoleState::Allocated);
    assert!(
        term.is_tty(TermStream::Stdout),
        "reopen_std_handles should have repointed stdout at the new console"
    );
    assert!(
        term.is_tty(TermStream::Stdin),
        "reopen_std_handles should have repointed stdin at the new console"
    );

    term.free_console().expect("FreeConsole should succeed");
    assert_eq!(term.console_state(), ConsoleState::None);
}

#[test]
fn alloc_while_already_attached_fails_with_permission_denied() {
    let _guard = lock_console_tests();
    let mut term = WindowsTerminal::new();
    let _ = term.free_console();

    term.alloc_console()
        .expect("the first alloc should establish a console");

    // Windows documents `AllocConsole` as failing with
    // `ERROR_ACCESS_DENIED` when the calling process already has one —
    // surfaced as-is (`ErrorKind::PermissionDenied`), the same
    // "report the real call's own documented failure" discipline
    // `errmap` already applies everywhere else in this crate.
    let err = term
        .alloc_console()
        .expect_err("a second alloc while one is already held must fail");
    assert_eq!(err.kind, platform::error::ErrorKind::PermissionDenied);
    // The failed second call must not have disturbed the state the
    // first call established.
    assert_eq!(term.console_state(), ConsoleState::Allocated);

    term.free_console().expect("FreeConsole should succeed");
}

#[test]
fn free_console_without_one_attached_is_a_no_op() {
    let _guard = lock_console_tests();
    let mut term = WindowsTerminal::new();
    let _ = term.free_console();
    assert_eq!(term.console_state(), ConsoleState::None);

    // Idempotent, matching `Terminal::leave_raw`'s own "without a prior
    // enter it's a no-op" contract rather than surfacing whatever
    // `FreeConsole` itself would report for "nothing to free".
    term.free_console()
        .expect("free_console with no console attached must be Ok, not an error");
    assert_eq!(term.console_state(), ConsoleState::None);
}

#[test]
fn new_reports_a_real_starting_state_not_a_default_guess() {
    let _guard = lock_console_tests();
    // Establish a known state (attached, via alloc) before probing a
    // *fresh* `WindowsTerminal` — `new()`'s `console_state()` must
    // reflect what the process actually has, not always start at
    // `None` regardless of reality.
    let mut setup = WindowsTerminal::new();
    let _ = setup.free_console();
    setup
        .alloc_console()
        .expect("AllocConsole should succeed once no console is attached");

    let fresh = WindowsTerminal::new();
    assert_eq!(
        fresh.console_state(),
        ConsoleState::Inherited,
        "a second handle constructed while a console is attached must see it as Inherited"
    );

    setup.free_console().expect("FreeConsole should succeed");
}
