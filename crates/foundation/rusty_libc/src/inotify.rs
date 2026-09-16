//! `inotify`: filesystem change notification (watch a path, get a stream of
//! create/delete/modify/move events back on a pollable fd) — composes with
//! [`crate::fd::poll`]/[`crate::epoll`] the same way `signalfd`/`timerfd`
//! already do.
//!
//! No confirmed `rush` consumer need drove this; it was filed and implemented
//! at the user's explicit request during the parity-loop sweep against
//! `libc`, same "no known consumer need" bar Round 4 applied to
//! `flock`/`chroot`/`sendfile` before declining them (see `REVIEW.md`).

use crate::arch::nr;
use crate::arch::{from_ret, from_ret_i32, syscall1, syscall2, syscall3, Errno};
use core::ffi::CStr;

/// `inotify_init1(2)` flag: set close-on-exec on the returned descriptor.
/// Same bit as [`crate::fd::O_CLOEXEC`].
pub const IN_CLOEXEC: i32 = 0o2000000;
/// `inotify_init1(2)` flag: open the descriptor non-blocking. Same bit as
/// [`crate::fd::O_NONBLOCK`].
pub const IN_NONBLOCK: i32 = 0o0004000;

/// File was accessed (read).
pub const IN_ACCESS: u32 = 0x0000_0001;
/// File was modified (write).
pub const IN_MODIFY: u32 = 0x0000_0002;
/// Metadata changed (permissions, timestamps, link count, …).
pub const IN_ATTRIB: u32 = 0x0000_0004;
/// A writable file was closed.
pub const IN_CLOSE_WRITE: u32 = 0x0000_0008;
/// A non-writable file was closed.
pub const IN_CLOSE_NOWRITE: u32 = 0x0000_0010;
/// File was opened.
pub const IN_OPEN: u32 = 0x0000_0020;
/// File was moved out of a watched directory.
pub const IN_MOVED_FROM: u32 = 0x0000_0040;
/// File was moved into a watched directory.
pub const IN_MOVED_TO: u32 = 0x0000_0080;
/// File/directory was created inside a watched directory.
pub const IN_CREATE: u32 = 0x0000_0100;
/// File/directory was deleted from a watched directory.
pub const IN_DELETE: u32 = 0x0000_0200;
/// The watched file/directory itself was deleted.
pub const IN_DELETE_SELF: u32 = 0x0000_0400;
/// The watched file/directory itself was moved.
pub const IN_MOVE_SELF: u32 = 0x0000_0800;
/// The watch's backing filesystem was unmounted.
pub const IN_UNMOUNT: u32 = 0x0000_2000;
/// The event queue overflowed (events were dropped); `wd` is `-1` on this
/// event.
pub const IN_Q_OVERFLOW: u32 = 0x0000_4000;
/// The watch was removed (explicitly via [`inotify_rm_watch`], or implicitly
/// because its target was deleted or the filesystem unmounted).
pub const IN_IGNORED: u32 = 0x0000_8000;
/// `IN_CLOSE_WRITE | IN_CLOSE_NOWRITE`.
pub const IN_CLOSE: u32 = IN_CLOSE_WRITE | IN_CLOSE_NOWRITE;
/// `IN_MOVED_FROM | IN_MOVED_TO`.
pub const IN_MOVE: u32 = IN_MOVED_FROM | IN_MOVED_TO;
/// `inotify_add_watch(2)` flag: only watch `path` if it is a directory.
pub const IN_ONLYDIR: u32 = 0x0100_0000;
/// `inotify_add_watch(2)` flag: don't dereference a symlink at `path`.
pub const IN_DONT_FOLLOW: u32 = 0x0200_0000;
/// `inotify_add_watch(2)` flag: don't report events for children after
/// they've been unlinked.
pub const IN_EXCL_UNLINK: u32 = 0x0400_0000;
/// `inotify_add_watch(2)` flag: fail with `EEXIST` if a watch already exists
/// for `path`, instead of adding to its mask.
pub const IN_MASK_CREATE: u32 = 0x1000_0000;
/// `inotify_add_watch(2)` flag: add to an existing watch's mask instead of
/// replacing it.
pub const IN_MASK_ADD: u32 = 0x2000_0000;
/// Event flag set by the kernel when the subject is a directory.
pub const IN_ISDIR: u32 = 0x4000_0000;
/// `inotify_add_watch(2)` flag: remove the watch after one event.
pub const IN_ONESHOT: u32 = 0x8000_0000;

