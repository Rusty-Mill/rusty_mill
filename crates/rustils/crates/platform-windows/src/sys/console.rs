//! Console terminal primitives (extraction map D9, via the rusty_win32
//! donor): tty probe, viewport size, raw mode over console modes,
//! (slice 2, roadmap Phase 2) live raw-mode probing, poll/read on
//! stdin, and echo toggling, plus (the `rusty_naner`-facet slice)
//! console *acquisition* — `alloc`/`attach`/`free` and the std-handle
//! reopen that makes an acquired console immediately usable through
//! every function above.
//!
//! The isatty analog IS `GetConsoleMode` succeeding — a redirected
//! (pipe/file) std handle fails the call. Raw mode clears the cooked
//! input bits (echo, line buffering, Ctrl-C processing) and sets the
//! virtual-terminal bits so a Win10+ console speaks the same byte
//! dialect as a Unix tty in raw mode.
//!
//! Track W (D-15, slice 3): the donor stopped being only a donor here.
//! Every foreign call in this module now has a `rusty_win32` arm behind
//! `track-w` except [`reopen`], which documents its own reason for
//! staying. The mode get/set pair is factored into two-armed [`get_mode`]/
//! [`set_mode`] helpers so the ten public functions built on it keep a
//! single body each.

#![allow(unsafe_code)]

use std::ffi::OsStr;
use std::time::Duration;

use platform::error::Result;
use platform::term::{ConsoleState, TermStream};

use crate::ffi::win32_surface as w;
use crate::sys::errmap;
use crate::util::wide::to_wide_nul;

/// The std-handle slot for a stream — handle values stop at this
/// boundary (RFC v2 §5).
fn slot(stream: TermStream) -> u32 {
    match stream {
        TermStream::Stdin => w::STD_INPUT_HANDLE,
        TermStream::Stdout => w::STD_OUTPUT_HANDLE,
        TermStream::Stderr => w::STD_ERROR_HANDLE,
    }
}

/// Track W (D-15): `rusty_win32::handle::get_std_handle`. The donor splits
/// `GetStdHandle`'s two zero-ish outcomes into `Ok(None)` (slot empty) and
/// `Err` (call failed, i.e. `INVALID_HANDLE_VALUE`) rather than returning
/// the raw sentinel. Both are folded straight back to the exact value the
/// raw call produced, so every caller below — all of which check
/// `is_null() || == INVALID_HANDLE_VALUE`, or simply let the next console
/// call fail — sees an identical handle in every case.
#[cfg(feature = "track-w")]
fn std_handle(stream: TermStream) -> w::HANDLE {
    match rusty_win32::handle::get_std_handle(slot(stream)) {
        Ok(Some(h)) => h,
        Ok(None) => std::ptr::null_mut(),
        Err(_) => w::INVALID_HANDLE_VALUE,
    }
}

#[cfg(not(feature = "track-w"))]
fn std_handle(stream: TermStream) -> w::HANDLE {
    // SAFETY: GetStdHandle takes a documented slot constant and has no
    // pointer arguments; the returned handle is process-owned and must
    // not be closed here.
    unsafe { w::GetStdHandle(slot(stream)) }
}

// The `GetConsoleMode`/`SetConsoleMode` pair is this module's workhorse:
// the tty probe, raw-mode enter/restore, the live raw probe, and the echo
// toggle are all built from it. Two-arming the pair once keeps every
// public function above single-bodied, rather than doubling ten of them —
// the same shape `sys::proc`'s `assign_to_job`/`resume_thread` helpers
// took in the previous slice.

/// `GetConsoleMode` — Track W (D-15): `rusty_win32::console::get_mode`.
/// `Ok` is the mode word; `Err` means the handle is not a console (the
/// isatty analog) or the call otherwise failed.
#[cfg(feature = "track-w")]
fn get_mode(h: w::HANDLE) -> Result<w::CONSOLE_MODE> {
    // SAFETY: `h` is a live process-owned std handle or a handle this
    // module just opened; `get_mode` reports a non-console or invalid one
    // as an ordinary error, not UB.
    unsafe { rusty_win32::console::get_mode(h) }
        .map_err(|e| errmap::trackw_err("GetConsoleMode", e))
}

#[cfg(not(feature = "track-w"))]
fn get_mode(h: w::HANDLE) -> Result<w::CONSOLE_MODE> {
    let mut mode: w::CONSOLE_MODE = 0;
    // SAFETY: `h` is a live process-owned std handle or a handle this
    // module just opened; `mode` is a valid out-pointer for the duration
    // of the call.
    if unsafe { w::GetConsoleMode(h, &mut mode) } == 0 {
        return Err(errmap::last_win32_err("GetConsoleMode", OsStr::new("")));
    }
    Ok(mode)
}

