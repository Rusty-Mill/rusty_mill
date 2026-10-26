pub trait Backend {
    /// Spawn the child shell on a new PTY sized to `cols`×`rows`. `shell`
    /// overrides the platform default (`$SHELL` / `%COMSPEC%`) when `Some` —
    /// the `shell` config key. `args` is explicit argv appended after `shell`
    /// (the launcher's pre-split `ExecutablePath` + `Arguments`, e.g. a
    /// trailing `-- prog arg...`); empty when the caller has none, in which
    /// case a `shell` string carrying its own args (`"bash --login -i"`) is
    /// still split and honored. `cwd` sets the child's initial working
    /// directory, defaulting to this process's cwd when `None`.
    fn spawn_shell(
        &self,
        cols: u16,
        rows: u16,
        shell: Option<&str>,
        args: &[String],
        cwd: Option<&std::path::Path>,
    ) -> Result<Box<dyn BackendHandle>, std::io::Error>;
    fn set_raw_mode(&self, enabled: bool) -> Result<(), std::io::Error>;
    /// Best-effort query of the controlling terminal's size as `(cols, rows)`.
    fn terminal_size(&self) -> Option<(u16, u16)>;
}

pub trait BackendHandle: Send {
    /// Read whatever bytes are currently available from the child (blocking).
    ///
    /// An `Ok` value containing an empty slice signals end-of-file: the child
    /// has exited and its side of the PTY is closed. Used by the windowed (gui)
    /// reader thread and by the Windows runtime's bridge thread; the Unix runtime
    /// drives the raw [`pty_fd`](Self::pty_fd) through its reactor instead, so the
    /// method is unused on a Unix terminal-only build.
    #[cfg_attr(unix, allow(dead_code))]
    fn read(&mut self) -> Result<Vec<u8>, std::io::Error> {
        let mut buf = vec![0u8; 4096];
        let n = self.read_into(&mut buf)?;
        buf.truncate(n);
        Ok(buf)
    }

    /// [`read`](Self::read) into `buf`, returning the byte count; `Ok(0)` is
    /// end-of-file. Blocks until at least one byte (or EOF) is available.
    #[cfg_attr(unix, allow(dead_code))]
    fn read_into(&mut self, buf: &mut [u8]) -> Result<usize, std::io::Error>;

    /// Whether a [`read_into`](Self::read_into) would return without
    /// blocking. `false` when the platform can't tell cheaply, which only
    /// costs batching (see [`read_batch`]).
    #[cfg_attr(unix, allow(dead_code))]
    fn output_ready(&self) -> bool {
        false
    }

    /// Write `data` to the child's input (blocking). Used by the windowed (gui)
    /// backend and the Windows runtime's bridge thread; the Unix runtime writes
    /// the raw [`pty_fd`](Self::pty_fd) through its reactor instead.
    #[cfg_attr(unix, allow(dead_code))]
    fn write(&mut self, data: &[u8]) -> Result<(), std::io::Error>;

    /// Produce an independent handle referring to the same child, so the
    /// read side and write side can live on separate threads without sharing
    /// a lock. The clone does not own the child (only the original reaps it).
    ///
    /// Descriptors are released via the handle's `Drop` impl.
    fn try_clone(&self) -> Result<Box<dyn BackendHandle>, std::io::Error>;

    /// Inform the child of a new window size (`cols`×`rows`), so it can reflow
    /// and so applications receive `SIGWINCH`.
    fn set_winsize(&mut self, cols: u16, rows: u16) -> Result<(), std::io::Error>;

    /// A blocking closure that returns once the child has exited, for front-ends
    /// that can't rely on read-EOF to detect it. `None` when read-EOF already
    /// signals exit (the Unix PTY) or this handle doesn't own the child; `Some`
    /// only on the owning Windows ConPTY handle (whose output pipe EOFs at
    /// teardown, not on child exit — without this, teardown would deadlock
    /// waiting for an EOF that only teardown itself produces). The windowed
    /// backend runs it on a watcher thread to close the window when the shell
    /// quits; the Windows console runtime uses it to stop its event loop.
    #[cfg_attr(unix, allow(dead_code))]
    fn exit_token(&self) -> Option<Box<dyn FnOnce() + Send>> {
        None
    }

    /// The PTY master descriptor backing this handle, for the Unix runtime to
    /// drive through a readiness reactor (`rusty_tokio::io::AsyncFd`). The fd stays owned
    /// by the handle: the runtime registers its own duplicate, never this number, so
    /// the handle may close it at any time. Unix-only; the
    /// Windows ConPTY handle has no equivalent pollable fd (its runtime bridges
    /// the blocking [`read`](Self::read)/[`write`](Self::write) instead).
    #[cfg(unix)]
    fn pty_fd(&self) -> std::os::unix::io::RawFd;

    /// Reap the owned child and return its exit status as a
    /// `std::process::exit`-compatible value: the child's own exit code on a
    /// normal exit, or 128+signal (the sh/bash convention) on a signal death.
    /// Called once the caller already knows the child has exited (read-EOF,
    /// a SIGCHLD, or — Windows — the [`exit_token`](Self::exit_token) watcher
    /// firing), so this should return promptly. `None` on a handle that
    /// doesn't own the child (a clone), or if it races another reaper for
    /// the same child and loses (harmless — whichever side wins reports the
    /// status).
    ///
    /// Without this, the exit code was always 0 regardless of what the
    /// child actually did — a failed `execvp` or a shell that ran a failing
    /// command as its last action looked identical to success to anything
    /// scripting `rusty_term -e ...`.
    fn reap_exit_status(&mut self) -> Option<i32> {
        None
    }

