//! Terminal attributes using the **kernel's** `struct termios` (`NCCS = 19`,
//! no glibc `c_ispeed`/`c_ospeed` fields), plus the controlling-terminal
//! foreground-group queries, `isatty`, and the [`Termios::make_raw`]
//! raw-mode recipe.
//!
//! Phase 1 provided the struct, `tcgetattr`/`tcsetattr`, `tcgetpgrp`/
//! `tcsetpgrp`, `isatty`, and the flag subset rush's line editor toggles.
//! Phase 2 adds the raw-mode transformation and validates the whole termios
//! path against a real terminal (see the PTY integration tests).

use crate::arch::Errno;
use crate::fd::ioctl;
use crate::fs;
use core::ffi::CStr;

/// Number of control characters in the kernel `struct termios`.
pub const NCCS: usize = 19;

// ioctl requests (asm-generic / x86_64).
const TCGETS: usize = 0x5401;
const TCSETS: usize = 0x5402; // TCSANOW: set immediately.
const TCSETSW: usize = 0x5403; // TCSADRAIN: drain output, then set.
const TCSETSF: usize = 0x5404; // TCSAFLUSH: drain output, flush input, then set.
const TCSBRK: usize = 0x5409; // with arg != 0, wait for output to drain (tcdrain).
const TCXONC: usize = 0x540a; // suspend/resume output or input (tcflow).
const TCFLSH: usize = 0x540b; // flush input and/or output queues (tcflush).
const TIOCGPGRP: usize = 0x540f;
const TIOCSPGRP: usize = 0x5410;
const TCSBRKP: usize = 0x5425; // POSIX tcsendbreak, distinct from plain TCSBRK.
const TIOCGSID: usize = 0x5429;

/// [`tcsetattr_with`] action: apply the change immediately.
pub const TCSANOW: i32 = 0;
/// [`tcsetattr_with`] action: apply after pending output drains (the default
/// used by [`tcsetattr`]).
pub const TCSADRAIN: i32 = 1;
/// [`tcsetattr_with`] action: drain pending output and discard pending input,
/// then apply. The usual choice when restoring the terminal on exit.
pub const TCSAFLUSH: i32 = 2;

/// [`tcflush`] queue selector: discard unread input.
pub const TCIFLUSH: i32 = 0;
/// [`tcflush`] queue selector: discard unwritten output.
pub const TCOFLUSH: i32 = 1;
/// [`tcflush`] queue selector: discard both input and output.
pub const TCIOFLUSH: i32 = 2;

/// [`tcflow`] action: suspend output.
pub const TCOOFF: i32 = 0;
/// [`tcflow`] action: resume suspended output.
pub const TCOON: i32 = 1;
/// [`tcflow`] action: transmit a STOP character, intended to suspend the
/// terminal's own output back to us.
pub const TCIOFF: i32 = 2;
/// [`tcflow`] action: transmit a START character, intended to resume the
/// terminal's own output back to us.
pub const TCION: i32 = 3;

// Input flags (`c_iflag`).
/// Ignore BREAK conditions on input.
pub const IGNBRK: u32 = 0o0000001;
/// Signal `SIGINT` on BREAK (else map to `\0` or discard).
pub const BRKINT: u32 = 0o0000002;
/// Mark parity/framing errors with a `\377 \0` prefix.
pub const PARMRK: u32 = 0o0000010;
/// Strip the eighth bit off input characters.
pub const ISTRIP: u32 = 0o0000040;
/// Map NL to CR on input.
pub const INLCR: u32 = 0o0000100;
/// Ignore CR on input.
pub const IGNCR: u32 = 0o0000200;
/// Map CR to NL on input.
pub const ICRNL: u32 = 0o0000400;
/// Enable start/stop output control.
pub const IXON: u32 = 0o0002000;

// Output flags (`c_oflag`).
/// Enable implementation-defined output processing.
pub const OPOST: u32 = 0o0000001;