/// `SetConsoleMode` — Track W (D-15): `rusty_win32::console::set_mode`.
#[cfg(feature = "track-w")]
fn set_mode(h: w::HANDLE, mode: w::CONSOLE_MODE) -> Result<()> {
    // SAFETY: `h` is a live console handle (every caller has just queried
    // it successfully with `get_mode`); `mode` is a plain bitmask.
    unsafe { rusty_win32::console::set_mode(h, mode) }
        .map_err(|e| errmap::trackw_err("SetConsoleMode", e))
}

#[cfg(not(feature = "track-w"))]
fn set_mode(h: w::HANDLE, mode: w::CONSOLE_MODE) -> Result<()> {
    // SAFETY: `h` is a live console handle (every caller has just queried
    // it successfully with `get_mode`); no pointer arguments.
    if unsafe { w::SetConsoleMode(h, mode) } == 0 {
        return Err(errmap::last_win32_err("SetConsoleMode", OsStr::new("")));
    }
    Ok(())
}

/// Whether `stream`'s handle is a console (`GetConsoleMode` succeeds).
pub fn is_tty(stream: TermStream) -> bool {
    let h = std_handle(stream);
    if h.is_null() || h == w::INVALID_HANDLE_VALUE {
        return false;
    }
    get_mode(h).is_ok()
}

/// Viewport size (srWindow, not the scrollback buffer) of the first
/// console-attached std stream.
pub fn window_size() -> Result<(u16, u16)> {
    for stream in [TermStream::Stdout, TermStream::Stderr, TermStream::Stdin] {
        if !is_tty(stream) {
            continue;
        }
        return screen_size(std_handle(stream));
    }
    Err(errmap::last_win32_err("GetConsoleMode", OsStr::new("")))
}

/// `GetConsoleScreenBufferInfo`'s viewport as `(rows, cols)`.
///
/// Track W (D-15): `rusty_win32::console::window_size`, which computes the
/// identical `srWindow` extents — **but returns them as `(cols, rows)`**,
/// the opposite order from this crate's `(rows, cols)` (which follows
/// `winsize`'s `ws_row`/`ws_col` ordering, since `platform::term` is a
/// portable surface with a Unix-shaped contract). Swapped at the boundary
/// here. Both orderings are defensible in isolation, which is exactly what
/// makes a same-typed `(u16, u16)` pair a live trap: nothing in the type
/// system catches getting it backwards, and a transposed terminal size
/// fails silently and cosmetically rather than loudly.
#[cfg(feature = "track-w")]
fn screen_size(h: w::HANDLE) -> Result<(u16, u16)> {
    // SAFETY: `h` is a live console output handle (`is_tty`-probed by the
    // caller immediately above).
    let (cols, rows) = unsafe { rusty_win32::console::window_size(h) }
        .map_err(|e| errmap::trackw_err("GetConsoleScreenBufferInfo", e))?;
    Ok((rows, cols))
}

#[cfg(not(feature = "track-w"))]
fn screen_size(h: w::HANDLE) -> Result<(u16, u16)> {
    // SAFETY: CONSOLE_SCREEN_BUFFER_INFO is plain-old-data; zeroed
    // is valid scratch the call overwrites on success.
    let mut info: w::CONSOLE_SCREEN_BUFFER_INFO = unsafe { std::mem::zeroed() };
    // SAFETY: `h` is a live console handle (probed by the caller); `info`
    // is a valid out-pointer outliving the call.
    if unsafe { w::GetConsoleScreenBufferInfo(h, &mut info) } == 0 {
        return Err(errmap::last_win32_err(
            "GetConsoleScreenBufferInfo",
            OsStr::new(""),
        ));
    }
    let cols = (info.srWindow.Right - info.srWindow.Left + 1).max(0) as u16;
    let rows = (info.srWindow.Bottom - info.srWindow.Top + 1).max(0) as u16;
    Ok((rows, cols))
}

/// The saved console modes `enter_raw` returns and `restore` takes back.
pub struct SavedModes {
    input: w::CONSOLE_MODE,
    output: Option<w::CONSOLE_MODE>,
}

