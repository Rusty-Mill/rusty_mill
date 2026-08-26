//! Terminal syscall facade.
//!
//! The line editor needs a small, fixed slice of terminal API: `isatty`,
//! get/set the raw-mode-relevant attributes, a raw-mode flag flip, poll for
//! input readiness, a raw byte read, and the window size. Three backends
//! provide it behind one interface:
//!
//! - **`rusty_libc`** — the hand-rolled raw-syscall crate; the default on
//!   Linux (zero third-party deps).
//! - **`libc` crate** — the backend on other Unix, and on Linux when forced
//!   with `--no-default-features --features libc-backend`.
//! - **`rusty_win32`** — `GetConsoleMode`/`SetConsoleMode`/`ReadFile`/
//!   `WaitForSingleObject`/`GetConsoleScreenBufferInfo` on Windows, the
//!   direct analog of `tcgetattr`/`tcsetattr`/`poll`/`read`/`TIOCGWINSZ` —
//!   not ConPTY (`CreatePseudoConsole`), which hosts a *child* process's
//!   console session rather than this process's own inherited one. This
//!   module decides what "raw mode" means in terms of `rusty_win32`'s mode
//!   bits (which to clear/set); `rusty_win32` itself is policy-free, the
//!   same way the `libc`/`rusty_libc` backends decide the termios raw-mode
//!   recipe rather than baking it into those crates.
//!
//! All functions target the streams the editor uses directly: stdin for
//! input and raw mode, stdout for the window size (and, on Windows, output
//! VT processing — Windows tracks input/output console modes separately,
//! unlike a single Unix `termios` covering both, so the Windows backend's
//! `Termios` bundles both into one value to keep this interface uniform
//! across backends).
//!
//! ## errno note
//!
//! `rusty_libc` does not write glibc's TLS `errno`, so `Error::last_os_error`
//! is meaningless after its calls. This facade therefore builds every
//! `io::Error` from the syscall's own return/`Errno`, so callers keep getting
//! a correct `ErrorKind` (notably `Interrupted` for `EINTR`) in every
//! backend.

pub use imp::*;

// Backend selection is target-driven: `rusty_libc` on Linux (the default),
// `libc` on other Unix. `libc-backend` forces libc on Linux too.
#[cfg(all(
    target_os = "linux",
    not(feature = "rusty-libc"),
    not(feature = "libc-backend")
))]
compile_error!("no backend: enable the default `rusty-libc`, or `libc-backend`");

// ---- libc backend: other Unix, or Linux with `libc-backend` --------------
#[cfg(any(
    all(unix, not(target_os = "linux")),
    all(target_os = "linux", feature = "libc-backend")
))]
mod imp {
    use std::io;

    /// Opaque terminal-attributes value, sized and laid out by the backend.
    pub type Termios = libc::termios;

    /// Is stdin a terminal?
    pub fn isatty_stdin() -> bool {
        // SAFETY: isatty takes an fd and touches no memory.
        unsafe { libc::isatty(0) != 0 }
    }

    /// Is stdout a terminal? The editor paints on stdout, so raw-mode
    /// editing needs it to be one just as much as stdin.
    pub fn isatty_stdout() -> bool {
        // SAFETY: isatty takes an fd and touches no memory.
        unsafe { libc::isatty(1) != 0 }
    }

    /// Read stdin's terminal attributes.
    pub fn tcgetattr_stdin() -> io::Result<Termios> {
        // SAFETY: `t` is a valid, zeroed termios the kernel fills.
        unsafe {
            let mut t: Termios = std::mem::zeroed();
            if libc::tcgetattr(0, &mut t) != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(t)
        }
    }

    /// Apply attributes to stdin, draining pending output first (`TCSADRAIN`).
    pub fn tcsetattr_stdin_drain(t: &Termios) -> io::Result<()> {
        // SAFETY: `t` is a valid termios the kernel only reads.
        unsafe {
            if libc::tcsetattr(0, libc::TCSADRAIN, t) != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }

    /// The editor's raw-mode recipe: no flow control or CR/NL mangling
    /// (`IGNCR` too — with it set, the `\r` that Enter sends would be
    /// discarded), no 7-bit stripping (`ISTRIP` would corrupt every
    /// UTF-8 high byte), no canonical mode, echo, signals, or
    /// literal-next; output stays cooked so ordinary `println!` still
    /// works. `VMIN = 1, VTIME = 0`: a `read` blocks until at least one
    /// byte is available, then returns immediately with whatever the
    /// kernel already has queued — up to however many bytes the caller
    /// asked for, not necessarily just one. `read_stdin_chunk` below
    /// exploits exactly this to batch many already-queued keystrokes (or
    /// a paste) into a single syscall instead of reading one byte at a
    /// time.
    pub fn apply_raw_flags(t: &mut Termios) {
        t.c_iflag &= !(libc::IXON | libc::ICRNL | libc::INLCR | libc::IGNCR | libc::ISTRIP);
        t.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG | libc::IEXTEN);
        t.c_cc[libc::VMIN] = 1;
        t.c_cc[libc::VTIME] = 0;
    }