// Control flags (`c_cflag`).
/// Character-size mask.
pub const CSIZE: u32 = 0o0000060;
/// 8 bits per character.
pub const CS8: u32 = 0o0000060;
/// Enable parity generation on output and checking on input.
pub const PARENB: u32 = 0o0000400;

// Local flags (`c_lflag`).
/// Enable signals (`INTR`, `QUIT`, `SUSP`).
pub const ISIG: u32 = 0o0000001;
/// Canonical (line-buffered) input.
pub const ICANON: u32 = 0o0000002;
/// Echo input characters.
pub const ECHO: u32 = 0o0000010;
/// Echo NL even when `ECHO` is off.
pub const ECHONL: u32 = 0o0000100;
/// Enable implementation-defined input processing.
pub const IEXTEN: u32 = 0o0100000;

// Indices into `c_cc` (asm-generic / x86_64 / aarch64 ordering).
/// `c_cc` index: INTR character (sends `SIGINT`, typically Ctrl-C).
pub const VINTR: usize = 0;
/// `c_cc` index: QUIT character (sends `SIGQUIT`, typically Ctrl-\).
pub const VQUIT: usize = 1;
/// `c_cc` index: ERASE character (erase last char, typically Backspace).
pub const VERASE: usize = 2;
/// `c_cc` index: KILL character (erase current line).
pub const VKILL: usize = 3;
/// `c_cc` index: EOF character (typically Ctrl-D).
pub const VEOF: usize = 4;
/// `c_cc` index: read timeout (tenths of a second) for non-canonical reads.
pub const VTIME: usize = 5;
/// `c_cc` index: minimum bytes for a non-canonical read.
pub const VMIN: usize = 6;
/// `c_cc` index: SWTC character (switch, rarely used).
pub const VSWTC: usize = 7;
/// `c_cc` index: START character (resume output, typically Ctrl-Q).
pub const VSTART: usize = 8;
/// `c_cc` index: STOP character (pause output, typically Ctrl-S).
pub const VSTOP: usize = 9;
/// `c_cc` index: SUSP character (sends `SIGTSTP`, typically Ctrl-Z).
pub const VSUSP: usize = 10;
/// `c_cc` index: EOL character (additional line terminator).
pub const VEOL: usize = 11;
/// `c_cc` index: REPRINT character (redraw the line, typically Ctrl-R).
pub const VREPRINT: usize = 12;
/// `c_cc` index: DISCARD character (toggle output discard, typically Ctrl-O).
pub const VDISCARD: usize = 13;
/// `c_cc` index: WERASE character (erase last word, typically Ctrl-W).
pub const VWERASE: usize = 14;
/// `c_cc` index: LNEXT character (quote the next char literally, typically Ctrl-V).
pub const VLNEXT: usize = 15;
/// `c_cc` index: EOL2 character (a second additional line terminator).
pub const VEOL2: usize = 16;

/// Kernel `struct termios`. Field order and sizes match `<asm/termbits.h>`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Termios {
    /// Input mode flags.
    pub c_iflag: u32,
    /// Output mode flags.
    pub c_oflag: u32,
    /// Control mode flags.
    pub c_cflag: u32,
    /// Local mode flags.
    pub c_lflag: u32,
    /// Line discipline.
    pub c_line: u8,
    /// Control characters, indexed by the `V*` constants.
    pub c_cc: [u8; NCCS],
}

const _: () = assert!(core::mem::size_of::<Termios>() == 36);
const _: () = assert!(core::mem::offset_of!(Termios, c_iflag) == 0);
const _: () = assert!(core::mem::offset_of!(Termios, c_oflag) == 4);
const _: () = assert!(core::mem::offset_of!(Termios, c_cflag) == 8);
const _: () = assert!(core::mem::offset_of!(Termios, c_lflag) == 12);
const _: () = assert!(core::mem::offset_of!(Termios, c_line) == 16);
const _: () = assert!(core::mem::offset_of!(Termios, c_cc) == 17);