/// Switch stdin (and stdout when attached) to raw mode, returning the
/// previous modes.
pub fn enter_raw() -> Result<SavedModes> {
    let hin = std_handle(TermStream::Stdin);
    let in_mode = get_mode(hin)?;
    let raw_in = (in_mode
        & !(w::ENABLE_ECHO_INPUT | w::ENABLE_LINE_INPUT | w::ENABLE_PROCESSED_INPUT))
        | w::ENABLE_VIRTUAL_TERMINAL_INPUT;
    set_mode(hin, raw_in)?;

    // Output VT processing is best-effort: stdout may be redirected
    // while stdin is still a console.
    let mut output = None;
    if is_tty(TermStream::Stdout) {
        let hout = std_handle(TermStream::Stdout);
        if let Ok(out_mode) = get_mode(hout) {
            let vt = out_mode | w::ENABLE_PROCESSED_OUTPUT | w::ENABLE_VIRTUAL_TERMINAL_PROCESSING;
            if set_mode(hout, vt).is_ok() {
                output = Some(out_mode);
            }
        }
    }
    Ok(SavedModes {
        input: in_mode,
        output,
    })
}

/// Restore the modes saved by [`enter_raw`].
pub fn restore(saved: &SavedModes) -> Result<()> {
    let hin = std_handle(TermStream::Stdin);
    set_mode(hin, saved.input)?;
    if let Some(out_mode) = saved.output {
        // Best-effort, matching `enter_raw`'s own stdout posture: stdin's
        // restore is what actually matters for leaving the terminal usable.
        let _ = set_mode(std_handle(TermStream::Stdout), out_mode);
    }
    Ok(())
}

/// Live probe: does stdin's *current* mode look raw (no `ENABLE_ECHO_INPUT`,
/// no `ENABLE_LINE_INPUT`)? A handle that cannot be queried is not
/// usefully "raw" — same best-effort contract as the Linux arm.
pub fn is_raw() -> bool {
    let hin = std_handle(TermStream::Stdin);
    if hin.is_null() || hin == w::INVALID_HANDLE_VALUE {
        return false;
    }
    match get_mode(hin) {
        Ok(mode) => mode & (w::ENABLE_ECHO_INPUT | w::ENABLE_LINE_INPUT) == 0,
        Err(_) => false,
    }
}

/// Toggle `ENABLE_ECHO_INPUT` on stdin without touching any other bit,
/// returning the previous on/off state.
pub fn set_echo(on: bool) -> Result<bool> {
    let hin = std_handle(TermStream::Stdin);
    let mode = get_mode(hin)?;
    let was_on = mode & w::ENABLE_ECHO_INPUT != 0;
    let next = if on {
        mode | w::ENABLE_ECHO_INPUT
    } else {
        mode & !w::ENABLE_ECHO_INPUT
    };
    set_mode(hin, next)?;
    Ok(was_on)
}

/// `WaitForSingleObject(stdin, timeout_ms)`; `None` timeout blocks
/// forever. A console input handle is "signaled" when an unread input
/// record is queued — coarser than "a byte is ready" (any input event,
/// not just keystrokes, wakes it), but `ReadFile` afterward blocks
/// correctly on whatever was actually queued, so a spurious wake costs
/// one extra round trip, never a wrong read.
pub fn poll_readable(timeout: Option<Duration>) -> Result<bool> {
    let hin = std_handle(TermStream::Stdin);
    let timeout_ms: u32 = match timeout {
        None => w::INFINITE,
        Some(d) => u32::try_from(d.as_millis()).unwrap_or(u32::MAX),
    };
    wait_signaled(hin, timeout_ms)
}

/// `WaitForSingleObject` reduced to signaled/timed-out — Track W (D-15):
/// `rusty_win32::console::wait_readable`, whose three-way match on
/// `WAIT_OBJECT_0`/`WAIT_TIMEOUT`/anything-else is the same one the
/// windows-sys arm writes out below.
#[cfg(feature = "track-w")]
fn wait_signaled(h: w::HANDLE, timeout_ms: u32) -> Result<bool> {
    // SAFETY: `h` is a live, waitable std handle.
    unsafe { rusty_win32::console::wait_readable(h, timeout_ms) }
        .map_err(|e| errmap::trackw_err("WaitForSingleObject", e))
}

#[cfg(not(feature = "track-w"))]
fn wait_signaled(h: w::HANDLE, timeout_ms: u32) -> Result<bool> {
    // SAFETY: `h` is a live, waitable std handle.
    let r = unsafe { w::WaitForSingleObject(h, timeout_ms) };
    if r == w::WAIT_OBJECT_0 {
        Ok(true)
    } else if r == w::WAIT_TIMEOUT {
        Ok(false)
    } else {
        Err(errmap::last_win32_err(
            "WaitForSingleObject",
            OsStr::new(""),
        ))
    }
}