    /// Does an attribute set still match the raw-mode recipe? What the
    /// idle tick checks to heal the terminal after an external
    /// SIGTSTP/SIGCONT or a host command that ran `stty`.
    pub fn is_raw(t: &Termios) -> bool {
        t.c_iflag & (libc::IXON | libc::ICRNL | libc::INLCR | libc::IGNCR | libc::ISTRIP) == 0
            && t.c_lflag & (libc::ICANON | libc::ECHO | libc::ISIG | libc::IEXTEN) == 0
    }

    /// Is a byte ready on stdin within `ms` milliseconds? A signal
    /// interrupting the poll retries it — EINTR must not read as "no
    /// input", which upstream would misdecode as a lone ESC or a
    /// timed-out escape sequence.
    pub fn poll_stdin(ms: i32) -> bool {
        loop {
            let mut pfd = libc::pollfd {
                fd: 0,
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: single valid pollfd, count 1.
            let n = unsafe { libc::poll(&mut pfd, 1, ms) };
            if n >= 0 {
                return n > 0;
            }
            if io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                return false;
            }
        }
    }

    /// Read up to `buf.len()` bytes from stdin in one syscall: `Ok(0)` at
    /// EOF, `Ok(n)` for `n` bytes read (may be less than `buf.len()` —
    /// a single `read` on a tty returns whatever the kernel line
    /// discipline currently has queued, which is exactly what the
    /// caller's userspace buffer should hold). Lets a caller collecting
    /// many bytes (a paste, a burst of escape sequences) pay one syscall
    /// instead of one per byte.
    pub fn read_stdin_chunk(buf: &mut [u8]) -> io::Result<usize> {
        // SAFETY: writing at most `buf.len()` bytes into `buf`.
        let n = unsafe { libc::read(0, buf.as_mut_ptr().cast(), buf.len()) };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(n as usize)
    }

    /// Terminal width in columns from stdout, if available and non-zero.
    pub fn term_cols_stdout() -> Option<usize> {
        term_size_stdout().map(|(cols, _)| cols as usize)
    }

    /// Terminal size from stdout as (columns, rows), if available and
    /// non-zero in both dimensions.
    pub fn term_size_stdout() -> Option<(u16, u16)> {
        // SAFETY: `ws` is a valid, zeroed winsize the kernel fills.
        unsafe {
            let mut ws: libc::winsize = std::mem::zeroed();
            if libc::ioctl(1, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0 && ws.ws_row > 0 {
                return Some((ws.ws_col, ws.ws_row));
            }
        }
        None
    }

    /// Turn terminal echo off in an attribute set (`stty -echo`'s bit).
    pub fn clear_echo_flag(t: &mut Termios) {
        t.c_lflag &= !libc::ECHO;
    }
}

// ---- rusty_libc backend: Linux default -----------------------------------
#[cfg(all(
    target_os = "linux",
    feature = "rusty-libc",
    not(feature = "libc-backend")
))]
mod imp {
    use rusty_libc::{Errno, fd, termios, tty};
    use std::io;

    /// Opaque terminal-attributes value, sized and laid out by the backend.
    pub type Termios = termios::Termios;

    /// Build an `io::Error` from a raw-syscall `Errno` (see the module note on
    /// why `last_os_error` cannot be used here).
    fn to_io(e: Errno) -> io::Error {
        io::Error::from_raw_os_error(e.code())
    }

    /// Is stdin a terminal?
    pub fn isatty_stdin() -> bool {
        termios::isatty(0)
    }

    /// Is stdout a terminal? The editor paints on stdout, so raw-mode
    /// editing needs it to be one just as much as stdin.
    pub fn isatty_stdout() -> bool {
        termios::isatty(1)
    }

    /// Read stdin's terminal attributes.
    pub fn tcgetattr_stdin() -> io::Result<Termios> {
        termios::tcgetattr(0).map_err(to_io)
    }