/// Create a new inotify instance; returns a pollable fd. `flags` is
/// [`IN_CLOEXEC`]/[`IN_NONBLOCK`] (or `0` for neither).
pub fn inotify_init1(flags: i32) -> Result<i32, Errno> {
    let ret = unsafe { syscall1(nr::INOTIFY_INIT1, flags as usize) };
    from_ret_i32(ret)
}

/// Add (or update) a watch on `path`, reporting the events in `mask`.
/// Returns a watch descriptor, unique within `fd`, that shows up as `wd` on
/// events read back from `fd` and is passed to [`inotify_rm_watch`].
pub fn inotify_add_watch(fd: i32, path: &CStr, mask: u32) -> Result<i32, Errno> {
    let ret = unsafe {
        syscall3(
            nr::INOTIFY_ADD_WATCH,
            fd as usize,
            path.as_ptr() as usize,
            mask as usize,
        )
    };
    from_ret_i32(ret)
}

/// Remove watch descriptor `wd` (as returned by [`inotify_add_watch`]) from
/// `fd`. A trailing `IN_IGNORED` event is queued for it.
pub fn inotify_rm_watch(fd: i32, wd: i32) -> Result<(), Errno> {
    let ret = unsafe { syscall2(nr::INOTIFY_RM_WATCH, fd as usize, wd as usize) };
    from_ret(ret).map(|_| ())
}

/// One event parsed out of an [`inotify_events`]-filled buffer — borrows
/// directly from it, no allocation.
#[derive(Debug, Clone, Copy)]
pub struct InotifyEvent<'a> {
    /// The watch descriptor this event is for (as returned by
    /// [`inotify_add_watch`]), or `-1` on an [`IN_Q_OVERFLOW`] event.
    pub wd: i32,
    /// An [`IN_ACCESS`]/[`IN_CREATE`]/… bitmask describing what happened.
    pub mask: u32,
    /// Ties together a related `IN_MOVED_FROM`/`IN_MOVED_TO` pair (same
    /// nonzero cookie on both); `0` when unused.
    pub cookie: u32,
    name: &'a [u8],
}

impl<'a> InotifyEvent<'a> {
    /// The subject's basename, for events on a watched directory's children
    /// (`IN_CREATE`, `IN_DELETE`, …). `None` for events on the watch's own
    /// target (`IN_MODIFY` on a watched file, `IN_DELETE_SELF`, …), which
    /// carry no name.
    pub fn name(&self) -> Option<&'a CStr> {
        if self.name.is_empty() {
            None
        } else {
            CStr::from_bytes_until_nul(self.name).ok()
        }
    }
}

/// Iterator over an [`inotify_add_watch`]-driven read buffer, yielding
/// [`InotifyEvent`]. Constructed by [`inotify_events`].
pub struct InotifyEvents<'a> {
    buf: &'a [u8],
}

/// Parse the `n` bytes a [`crate::fd::read`] of `fd` (an [`inotify_init1`]
/// descriptor) wrote into `buf[..n]` as a sequence of [`InotifyEvent`]s.
/// Pass exactly the filled prefix — the kernel always writes whole records,
/// never a partial trailing one, so a self-consistent buffer parses
/// correctly regardless of how many events it holds.
///
/// # Panics
///
/// Panics on a buffer that isn't a genuine inotify read (truncated
/// mid-record, or a corrupt length field) — a programmer error, not a
/// runtime condition: `buf` is kernel-produced, never external/untrusted
/// data, so there is nothing to recover from gracefully here.
pub fn inotify_events(buf: &[u8]) -> InotifyEvents<'_> {
    InotifyEvents { buf }
}

impl<'a> Iterator for InotifyEvents<'a> {
    type Item = InotifyEvent<'a>;