/// `ReadFile(stdin, buf)` — one call, batched, `Ok(0)` = EOF.
///
/// Track W (D-15): `rusty_win32::console::read`. Same clamp as
/// `sys::fileio::read`'s track-w arm and for the same reason — the donor
/// passes `buf.len() as u32` straight through, which wraps rather than
/// saturating on an oversized slice.
#[cfg(feature = "track-w")]
pub fn read_chunk(buf: &mut [u8]) -> Result<usize> {
    let hin = std_handle(TermStream::Stdin);
    let len = buf.len().min(u32::MAX as usize);
    // SAFETY: `hin` is a live process-owned std handle open for reading.
    unsafe { rusty_win32::console::read(hin, &mut buf[..len]) }
        .map_err(|e| errmap::trackw_err("ReadFile", e))
}

#[cfg(not(feature = "track-w"))]
pub fn read_chunk(buf: &mut [u8]) -> Result<usize> {
    let hin = std_handle(TermStream::Stdin);
    let mut n: u32 = 0;
    let len = u32::try_from(buf.len()).unwrap_or(u32::MAX);
    // SAFETY: `buf` is a valid writable region of at least `len` bytes
    // and `n` a valid out-pointer, both outliving the call; the handle
    // is synchronous (no OVERLAPPED), so the null overlapped is valid.
    let ok = unsafe { w::ReadFile(hin, buf.as_mut_ptr(), len, &mut n, std::ptr::null_mut()) };
    if ok == 0 {
        return Err(errmap::last_win32_err("ReadFile", OsStr::new("")));
    }
    Ok(n as usize)
}

// Console *acquisition* (`platform::term::ConsoleAcquisition`, D9's
// rusty_naner facet) — alloc/attach/free plus the std-handle reopen a
// caller needs afterward to actually use the acquired console through
// this same module's other functions. Distinct from everything above:
// those all operate on whatever console (if any) this process already
// had at start; these change *which* console it has.

/// Whether this process currently has any console attached at all —
/// probed by attempting to open `CONIN$` and immediately closing it on
/// success. **Deliberately not `GetConsoleWindow`**: a first version of
/// this probe used it, and `windows-latest` CI caught the real bug that
/// choice hides — a ConPTY-hosted console (this crate's own
/// `platform::pty` backend, and, it turns out, however the Actions
/// runner hosts `pwsh` for a `cargo test` step) has no `HWND` at all,
/// so `GetConsoleWindow` returning null does not mean "no console" the
/// way it does for a classic conhost-windowed console. That false
/// negative made [`initial_state`] report `None` for a process that
/// genuinely had one attached, which made a test's `free_console()`
/// wrongly no-op, which made the next `alloc_console()` fail with a
/// real, correctly-reported `ERROR_ACCESS_DENIED` — the OS was right;
/// the probe was wrong. Opening `CONIN$` answers "is a console attached
/// right now" directly, independent of whether that console has a
/// window (unlike [`is_tty`], which is std-handle-based and would miss
/// an attached-but-redirected-away-from console the same wrong
/// direction).
fn has_console() -> bool {
    match reopen("CONIN$", w::GENERIC_READ) {
        Ok(h) => {
            close_probe_handle(h);
            true
        }
        Err(_) => false,
    }
}

/// Close the throwaway handle [`has_console`] opened purely to learn that
/// the open succeeded. Failure is unreportable and uninteresting here —
/// the probe's answer is already decided by the time this runs.
///
/// Track W (D-15): `rusty_win32::handle::close`, whose `Result` is
/// discarded exactly as the raw `BOOL` is.
#[cfg(feature = "track-w")]
fn close_probe_handle(h: w::HANDLE) {
    // SAFETY: `h` is the live handle `reopen` just returned, owned by the
    // caller's frame and never used again after this call — `close`'s own
    // "don't use it again" obligation.
    let _ = unsafe { rusty_win32::handle::close(h) };
}

#[cfg(not(feature = "track-w"))]
fn close_probe_handle(h: w::HANDLE) {
    // SAFETY: `h` is the live handle `reopen` just returned, owned by the
    // caller's frame and never used again after this call.
    unsafe {
        w::CloseHandle(h);
    }
}