impl Default for Termios {
    fn default() -> Self {
        Termios {
            c_iflag: 0,
            c_oflag: 0,
            c_cflag: 0,
            c_lflag: 0,
            c_line: 0,
            c_cc: [0; NCCS],
        }
    }
}

impl Termios {
    /// Transform these attributes into "raw" mode in place: no canonical line
    /// processing, no echo, no signal generation, no CR/NL or output
    /// translation, 8-bit characters, and byte-at-a-time reads (`VMIN = 1`,
    /// `VTIME = 0`).
    ///
    /// This is the canonical `cfmakeraw(3)` recipe. The typical use is
    /// `let saved = tcgetattr(fd)?; let mut raw = saved; raw.make_raw();
    /// tcsetattr(fd, &raw)?;` then restore `saved` on exit. Callers that need
    /// finer control can manipulate the flag fields directly with the `pub`
    /// constants in this module.
    pub fn make_raw(&mut self) {
        self.c_iflag &= !(IGNBRK | BRKINT | PARMRK | ISTRIP | INLCR | IGNCR | ICRNL | IXON);
        self.c_oflag &= !OPOST;
        self.c_lflag &= !(ECHO | ECHONL | ICANON | ISIG | IEXTEN);
        self.c_cflag &= !(CSIZE | PARENB);
        self.c_cflag |= CS8;
        self.c_cc[VMIN] = 1;
        self.c_cc[VTIME] = 0;
    }
}

/// Read the terminal attributes of `fd`.
pub fn tcgetattr(fd: i32) -> Result<Termios, Errno> {
    let mut t = Termios::default();
    // SAFETY: `TCGETS` expects a `*mut termios`; `t` is exactly that.
    unsafe { ioctl(fd, TCGETS, &mut t as *mut Termios as usize) }?;
    Ok(t)
}

/// Set the terminal attributes of `fd` after draining pending output
/// ([`TCSADRAIN`]). Convenience wrapper over [`tcsetattr_with`].
#[inline]
pub fn tcsetattr(fd: i32, termios: &Termios) -> Result<(), Errno> {
    tcsetattr_with(fd, TCSADRAIN, termios)
}

/// Set the terminal attributes of `fd`, choosing when the change takes effect
/// with `actions` ([`TCSANOW`], [`TCSADRAIN`], or [`TCSAFLUSH`]). An unknown
/// `actions` value is treated as [`TCSADRAIN`].
pub fn tcsetattr_with(fd: i32, actions: i32, termios: &Termios) -> Result<(), Errno> {
    let request = if actions == TCSANOW {
        TCSETS
    } else if actions == TCSAFLUSH {
        TCSETSF
    } else {
        TCSETSW
    };
    // SAFETY: each request expects a `*const termios`; `termios` is a valid
    // borrow the kernel only reads.
    unsafe { ioctl(fd, request, termios as *const Termios as usize) }?;
    Ok(())
}

/// Discard queued terminal data on `fd`: `queue` is [`TCIFLUSH`] (unread
/// input), [`TCOFLUSH`] (unwritten output), or [`TCIOFLUSH`] (both). A line
/// editor calls `tcflush(fd, TCIFLUSH)` to drop type-ahead after an interrupt.
pub fn tcflush(fd: i32, queue: i32) -> Result<(), Errno> {
    // TCFLSH takes the queue selector as an integer arg, not a pointer.
    // SAFETY: integer request; no memory is dereferenced.
    unsafe { ioctl(fd, TCFLSH, queue as usize) }?;
    Ok(())
}

/// Block until all output written to `fd` has been transmitted (`tcdrain`).
pub fn tcdrain(fd: i32) -> Result<(), Errno> {
    // tcdrain is TCSBRK with a non-zero argument (0 would send a BREAK).
    // SAFETY: integer request; no memory is dereferenced.
    unsafe { ioctl(fd, TCSBRK, 1) }?;
    Ok(())
}