    fn next(&mut self) -> Option<InotifyEvent<'a>> {
        if self.buf.is_empty() {
            return None;
        }
        // Kernel `struct inotify_event` layout: wd (i32) @0, mask (u32) @4,
        // cookie (u32) @8, len (u32) @12, then a NUL-padded name[len].
        const HEADER: usize = 16;
        let rec = self.buf;
        let wd = i32::from_ne_bytes(rec[0..4].try_into().unwrap());
        let mask = u32::from_ne_bytes(rec[4..8].try_into().unwrap());
        let cookie = u32::from_ne_bytes(rec[8..12].try_into().unwrap());
        let len = u32::from_ne_bytes(rec[12..16].try_into().unwrap()) as usize;
        let name = &rec[HEADER..HEADER + len];
        self.buf = &rec[HEADER + len..];
        Some(InotifyEvent {
            wd,
            mask,
            cookie,
            name,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{fd, fs};

    fn temp_path(tag: &str) -> std::ffi::CString {
        let p = format!(
            "{}/rusty_libc_inotify_{}_{}",
            std::env::temp_dir().display(),
            tag,
            std::process::id()
        );
        std::ffi::CString::new(p).unwrap()
    }

    #[test]
    fn create_and_close_in_a_watched_directory_are_reported() {
        let dir = temp_path("dir");
        let _ = fs::rmdir(&dir);
        fs::mkdir(&dir, 0o700).expect("mkdir");

        let ifd = inotify_init1(IN_CLOEXEC).expect("inotify_init1");
        let wd =
            inotify_add_watch(ifd, &dir, IN_CREATE | IN_CLOSE_WRITE).expect("inotify_add_watch");

        let file_path_str = format!("{}/leaf", dir.to_str().unwrap());
        let file_path = std::ffi::CString::new(file_path_str).unwrap();
        let f = fd::open(&file_path, fd::O_WRONLY | fd::O_CREAT, 0o600).expect("open");
        fd::close(f).expect("close");

        let mut buf = [0u8; 4096];
        let n = fd::read(ifd, &mut buf).expect("read");
        let events: std::vec::Vec<_> = inotify_events(&buf[..n]).collect();

        assert!(events.iter().any(|e| e.wd == wd
            && e.mask & IN_CREATE != 0
            && e.name().map(|n| n.to_bytes()) == Some(b"leaf".as_slice())));
        assert!(events
            .iter()
            .any(|e| e.wd == wd && e.mask & IN_CLOSE_WRITE != 0));

        fs::unlink(&file_path).ok();
        inotify_rm_watch(ifd, wd).expect("inotify_rm_watch");
        fd::close(ifd).expect("close");
        fs::rmdir(&dir).expect("rmdir");
    }

    #[test]
    fn delete_self_is_reported_with_no_name() {
        let dir = temp_path("delself");
        let _ = fs::rmdir(&dir);
        fs::mkdir(&dir, 0o700).expect("mkdir");

        let ifd = inotify_init1(0).expect("inotify_init1");
        let wd = inotify_add_watch(ifd, &dir, IN_DELETE_SELF).expect("inotify_add_watch");

        fs::rmdir(&dir).expect("rmdir");

        let mut buf = [0u8; 4096];
        let n = fd::read(ifd, &mut buf).expect("read");
        let events: std::vec::Vec<_> = inotify_events(&buf[..n]).collect();

        assert!(events
            .iter()
            .any(|e| e.wd == wd && e.mask & IN_DELETE_SELF != 0 && e.name().is_none()));

        fd::close(ifd).expect("close");
    }

    #[test]
    fn nonblocking_read_with_no_events_is_eagain() {
        let dir = temp_path("nb");
        let _ = fs::rmdir(&dir);
        fs::mkdir(&dir, 0o700).expect("mkdir");

        let ifd = inotify_init1(IN_NONBLOCK).expect("inotify_init1");
        let _wd = inotify_add_watch(ifd, &dir, IN_CREATE).expect("inotify_add_watch");

        let mut buf = [0u8; 64];
        assert_eq!(fd::read(ifd, &mut buf), Err(Errno::EAGAIN));

        fd::close(ifd).expect("close");
        fs::rmdir(&dir).expect("rmdir");
    }

    #[test]
    fn add_watch_on_a_missing_path_is_enoent() {
        let ifd = inotify_init1(0).expect("inotify_init1");
        assert_eq!(
            inotify_add_watch(ifd, c"/no/such/rusty_libc/path", IN_CREATE),
            Err(Errno::ENOENT)
        );
        fd::close(ifd).expect("close");
    }

    #[test]
    fn rm_watch_with_a_stale_descriptor_is_einval() {
        let ifd = inotify_init1(0).expect("inotify_init1");
        assert_eq!(inotify_rm_watch(ifd, 12345), Err(Errno::EINVAL));
        fd::close(ifd).expect("close");
    }
}