    /// The owned child's pid, for the Unix runtime's SIGCHLD watcher (see
    /// [`crate::runtime`]): a background process that inherited the pty as
    /// its own stdout/stderr (`nohup cmd &` then exiting the shell) never
    /// produces read-EOF on the master, since the fd stays open via the
    /// orphan — reaping proactively on SIGCHLD instead of only on EOF avoids
    /// wedging shutdown forever in that case. `None` on a handle that
    /// doesn't own the child (a clone) or on a platform where this isn't
    /// meaningful.
    #[cfg(unix)]
    fn child_pid(&self) -> Option<libc::pid_t> {
        None
    }
}

/// Fill `buf` with child output: block for the first read, then keep
/// reading while more is ready without blocking, until `buf` is full. One
/// batch is parsed, locked and redrawn once instead of once per read (a PTY
/// master returns at most about 4 KiB a read on Linux). `Ok(0)` is
/// end-of-file. An EOF or error after some bytes ends the batch with those
/// bytes; the next call meets it again (both persist on a closed PTY).
#[cfg_attr(not(feature = "gui"), allow(dead_code))]
pub fn read_batch(handle: &mut dyn BackendHandle, buf: &mut [u8]) -> Result<usize, std::io::Error> {
    let mut n = handle.read_into(buf)?;
    while n > 0 && n < buf.len() && handle.output_ready() {
        match handle.read_into(&mut buf[n..]) {
            Ok(0) | Err(_) => break,
            Ok(more) => n += more,
        }
    }
    Ok(n)
}

// Each backend only compiles on its own platform — the Unix one leans on
// platform-specific libc (openpty, termios, …), the Windows one on ConPTY.
#[cfg(unix)]
pub mod unix;
#[cfg(windows)]
pub mod windows;

#[cfg(unix)]
pub use unix::UnixBackend;
#[cfg(windows)]
pub use windows::WindowsBackend;

#[cfg(test)]
mod tests {
    use super::{BackendHandle, read_batch};
    use std::collections::VecDeque;
    use std::io::Error;

    /// Reads hand out `chunks` in order, then EOF; `ready` while any remain.
    /// An error stays put, as a closed PTY's does.
    struct Chunks(VecDeque<Result<Vec<u8>, Error>>);

    impl BackendHandle for Chunks {
        fn read_into(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
            if let Some(Err(e)) = self.0.front() {
                return Err(Error::new(e.kind(), e.to_string()));
            }
            let Some(Ok(mut chunk)) = self.0.pop_front() else {
                return Ok(0);
            };
            let n = chunk.len().min(buf.len());
            buf[..n].copy_from_slice(&chunk[..n]);
            if n < chunk.len() {
                self.0.push_front(Ok(chunk.split_off(n)));
            }
            Ok(n)
        }
        fn output_ready(&self) -> bool {
            !self.0.is_empty()
        }
        fn write(&mut self, _data: &[u8]) -> Result<(), Error> {
            Ok(())
        }
        fn try_clone(&self) -> Result<Box<dyn BackendHandle>, Error> {
            Err(Error::other("not cloneable"))
        }
        fn set_winsize(&mut self, _cols: u16, _rows: u16) -> Result<(), Error> {
            Ok(())
        }
        #[cfg(unix)]
        fn pty_fd(&self) -> std::os::unix::io::RawFd {
            -1
        }
    }

    fn chunks(parts: Vec<Result<&[u8], Error>>) -> Chunks {
        Chunks(parts.into_iter().map(|p| p.map(<[u8]>::to_vec)).collect())
    }

    #[test]
    fn ready_reads_join_one_batch() {
        let mut h = chunks(vec![Ok(b"ab"), Ok(b"cd"), Ok(b"e")]);
        let mut buf = [0u8; 16];
        assert_eq!(read_batch(&mut h, &mut buf).unwrap(), 5);
        assert_eq!(&buf[..5], b"abcde");
        assert_eq!(read_batch(&mut h, &mut buf).unwrap(), 0, "then EOF");
    }

    #[test]
    fn a_batch_stops_when_the_buffer_is_full() {
        let mut h = chunks(vec![Ok(b"abc"), Ok(b"defg")]);
        let mut buf = [0u8; 5];
        assert_eq!(read_batch(&mut h, &mut buf).unwrap(), 5);
        assert_eq!(&buf, b"abcde");
        assert_eq!(read_batch(&mut h, &mut buf).unwrap(), 2);
        assert_eq!(&buf[..2], b"fg");
    }

    #[test]
    fn an_error_after_bytes_waits_for_the_next_batch() {
        let mut h = chunks(vec![Ok(b"ab"), Err(Error::other("gone"))]);
        let mut buf = [0u8; 16];
        assert_eq!(read_batch(&mut h, &mut buf).unwrap(), 2);
        assert!(read_batch(&mut h, &mut buf).is_err());
    }

    #[test]
    fn a_first_read_error_is_returned() {
        let mut h = chunks(vec![Err(Error::other("gone"))]);
        assert!(read_batch(&mut h, &mut [0u8; 16]).is_err());
    }
}