    /// Apply attributes to stdin, draining pending output first (`TCSADRAIN`,
    /// which `rusty_libc::termios::tcsetattr` always uses).
    pub fn tcsetattr_stdin_drain(t: &Termios) -> io::Result<()> {
        termios::tcsetattr(0, t).map_err(to_io)
    }

    /// The editor's raw-mode recipe (identical to the libc backend's, using
    /// the same kernel flag bits — see that backend for the `IGNCR`/
    /// `ISTRIP` rationale). Deliberately lighter than
    /// `Termios::make_raw`: output processing is left on.
    pub fn apply_raw_flags(t: &mut Termios) {
        t.c_iflag &=
            !(termios::IXON | termios::ICRNL | termios::INLCR | termios::IGNCR | termios::ISTRIP);
        t.c_lflag &= !(termios::ICANON | termios::ECHO | termios::ISIG | termios::IEXTEN);
        t.c_cc[termios::VMIN] = 1;
        t.c_cc[termios::VTIME] = 0;
    }

    /// Does an attribute set still match the raw-mode recipe? What the
    /// idle tick checks to heal the terminal after an external
    /// SIGTSTP/SIGCONT or a host command that ran `stty`.
    pub fn is_raw(t: &Termios) -> bool {
        t.c_iflag
            & (termios::IXON | termios::ICRNL | termios::INLCR | termios::IGNCR | termios::ISTRIP)
            == 0
            && t.c_lflag & (termios::ICANON | termios::ECHO | termios::ISIG | termios::IEXTEN) == 0
    }

    /// Is a byte ready on stdin within `ms` milliseconds? A signal
    /// interrupting the poll retries it — EINTR must not read as "no
    /// input" (see the libc backend).
    pub fn poll_stdin(ms: i32) -> bool {
        loop {
            let mut fds = [fd::PollFd {
                fd: 0,
                events: fd::POLLIN,
                revents: 0,
            }];
            match fd::poll(&mut fds, ms) {
                Ok(n) => return n > 0,
                Err(e) if to_io(e).kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => return false,
            }
        }
    }

    /// Read up to `buf.len()` bytes from stdin in one syscall (see the
    /// libc backend for the rationale — one read call instead of one per
    /// byte when collecting a paste or a burst of escape sequences).
    pub fn read_stdin_chunk(buf: &mut [u8]) -> io::Result<usize> {
        fd::read(0, buf).map_err(to_io)
    }

    /// Terminal width in columns from stdout, if available and non-zero.
    pub fn term_cols_stdout() -> Option<usize> {
        term_size_stdout().map(|(cols, _)| cols as usize)
    }

    /// Terminal size from stdout as (columns, rows), if available and
    /// non-zero in both dimensions.
    pub fn term_size_stdout() -> Option<(u16, u16)> {
        match tty::window_size(1) {
            Ok(ws) if ws.ws_col > 0 && ws.ws_row > 0 => Some((ws.ws_col, ws.ws_row)),
            _ => None,
        }
    }

    /// Turn terminal echo off in an attribute set (`stty -echo`'s bit).
    pub fn clear_echo_flag(t: &mut Termios) {
        t.c_lflag &= !termios::ECHO;
    }
}

// ---- rusty_win32 backend: Windows -----------------------------------------
#[cfg(windows)]
mod imp {
    use rusty_win32::console::{
        ENABLE_ECHO_INPUT, ENABLE_EXTENDED_FLAGS, ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT,
        ENABLE_QUICK_EDIT_MODE, ENABLE_VIRTUAL_TERMINAL_INPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
    };
    use std::io;
    use std::os::windows::io::AsRawHandle;

    /// Windows tracks input and output console modes as two independent
    /// `DWORD`s (`GetConsoleMode` on the stdin handle vs. the stdout
    /// handle), unlike a single Unix `termios` covering both — bundled into
    /// one value here so the rest of this facade's interface (and every
    /// call site in `lib.rs`) stays identical across backends.
    #[derive(Debug, Clone, Copy)]
    pub struct Termios {
        input_mode: u32,
        output_mode: u32,
    }

    fn stdin_handle() -> rusty_win32::RawHandle {
        io::stdin().as_raw_handle() as rusty_win32::RawHandle
    }

    fn stdout_handle() -> rusty_win32::RawHandle {
        io::stdout().as_raw_handle() as rusty_win32::RawHandle
    }

    fn to_io(e: rusty_win32::Win32Error) -> io::Error {
        e.into()
    }