/// Get the foreground process group ID of the terminal `fd`.
pub fn tcgetpgrp(fd: i32) -> Result<i32, Errno> {
    let mut pgrp: i32 = 0;
    // SAFETY: `TIOCGPGRP` expects a `*mut pid_t`.
    unsafe { ioctl(fd, TIOCGPGRP, &mut pgrp as *mut i32 as usize) }?;
    Ok(pgrp)
}

/// Set the foreground process group of the terminal `fd` to `pgrp`.
pub fn tcsetpgrp(fd: i32, pgrp: i32) -> Result<(), Errno> {
    // SAFETY: `TIOCSPGRP` expects a `*const pid_t` the kernel only reads.
    unsafe { ioctl(fd, TIOCSPGRP, &pgrp as *const i32 as usize) }?;
    Ok(())
}

/// Suspend or resume transmission/reception on terminal `fd` (`action` is
/// one of [`TCOOFF`]/[`TCOON`]/[`TCIOFF`]/[`TCION`]).
pub fn tcflow(fd: i32, action: i32) -> Result<(), Errno> {
    // TCXONC takes the action as an integer arg, not a pointer.
    // SAFETY: integer request; no memory is dereferenced.
    unsafe { ioctl(fd, TCXONC, action as usize) }?;
    Ok(())
}

/// Transmit a break condition on terminal `fd`. `duration == 0` sends a
/// break of the kernel's own default length (0.25-0.5s per POSIX); a
/// nonzero `duration` is interpreted by the kernel driver in
/// implementation-defined units (roughly hundreds of milliseconds on
/// Linux's serial drivers) -- there is no portable way to request an exact
/// duration, only "the default" vs. "some other, driver-defined length".
///
/// Uses `TCSBRKP`, the ioctl the kernel added specifically to back this
/// function (see `TCSBRKP`'s own kernel comment), distinct from the plain
/// `TCSBRK` [`tcdrain`] uses -- unlike `TCSBRK`, `TCSBRKP` sends a break for
/// *any* argument value, including nonzero ones, rather than treating a
/// nonzero argument as "just drain, no break".
pub fn tcsendbreak(fd: i32, duration: i32) -> Result<(), Errno> {
    // SAFETY: integer request; no memory is dereferenced.
    unsafe { ioctl(fd, TCSBRKP, duration as usize) }?;
    Ok(())
}

/// Get the session ID of the session terminal `fd` is the controlling
/// terminal for (the process group leader's pid at the time the session was
/// created, per POSIX `tcgetsid`).
pub fn tcgetsid(fd: i32) -> Result<i32, Errno> {
    let mut sid: i32 = 0;
    // SAFETY: `TIOCGSID` expects a `*mut pid_t`.
    unsafe { ioctl(fd, TIOCGSID, &mut sid as *mut i32 as usize) }?;
    Ok(sid)
}

/// Return `true` if `fd` refers to a terminal (a successful `TCGETS`).
#[inline]
pub fn isatty(fd: i32) -> bool {
    tcgetattr(fd).is_ok()
}