/// [`WindowsTerminal`](crate::WindowsTerminal)'s starting
/// [`ConsoleState`] — `Inherited` if a console was already attached at
/// process creation (the ordinary console-subsystem case), `None`
/// otherwise (a GUI-subsystem process with no console yet). Never
/// reports `Allocated`/`Attached`: those states only exist after this
/// same handle's own [`alloc`]/[`attach`] call, which is exactly what
/// `WindowsTerminal` uses this function for — a one-time probe at
/// construction, not a live query [`ConsoleAcquisition::console_state`](
/// platform::term::ConsoleAcquisition::console_state) re-runs on every
/// call.
pub fn initial_state() -> ConsoleState {
    if has_console() {
        ConsoleState::Inherited
    } else {
        ConsoleState::None
    }
}

/// Allocate a brand-new console for this process — `AllocConsole`.
/// Fails with `ErrorKind::PermissionDenied` (Windows' own
/// `ERROR_ACCESS_DENIED`) if this process already has one.
/// Track W (D-15): `rusty_win32::console::alloc`.
#[cfg(feature = "track-w")]
pub fn alloc() -> Result<()> {
    rusty_win32::console::alloc().map_err(|e| errmap::trackw_err("AllocConsole", e))
}

#[cfg(not(feature = "track-w"))]
pub fn alloc() -> Result<()> {
    // SAFETY: `AllocConsole` takes no arguments and has no precondition.
    if unsafe { w::AllocConsole() } == 0 {
        return Err(errmap::last_win32_err("AllocConsole", OsStr::new("")));
    }
    Ok(())
}

/// Detach this process from its current console — `FreeConsole`.
/// Track W (D-15): `rusty_win32::console::free`.
#[cfg(feature = "track-w")]
pub fn free() -> Result<()> {
    rusty_win32::console::free().map_err(|e| errmap::trackw_err("FreeConsole", e))
}

#[cfg(not(feature = "track-w"))]
pub fn free() -> Result<()> {
    // SAFETY: `FreeConsole` takes no arguments and has no precondition.
    if unsafe { w::FreeConsole() } == 0 {
        return Err(errmap::last_win32_err("FreeConsole", OsStr::new("")));
    }
    Ok(())
}

/// Attach to another process's console — `AttachConsole`. `pid = None`
/// maps to `ATTACH_PARENT_PROCESS`, attaching to whatever console (if
/// any) launched this process.
///
/// Track W (D-15): `rusty_win32::console::attach`, which takes the same
/// `Option<u32>` and applies the same `ATTACH_PARENT_PROCESS` default for
/// `None` — one of the few places the donor's signature already matches
/// this crate's exactly, sentinel handling included.
#[cfg(feature = "track-w")]
pub fn attach(pid: Option<u32>) -> Result<()> {
    rusty_win32::console::attach(pid).map_err(|e| errmap::trackw_err("AttachConsole", e))
}

#[cfg(not(feature = "track-w"))]
pub fn attach(pid: Option<u32>) -> Result<()> {
    let process_id = pid.unwrap_or(w::ATTACH_PARENT_PROCESS);
    // SAFETY: `AttachConsole` takes a plain `DWORD` process id (or the
    // documented `ATTACH_PARENT_PROCESS` sentinel) and has no other
    // precondition.
    if unsafe { w::AttachConsole(process_id) } == 0 {
        return Err(errmap::last_win32_err("AttachConsole", OsStr::new("")));
    }
    Ok(())
}