    /// Is stdin a terminal? `std::io::IsTerminal` already handles this
    /// portably (it resolves to `GetConsoleMode` succeeding under the
    /// hood on Windows) — no `rusty_win32` call needed.
    pub fn isatty_stdin() -> bool {
        io::IsTerminal::is_terminal(&io::stdin())
    }

    /// Is stdout a terminal? The editor paints on stdout, so raw-mode
    /// editing needs it to be one just as much as stdin.
    pub fn isatty_stdout() -> bool {
        io::IsTerminal::is_terminal(&io::stdout())
    }

    /// Read stdin's console input mode and stdout's console output mode —
    /// the Windows analog of `tcgetattr`, bundled per the module doc note.
    pub fn tcgetattr_stdin() -> io::Result<Termios> {
        // SAFETY: both handles come from `AsRawHandle` on this process's
        // own open stdin/stdout, valid for the duration of this call.
        let input_mode =
            unsafe { rusty_win32::console::get_mode(stdin_handle()) }.map_err(to_io)?;
        // SAFETY: see above.
        let output_mode =
            unsafe { rusty_win32::console::get_mode(stdout_handle()) }.map_err(to_io)?;
        Ok(Termios {
            input_mode,
            output_mode,
        })
    }

    /// Apply `t`'s modes to stdin's console input mode and stdout's console
    /// output mode — the Windows analog of `tcsetattr`. Windows console
    /// mode changes take effect immediately; there's no `TCSADRAIN`-style
    /// drain-first mode to opt into or out of.
    pub fn tcsetattr_stdin_drain(t: &Termios) -> io::Result<()> {
        // SAFETY: `stdin_handle()`/`stdout_handle()` are this process's own
        // open, valid handles; `t`'s fields are plain bitmasks Windows
        // itself reported as valid via a prior `tcgetattr_stdin` (or a
        // caller-derived variant of one).
        unsafe { rusty_win32::console::set_mode(stdin_handle(), t.input_mode) }.map_err(to_io)?;
        // SAFETY: see above.
        unsafe { rusty_win32::console::set_mode(stdout_handle(), t.output_mode) }.map_err(to_io)?;
        Ok(())
    }

    /// The editor's raw-mode recipe (Windows shape of the same intent as
    /// the Unix backends' `apply_raw_flags`): no line buffering or echo
    /// (`ENABLE_LINE_INPUT`/`ENABLE_ECHO_INPUT` off — the analogs of
    /// `ICANON`/`ECHO`), Ctrl+C delivered as ordinary input instead of the
    /// OS terminating the process (`ENABLE_PROCESSED_INPUT` off — the
    /// analog of `ISIG`), the console's own Quick Edit mouse-selection UI
    /// disabled (`ENABLE_QUICK_EDIT_MODE` off, gated by
    /// `ENABLE_EXTENDED_FLAGS` on — a Windows-only concern with no Unix
    /// analog), and VT/ANSI escape sequences flowing both directions
    /// (`ENABLE_VIRTUAL_TERMINAL_INPUT`/`ENABLE_VIRTUAL_TERMINAL_PROCESSING`
    /// on — what makes arrow keys arrive as bytes this crate's existing
    /// CSI decoder already parses, and repaint escape sequences render).
    /// Output stays otherwise cooked (`ENABLE_PROCESSED_OUTPUT` untouched),
    /// matching the Unix backends' own "output processing is left on"
    /// policy.
    pub fn apply_raw_flags(t: &mut Termios) {
        t.input_mode &= !(ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT);
        t.input_mode |= ENABLE_VIRTUAL_TERMINAL_INPUT | ENABLE_EXTENDED_FLAGS;
        t.input_mode &= !ENABLE_QUICK_EDIT_MODE;
        t.output_mode |= ENABLE_VIRTUAL_TERMINAL_PROCESSING;
    }

    /// Does an attribute set still match the raw-mode recipe? What the
    /// idle tick checks to heal the terminal after a host command that
    /// reset the console mode (Windows' analog of the Unix backends'
    /// external-`stty`/`SIGTSTP`-`SIGCONT` healing case).
    pub fn is_raw(t: &Termios) -> bool {
        t.input_mode & (ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT) == 0
            && t.input_mode & ENABLE_VIRTUAL_TERMINAL_INPUT != 0
            && t.output_mode & ENABLE_VIRTUAL_TERMINAL_PROCESSING != 0
    }