/// Format `/proc/self/fd/{fd}\0` into `out`, returning it as a `CStr`. Not a
/// distinct syscall: this is exactly how glibc's own `ttyname`/`ttyname_r`
/// resolve an fd to a path, via `readlink` on the `/proc` fd-symlink.
fn proc_fd_path(fd: i32, out: &mut [u8; 32]) -> &CStr {
    const PREFIX: &[u8] = b"/proc/self/fd/";
    out[..PREFIX.len()].copy_from_slice(PREFIX);
    let mut pos = PREFIX.len();

    if fd < 0 {
        out[pos] = b'-';
        pos += 1;
    }
    let start = pos;
    let mut n = fd.unsigned_abs();
    loop {
        out[pos] = b'0' + (n % 10) as u8;
        pos += 1;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    out[start..pos].reverse();

    out[pos] = 0;
    CStr::from_bytes_with_nul(&out[..=pos]).unwrap()
}

/// The path of the terminal device `fd` refers to, written into `buf`
/// (typically sized `64` or more — `/dev/pts/N`-style paths are short, but a
/// too-small `buf` fails with [`Errno::ERANGE`] rather than truncating).
///
/// Fails with [`Errno::ENOTTY`] if `fd` isn't a terminal at all — the same
/// check [`isatty`] makes — since resolving *some* path for a non-tty fd
/// (its readlink target always exists) wouldn't match `ttyname`'s contract.
pub fn ttyname(fd: i32, buf: &mut [u8]) -> Result<&[u8], Errno> {
    if !isatty(fd) {
        return Err(Errno::ENOTTY);
    }
    let mut path_buf = [0u8; 32];
    let path = proc_fd_path(fd, &mut path_buf);
    let full_len = buf.len();
    // Unlike a plain readlink, don't hand back a possibly-truncated path:
    // `readlink(2)` fills as many bytes as fit and reports that count with
    // no error, so a result exactly as long as `buf` is indistinguishable
    // from "fit perfectly" -- treat it as "didn't fit" instead, matching
    // glibc's own `ttyname_r` contract.
    let target = fs::readlinkat(fs::AT_FDCWD, path, buf)?;
    if target.len() == full_len {
        Err(Errno::ERANGE)
    } else {
        Ok(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipe_is_not_a_tty() {
        let (r, w) = crate::fd::pipe2(0).unwrap();
        assert!(!isatty(r));
        assert_eq!(tcgetattr(r), Err(Errno(25))); // ENOTTY
        crate::fd::close(r).unwrap();
        crate::fd::close(w).unwrap();
    }

    #[test]
    fn ttyname_on_a_pipe_is_enotty() {
        let (r, w) = crate::fd::pipe2(0).unwrap();
        let mut buf = [0u8; 64];
        assert_eq!(ttyname(r, &mut buf), Err(Errno::ENOTTY));
        crate::fd::close(r).unwrap();
        crate::fd::close(w).unwrap();
    }

    #[test]
    fn ttyname_on_a_pty_slave_reports_a_dev_pts_path() {
        use std::os::fd::AsRawFd;
        let (_m, s) = open_pty();
        let mut buf = [0u8; 64];
        let path = ttyname(s.as_raw_fd(), &mut buf).expect("ttyname");
        let path = core::str::from_utf8(path).expect("utf8");
        assert!(
            path.starts_with("/dev/pts/"),
            "expected a /dev/pts/N path, got {path:?}"
        );
    }

    #[test]
    fn ttyname_too_small_a_buffer_is_erange() {
        use std::os::fd::AsRawFd;
        let (_m, s) = open_pty();
        let mut buf = [0u8; 1];
        assert_eq!(ttyname(s.as_raw_fd(), &mut buf), Err(Errno::ERANGE));
    }

    #[test]
    fn layout_is_kernel_not_glibc() {
        // glibc's termios is 60 bytes (NCCS=32 + speed fields); the kernel's
        // is 36. The const assertions above already enforce this at compile
        // time; restate it as a runtime guard for clarity.
        assert_eq!(core::mem::size_of::<Termios>(), 36);
        assert_eq!(NCCS, 19);
    }

    #[test]
    fn make_raw_clears_the_expected_flags() {
        // Start from a "cooked" terminal with the interesting bits set.
        let mut t = Termios {
            c_iflag: ICRNL | IXON | ISTRIP | BRKINT,
            c_oflag: OPOST,
            c_lflag: ICANON | ECHO | ISIG | IEXTEN | ECHONL,
            c_cflag: PARENB, // and CSIZE bits absent → make_raw must set CS8
            ..Default::default()
        };
        t.c_cc[VMIN] = 0;
        t.c_cc[VTIME] = 5;

        t.make_raw();

        // Every processing flag cleared.
        assert_eq!(t.c_iflag, 0);
        assert_eq!(t.c_oflag & OPOST, 0);
        assert_eq!(t.c_lflag & (ICANON | ECHO | ISIG | IEXTEN | ECHONL), 0);
        // 8-bit, no parity.
        assert_eq!(t.c_cflag & PARENB, 0);
        assert_eq!(t.c_cflag & CSIZE, CS8);
        // Byte-at-a-time, no timeout.
        assert_eq!(t.c_cc[VMIN], 1);
        assert_eq!(t.c_cc[VTIME], 0);
    }

    // --- PTY integration: validate the real-terminal path, not just the
    // ENOTTY-on-a-pipe path. ---

    /// Open a `(master, slave)` pty pair as owned `File`s so the fds close
    /// on drop. Delegates the actual allocation to `tty::openpty` (this
    /// also doubles as that function's own integration coverage, since
    /// every test below exercises the pair it returns); the `File` wrap
    /// is purely so a panicking assertion mid-test doesn't need explicit
    /// cleanup.
    fn open_pty() -> (std::fs::File, std::fs::File) {
        use std::os::fd::FromRawFd;

        let (master, slave) = crate::tty::openpty().expect("openpty");
        // SAFETY: `openpty` returns two freshly opened, valid, owned fds
        // neither of which is used again outside these `File`s.
        unsafe {
            (
                std::fs::File::from_raw_fd(master),
                std::fs::File::from_raw_fd(slave),
            )
        }
    }

    #[test]
    fn pty_slave_is_a_tty() {
        use std::os::fd::AsRawFd;
        let (_m, s) = open_pty();
        assert!(isatty(s.as_raw_fd()));
        assert!(tcgetattr(s.as_raw_fd()).is_ok());
    }

    #[test]
    fn raw_mode_roundtrips_through_the_kernel() {
        use std::os::fd::AsRawFd;
        let (_m, s) = open_pty();
        let sfd = s.as_raw_fd();

        // A fresh pty slave comes up in cooked mode: ICANON and ECHO set.
        let cooked = tcgetattr(sfd).expect("tcgetattr");
        assert_ne!(cooked.c_lflag & ICANON, 0);
        assert_ne!(cooked.c_lflag & ECHO, 0);

        // Apply raw mode and read it back from the kernel.
        let mut raw = cooked;
        raw.make_raw();
        tcsetattr(sfd, &raw).expect("tcsetattr raw");

        let after = tcgetattr(sfd).expect("tcgetattr after");
        assert_eq!(after.c_lflag & (ICANON | ECHO | ISIG | IEXTEN), 0);
        assert_eq!(after.c_iflag & (ICRNL | IXON), 0);
        assert_eq!(after.c_cc[VMIN], 1);
        assert_eq!(after.c_cc[VTIME], 0);

        // Restore the saved attributes and confirm the round-trip.
        tcsetattr(sfd, &cooked).expect("tcsetattr restore");
        let restored = tcgetattr(sfd).expect("tcgetattr restored");
        assert_ne!(restored.c_lflag & ICANON, 0);
        assert_ne!(restored.c_lflag & ECHO, 0);
    }

    #[test]
    fn tcsetattr_with_flush_and_now_apply() {
        use std::os::fd::AsRawFd;
        let (_m, s) = open_pty();
        let sfd = s.as_raw_fd();

        let cooked = tcgetattr(sfd).expect("tcgetattr");
        let mut raw = cooked;
        raw.make_raw();

        // TCSAFLUSH applies raw mode (and discards pending input).
        tcsetattr_with(sfd, TCSAFLUSH, &raw).expect("tcsetattr_with FLUSH");
        let after = tcgetattr(sfd).expect("tcgetattr after");
        assert_eq!(after.c_lflag & (ICANON | ECHO), 0);

        // TCSANOW restores the saved attributes immediately.
        tcsetattr_with(sfd, TCSANOW, &cooked).expect("tcsetattr_with NOW");
        let restored = tcgetattr(sfd).expect("tcgetattr restored");
        assert_ne!(restored.c_lflag & ICANON, 0);
        assert_ne!(restored.c_lflag & ECHO, 0);
    }

    #[test]
    fn tcflush_and_tcdrain_on_pty() {
        use std::os::fd::AsRawFd;
        let (_m, s) = open_pty();
        let sfd = s.as_raw_fd();
        // Both are valid terminal operations on a pty slave.
        tcflush(sfd, TCIFLUSH).expect("tcflush input");
        tcflush(sfd, TCIOFLUSH).expect("tcflush both");
        tcdrain(sfd).expect("tcdrain");
    }

    #[test]
    fn tcsetpgrp_tcgetpgrp_roundtrip_on_pty() {
        use std::os::fd::AsRawFd;
        let (_m, s) = open_pty();
        let sfd = s.as_raw_fd();

        // Set the slave's foreground group to our own and read it back.
        // Without a controlling terminal the kernel may reject the set
        // (ENOTTY/EPERM); that is a valid, non-panicking outcome, so only
        // assert the round-trip when the set succeeds.
        let pgrp = crate::process::getpid();
        if tcsetpgrp(sfd, pgrp).is_ok() {
            assert_eq!(tcgetpgrp(sfd).expect("tcgetpgrp"), pgrp);
        }
    }

    #[test]
    fn tcflow_suspend_and_resume_on_pty() {
        use std::os::fd::AsRawFd;
        let (_m, s) = open_pty();
        let sfd = s.as_raw_fd();
        // Suspend then resume output; both are valid on a pty slave.
        tcflow(sfd, TCOOFF).expect("tcflow TCOOFF");
        tcflow(sfd, TCOON).expect("tcflow TCOON");
        // Same for the input direction.
        tcflow(sfd, TCIOFF).expect("tcflow TCIOFF");
        tcflow(sfd, TCION).expect("tcflow TCION");
    }

    #[test]
    fn tcflow_on_a_pipe_is_enotty() {
        let (r, w) = crate::fd::pipe2(0).unwrap();
        assert_eq!(tcflow(r, TCOOFF), Err(Errno(25))); // ENOTTY
        crate::fd::close(r).unwrap();
        crate::fd::close(w).unwrap();
    }

    #[test]
    fn tcsendbreak_on_pty() {
        use std::os::fd::AsRawFd;
        let (_m, s) = open_pty();
        let sfd = s.as_raw_fd();
        // Default-length break, and an explicit nonzero duration.
        tcsendbreak(sfd, 0).expect("tcsendbreak default");
        tcsendbreak(sfd, 1).expect("tcsendbreak explicit duration");
    }

    #[test]
    fn tcsendbreak_on_a_pipe_is_enotty() {
        let (r, w) = crate::fd::pipe2(0).unwrap();
        assert_eq!(tcsendbreak(r, 0), Err(Errno(25))); // ENOTTY
        crate::fd::close(r).unwrap();
        crate::fd::close(w).unwrap();
    }

    #[test]
    fn tcgetsid_on_pty() {
        use std::os::fd::AsRawFd;
        let (_m, s) = open_pty();
        let sfd = s.as_raw_fd();
        // Without a controlling terminal the kernel may reject this
        // (ENOTTY); that is a valid, non-panicking outcome here too (see
        // tcsetpgrp_tcgetpgrp_roundtrip_on_pty above for the same caveat).
        if let Ok(sid) = tcgetsid(sfd) {
            assert!(sid > 0);
        }
    }

    #[test]
    fn tcgetsid_on_a_pipe_is_enotty() {
        let (r, w) = crate::fd::pipe2(0).unwrap();
        assert_eq!(tcgetsid(r), Err(Errno(25))); // ENOTTY
        crate::fd::close(r).unwrap();
        crate::fd::close(w).unwrap();
    }
}
