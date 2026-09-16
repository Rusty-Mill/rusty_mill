//! Terminal window size query (`TIOCGWINSZ`) and pty pair allocation
//! (`/dev/ptmx` + `TIOCSPTLCK`/`TIOCGPTN`).

use crate::arch::Errno;
use crate::fd::ioctl;

/// `ioctl` request: get the terminal window size.
const TIOCGWINSZ: usize = 0x5413;

/// Terminal dimensions. Kernel `struct winsize` layout.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Winsize {
    /// Rows, in characters.
    pub ws_row: u16,
    /// Columns, in characters.
    pub ws_col: u16,
    /// Width, in pixels (often 0).
    pub ws_xpixel: u16,
    /// Height, in pixels (often 0).
    pub ws_ypixel: u16,
}

const _: () = assert!(core::mem::size_of::<Winsize>() == 8);

/// Query the window size of the terminal referred to by `fd`.
pub fn window_size(fd: i32) -> Result<Winsize, Errno> {
    let mut ws = Winsize::default();
    // SAFETY: `TIOCGWINSZ` expects a `*mut winsize`; `ws` is exactly that and
    // exclusively borrowed.
    unsafe { ioctl(fd, TIOCGWINSZ, &mut ws as *mut Winsize as usize) }?;
    Ok(ws)
}

/// `ioctl` request: unlock a `/dev/ptmx` master's slave device.
const TIOCSPTLCK: usize = 0x4004_5431;
/// `ioctl` request: fetch a `/dev/ptmx` master's slave device number.
const TIOCGPTN: usize = 0x8004_5430;

/// Allocate a new pseudoterminal pair, returning `(master, slave)` fds.
///
/// Opens `/dev/ptmx` for the master, unlocks the kernel-assigned slave
/// (`TIOCSPTLCK`), discovers its number (`TIOCGPTN`), and opens
/// `/dev/pts/<n>` for the slave — the sequence every `openpty(3)`-style
/// helper performs, exposed directly since nothing in this crate did
/// before. Running a subprocess under a pty (as opposed to a plain pipe)
/// gets it a real controlling-terminal-shaped fd: programs that check
/// `isatty()` before doing line buffering, prompting, or enabling color
/// see a tty, and terminal ioctls like [`crate::termios::tcsetattr`] or
/// [`window_size`] work on it exactly as they would on a real terminal.
///
/// Both fds are plain, blocking, `O_RDWR` — apply `O_NONBLOCK`/`O_CLOEXEC`
/// via [`crate::fd::fcntl`] afterward if needed, the same as for any other
/// fd this crate returns.
pub fn openpty() -> Result<(i32, i32), Errno> {
    let master = crate::fd::open(c"/dev/ptmx", crate::fd::O_RDWR, 0)?;

    // Unlock the slave (TIOCSPTLCK with a zero lock value) and discover
    // its device number; either failing leaves nothing to clean up but
    // the master itself.
    let unlock: i32 = 0;
    // SAFETY: `TIOCSPTLCK` expects a `*const i32` lock flag; `unlock` is
    // exactly that.
    if let Err(e) = unsafe { ioctl(master, TIOCSPTLCK, &unlock as *const i32 as usize) } {
        let _ = crate::fd::close(master);
        return Err(e);
    }
    let mut ptn: i32 = 0;
    // SAFETY: `TIOCGPTN` expects a `*mut i32`; `ptn` is exactly that and
    // exclusively borrowed.
    if let Err(e) = unsafe { ioctl(master, TIOCGPTN, &mut ptn as *mut i32 as usize) } {
        let _ = crate::fd::close(master);
        return Err(e);
    }

    // Build "/dev/pts/<ptn>\0" without allocating (this crate's no_std
    // core has no allocator): ptn is a small, non-negative kernel-assigned
    // index, so a fixed-size stack buffer comfortably covers every
    // possible value (even the full range of a 32-bit index).
    let mut path_buf = [0u8; 21]; // b"/dev/pts/" (9) + 10 digits + NUL
    let path = format_pts_path(ptn, &mut path_buf);

    match crate::fd::open(path, crate::fd::O_RDWR, 0) {
        Ok(slave) => Ok((master, slave)),
        Err(e) => {
            let _ = crate::fd::close(master);
            Err(e)
        }
    }
}

/// Render `/dev/pts/<ptn>` (`ptn >= 0`) into `buf`, NUL-terminated.
fn format_pts_path(ptn: i32, buf: &mut [u8; 21]) -> &core::ffi::CStr {
    const PREFIX: &[u8] = b"/dev/pts/";
    buf[..PREFIX.len()].copy_from_slice(PREFIX);

    let mut digits = [0u8; 10];
    let mut n = ptn as u32;
    let mut len = 0;
    loop {
        digits[len] = b'0' + (n % 10) as u8;
        len += 1;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    digits[..len].reverse();

    let end = PREFIX.len() + len;
    buf[PREFIX.len()..end].copy_from_slice(&digits[..len]);
    buf[end] = 0;
    // SAFETY: `buf[..end]` holds only the "/dev/pts/" prefix and ASCII
    // digits (no NUL byte among them), and `buf[end]` is the sole
    // terminating NUL -- exactly `CStr::from_bytes_with_nul`'s contract.
    core::ffi::CStr::from_bytes_with_nul(&buf[..=end]).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_tty_fails() {
        // A pipe is not a terminal: expect ENOTTY (25).
        let (r, w) = crate::fd::pipe2(0).unwrap();
        assert_eq!(window_size(r), Err(Errno(25)));
        crate::fd::close(r).unwrap();
        crate::fd::close(w).unwrap();
    }

    #[test]
    fn openpty_allocates_a_working_pair() {
        let (master, slave) = openpty().expect("openpty");
        assert_ne!(master, slave);
        assert!(master >= 0);
        assert!(slave >= 0);

        // Slave-to-master is the terminal "output" direction and isn't
        // subject to canonical-mode line buffering (unlike master-to-slave,
        // which is fully exercised by termios.rs's own PTY integration
        // tests via this same function), so a plain write/read roundtrip
        // works here without needing a trailing newline.
        let n = crate::fd::write(slave, b"hi").expect("write to slave");
        assert_eq!(n, 2);
        let mut buf = [0u8; 8];
        let read_n = crate::fd::read(master, &mut buf).expect("read from master");
        assert_eq!(&buf[..read_n], b"hi");

        crate::fd::close(master).expect("close master");
        crate::fd::close(slave).expect("close slave");
    }

    #[test]
    fn format_pts_path_renders_expected_paths() {
        let mut buf = [0u8; 21];
        assert_eq!(format_pts_path(0, &mut buf).to_bytes(), b"/dev/pts/0");
        assert_eq!(format_pts_path(7, &mut buf).to_bytes(), b"/dev/pts/7");
        assert_eq!(format_pts_path(42, &mut buf).to_bytes(), b"/dev/pts/42");
        assert_eq!(
            format_pts_path(i32::MAX, &mut buf).to_bytes(),
            b"/dev/pts/2147483647"
        );
    }
}