    /// Is a byte ready on stdin within `ms` milliseconds (`< 0` blocks
    /// indefinitely, matching `poll(2)`'s `-1` convention the Unix
    /// backends pass through directly)? `WaitForSingleObject` on a console
    /// input handle becomes signaled once at least one unread input
    /// record is queued — the Windows analog of `poll(POLLIN)` readiness.
    /// Unlike the Unix backends, no `EINTR`-style retry loop is needed:
    /// Windows waits aren't interrupted by an asynchronous signal the way
    /// a POSIX blocking syscall is.
    pub fn poll_stdin(ms: i32) -> bool {
        let timeout = if ms < 0 { u32::MAX } else { ms as u32 };
        // SAFETY: `stdin_handle()` is this process's own open, valid,
        // waitable console input handle.
        unsafe { rusty_win32::console::wait_readable(stdin_handle(), timeout) }.unwrap_or(false)
    }

    /// Read up to `buf.len()` bytes from stdin in one call: `Ok(0)` at EOF,
    /// `Ok(n)` for `n` bytes read (may be less than `buf.len()`) — with
    /// `ENABLE_VIRTUAL_TERMINAL_INPUT` set (always true whenever this is
    /// called through the raw-mode path — see `apply_raw_flags`),
    /// `ReadFile` on a console input handle delivers a VT/ANSI byte stream
    /// the same shape as a Unix `read` on a raw-mode tty, letting a caller
    /// collecting many bytes (a paste, a burst of escape sequences) pay
    /// one call instead of one per byte, exactly like the Unix backends.
    pub fn read_stdin_chunk(buf: &mut [u8]) -> io::Result<usize> {
        // SAFETY: `stdin_handle()` is this process's own open, valid
        // handle; `buf` is a valid, writable buffer of its own stated
        // length.
        unsafe { rusty_win32::console::read(stdin_handle(), buf) }.map_err(to_io)
    }

    /// Terminal width in columns from stdout, if available and non-zero.
    pub fn term_cols_stdout() -> Option<usize> {
        term_size_stdout().map(|(cols, _)| cols as usize)
    }

    /// Terminal size from stdout as (columns, rows), if available and
    /// non-zero in both dimensions — `GetConsoleScreenBufferInfo`'s visible
    /// window, the Windows analog of `TIOCGWINSZ`.
    pub fn term_size_stdout() -> Option<(u16, u16)> {
        // SAFETY: `stdout_handle()` is this process's own open, valid
        // console output handle.
        match unsafe { rusty_win32::console::window_size(stdout_handle()) } {
            Ok((cols, rows)) if cols > 0 && rows > 0 => Some((cols, rows)),
            _ => None,
        }
    }

    /// Turn terminal echo off in an attribute set (`stty -echo`'s Windows
    /// analog — `ENABLE_ECHO_INPUT`).
    pub fn clear_echo_flag(t: &mut Termios) {
        t.input_mode &= !ENABLE_ECHO_INPUT;
    }

    // `apply_raw_flags`/`is_raw`/`clear_echo_flag` are pure bit-math over an
    // in-memory `Termios` — no console handle, no FFI call — so they're
    // testable on any CI runner regardless of console attachment, unlike
    // the handle-taking functions above (already covered by `rusty_win32`'s
    // own tests at the primitive layer, verified on real `windows-latest`
    // CI in that crate).
    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn apply_raw_flags_produces_a_state_is_raw_accepts() {
            let mut t = Termios {
                input_mode: 0,
                output_mode: 0,
            };
            apply_raw_flags(&mut t);
            assert!(is_raw(&t));
        }

        #[test]
        fn is_raw_rejects_line_buffered_input() {
            let t = Termios {
                input_mode: ENABLE_LINE_INPUT | ENABLE_VIRTUAL_TERMINAL_INPUT,
                output_mode: ENABLE_VIRTUAL_TERMINAL_PROCESSING,
            };
            assert!(
                !is_raw(&t),
                "ENABLE_LINE_INPUT still set must not read as raw"
            );
        }

        #[test]
        fn is_raw_rejects_missing_vt_processing() {
            let t = Termios {
                input_mode: ENABLE_VIRTUAL_TERMINAL_INPUT,
                output_mode: 0,
            };
            assert!(
                !is_raw(&t),
                "no ENABLE_VIRTUAL_TERMINAL_PROCESSING must not read as raw"
            );
        }

        #[test]
        fn clear_echo_flag_only_clears_echo() {
            let mut t = Termios {
                input_mode: ENABLE_ECHO_INPUT | ENABLE_LINE_INPUT,
                output_mode: 0,
            };
            clear_echo_flag(&mut t);
            assert_eq!(
                t.input_mode, ENABLE_LINE_INPUT,
                "line input must survive; only echo clears"
            );
        }
    }
}