/// Open a real handle to whatever console this process is currently
/// attached to — `CreateFileW("CONIN$"/"CONOUT$", ...)`, the documented
/// way to get a console handle that is immune to std-handle redirection
/// (unlike `GetStdHandle`, which keeps returning whatever was inherited
/// or last set, console-attached or not).
///
/// **Stays on windows-sys in both Track W configurations (D-15)** — the
/// one call in this module that didn't migrate, and a judgment call rather
/// than a hard blocker like `sys::proc`'s `CreateProcessW`.
/// `rusty_win32::fs::open_file` matches on everything that obviously
/// matters (`OPEN_EXISTING`, `FILE_SHARE_READ | FILE_SHARE_WRITE`, null
/// security attributes) but passes `FILE_ATTRIBUTE_NORMAL` where this
/// passes `0`. On an `OPEN_EXISTING` open of a console pseudo-device that
/// difference is very probably inert — and "very probably" is the problem:
/// [`has_console`] is a *probe* whose previous incarnation
/// (`GetConsoleWindow`) shipped a false negative that only surfaced as a
/// confusing downstream `ERROR_ACCESS_DENIED` on a CI runner, as that
/// function's own comment records at length. Trading a verified-correct
/// call for an unverified-equivalent one, to gain provenance on a single
/// site, is not a good trade here. Revisit if the donor grows an explicit
/// console-device open.
fn reopen(name: &str, access: u32) -> Result<w::HANDLE> {
    let wide = to_wide_nul(OsStr::new(name));
    // SAFETY: `wide` is a valid, NUL-terminated UTF-16 string naming a
    // well-known console pseudo-device; no security-attributes pointer
    // is needed (this handle is not meant to be inherited by a future
    // child — nothing here spawns one).
    let h = unsafe {
        w::CreateFileW(
            wide.as_ptr(),
            access,
            w::FILE_SHARE_READ | w::FILE_SHARE_WRITE,
            std::ptr::null(),
            w::OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    if h.is_null() || h == w::INVALID_HANDLE_VALUE {
        return Err(errmap::last_win32_err("CreateFileW", OsStr::new(name)));
    }
    Ok(h)
}

/// Repoint this process's std handles onto whatever console it is
/// currently attached to — `CONIN$`/`CONOUT$` reopened via
/// [`reopen`] and installed with `SetStdHandle`, since neither
/// `AllocConsole` nor `AttachConsole` update a std slot that was ever
/// explicitly redirected (a documented Windows quirk: a `cargo test`
/// process under a CI runner with stdin closed keeps `GetStdHandle`
/// returning the old, redirected handle even after acquiring a real
/// console, unless something reopens it explicitly — the reason this
/// function exists at all rather than trusting acquisition alone).
/// Stdout and stderr are two independent handles onto the same
/// `CONOUT$` screen buffer (Windows has no distinct "CONERR$"); VT
/// output processing is re-enabled on the freshly opened stdout handle,
/// best-effort, so [`enter_raw`]'s own VT expectations already hold for
/// a caller that never touches raw mode explicitly. Called by
/// [`crate::term::WindowsTerminal`] right after a successful
/// [`alloc`]/[`attach`] — never on its own from outside this crate.
pub fn reopen_std_handles() -> Result<()> {
    let hin = reopen("CONIN$", w::GENERIC_READ | w::GENERIC_WRITE)?;
    let hout = reopen("CONOUT$", w::GENERIC_READ | w::GENERIC_WRITE)?;
    let herr = reopen("CONOUT$", w::GENERIC_READ | w::GENERIC_WRITE)?;

    set_std(w::STD_INPUT_HANDLE, hin, "stdin")?;
    set_std(w::STD_OUTPUT_HANDLE, hout, "stdout")?;
    set_std(w::STD_ERROR_HANDLE, herr, "stderr")?;

    // Best-effort VT enable, matching `enter_raw`'s own stdout posture
    // (see its doc comment): a failure here doesn't fail acquisition
    // itself, it just means a caller gets the same "VT is best-effort"
    // contract `enter_raw` already documents.
    if let Ok(mode) = get_mode(hout) {
        let vt = mode | w::ENABLE_PROCESSED_OUTPUT | w::ENABLE_VIRTUAL_TERMINAL_PROCESSING;
        let _ = set_mode(hout, vt);
    }
    Ok(())
}

/// Install `h` into one of this process's std slots.
///
/// Track W (D-15): `rusty_win32::handle::set_std_handle`. `SetStdHandle`
/// stores the value without duplicating or otherwise invalidating it —
/// which is exactly why the handles [`reopen_std_handles`] opens are
/// deliberately never closed: the slot now refers to them for the life of
/// the process.
#[cfg(feature = "track-w")]
fn set_std(slot: u32, h: w::HANDLE, label: &str) -> Result<()> {
    // SAFETY: `h` is a live, valid handle just opened by `reopen`, and it
    // outlives its use as a standard handle (never closed) — precisely the
    // obligation `set_std_handle` documents.
    unsafe { rusty_win32::handle::set_std_handle(slot, h) }
        .map_err(|e| errmap::trackw_err("SetStdHandle", e).with_path(OsStr::new(label)))
}

#[cfg(not(feature = "track-w"))]
fn set_std(slot: u32, h: w::HANDLE, label: &str) -> Result<()> {
    // SAFETY: `h` is a live, valid handle just opened by `reopen`;
    // `SetStdHandle` stores the value into this process's std slot
    // without duplicating or otherwise invalidating it.
    if unsafe { w::SetStdHandle(slot, h) } == 0 {
        return Err(errmap::last_win32_err("SetStdHandle", OsStr::new(label)));
    }
    Ok(())
}
