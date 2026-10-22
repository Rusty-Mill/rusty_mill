//! What `bbp mod` waits on between passes. A pass runs as soon as something
//! it reacts to happens: the task's log is appended to (a harness's `bbp mcp`,
//! the runner or a human command wrote an event), or a child the moderator
//! started (a harness or the runner) exits. The configured poll is only the
//! longest wait, so deadlines, the wall limit, the end grace and the cost cap
//! are still checked at least that often when nothing happens.
//!
//! Linux watches the log with inotify and the children with pidfds. Elsewhere,
//! or if the watch cannot be set up, the waker sleeps for the whole timeout,
//! which is the loop's previous behavior.

use std::path::Path;
use std::time::Duration;

/// Why [`Waker::wait`] returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wake {
    /// The task's log changed or a watched child exited.
    Event,
    /// The timeout passed with neither.
    Timeout,
}

/// Waits for the task's log to change or a child to exit.
pub struct Waker {
    #[cfg(target_os = "linux")]
    inotify: Option<linux::Inotify>,
}

impl Waker {
    /// Watch `log`, the task's log file, which must exist. A watch that cannot
    /// be set up is reported and the waker falls back to sleeping.
    pub fn new(log: &Path) -> Waker {
        #[cfg(target_os = "linux")]
        {
            let inotify = linux::Inotify::watch(log)
                .map_err(|e| {
                    eprintln!(
                        "bbp mod: cannot watch {} ({e}); polling instead",
                        log.display()
                    )
                })
                .ok();
            Waker { inotify }
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = log;
            Waker {}
        }
    }

    /// Block until the log changes, one of `pids` exits, or `timeout` passes.
    /// An exited child that is not yet reaped wakes at once.
    pub fn wait(&mut self, pids: &[u32], timeout: Duration) -> Result<Wake, String> {
        #[cfg(target_os = "linux")]
        if let Some(inotify) = &self.inotify {
            return linux::wait(inotify, pids, timeout);
        }
        let _ = pids;
        std::thread::sleep(timeout);
        Ok(Wake::Timeout)
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::Wake;
    use rusty_libc::fd::{close, poll, read, PollFd, POLLIN};
    use rusty_libc::inotify::{
        inotify_add_watch, inotify_init1, IN_CLOEXEC, IN_MODIFY, IN_NONBLOCK,
    };
    use rusty_libc::process::pidfd_open;
    use rusty_libc::Errno;
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;
    use std::time::Duration;

    /// An inotify descriptor watching one file for writes; closed on drop.
    pub struct Inotify(i32);

    impl Inotify {
        pub fn watch(path: &Path) -> Result<Inotify, String> {
            let c = CString::new(path.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
            let fd = inotify_init1(IN_NONBLOCK | IN_CLOEXEC)
                .map_err(|e| format!("inotify_init1: {e:?}"))?;
            let me = Inotify(fd);
            inotify_add_watch(fd, &c, IN_MODIFY)
                .map_err(|e| format!("inotify_add_watch: {e:?}"))?;
            Ok(me)
        }

        /// Read every queued event, so the next wait blocks until a new one.
        fn drain(&self) -> Result<(), String> {
            let mut buf = [0u8; 4096];
            loop {
                match read(self.0, &mut buf) {
                    Ok(0) | Err(Errno::EAGAIN) => return Ok(()),
                    Ok(_) | Err(Errno::EINTR) => continue,
                    Err(e) => return Err(format!("inotify read: {e:?}")),
                }
            }
        }
    }

    impl Drop for Inotify {
        fn drop(&mut self) {
            if let Err(e) = close(self.0) {
                eprintln!("bbp mod: inotify close: {e:?}");
            }
        }
    }

    /// Closes the pidfds opened for one wait.
    struct Pidfds(Vec<i32>);

    impl Drop for Pidfds {
        fn drop(&mut self) {
            for fd in self.0.drain(..) {
                if let Err(e) = close(fd) {
                    eprintln!("bbp mod: pidfd close: {e:?}");
                }
            }
        }
    }

    pub fn wait(inotify: &Inotify, pids: &[u32], timeout: Duration) -> Result<Wake, String> {
        // The children are unreaped, so their pids cannot be reused under us.
        let mut pidfds = Pidfds(Vec::with_capacity(pids.len()));
        for &pid in pids {
            let pid = i32::try_from(pid).map_err(|_| "pid out of range".to_owned())?;
            match pidfd_open(pid, 0) {
                Ok(fd) => pidfds.0.push(fd),
                // Already reaped: nothing to wait for.
                Err(Errno::ESRCH) => {}
                Err(e) => return Err(format!("pidfd_open({pid}): {e:?}")),
            }
        }
        let mut fds: Vec<PollFd> = std::iter::once(inotify.0)
            .chain(pidfds.0.iter().copied())
            .map(|fd| PollFd {
                fd,
                events: POLLIN,
                revents: 0,
            })
            .collect();
        let ms = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
        let ready = match poll(&mut fds, ms) {
            Ok(n) => n,
            // A signal cut the wait short: let the loop run a pass.
            Err(Errno::EINTR) => return Ok(Wake::Event),
            Err(e) => return Err(format!("poll: {e:?}")),
        };
        inotify.drain()?;
        Ok(if ready == 0 {
            Wake::Timeout
        } else {
            Wake::Event
        })
    }
}

#[cfg(all(test, target_os = "linux"))]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::Instant;

    fn log_file(name: &str) -> (std::path::PathBuf, std::fs::File) {
        let dir = std::env::temp_dir().join(format!("bbp-waker-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("task.log");
        let file = std::fs::File::create(&path).expect("log file");
        (path, file)
    }

    #[test]
    fn an_append_wakes_the_wait_well_before_its_timeout() {
        let (path, mut file) = log_file("append");
        let mut waker = Waker::new(&path);
        let writer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            file.write_all(b"[]\n").expect("append");
        });
        let started = Instant::now();
        let woke = waker.wait(&[], Duration::from_secs(10)).expect("wait");
        writer.join().expect("writer");
        assert_eq!(woke, Wake::Event);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn nothing_happening_waits_out_the_timeout() {
        let (path, _file) = log_file("quiet");
        let mut waker = Waker::new(&path);
        let started = Instant::now();
        let woke = waker.wait(&[], Duration::from_millis(100)).expect("wait");
        assert_eq!(woke, Wake::Timeout);
        assert!(started.elapsed() >= Duration::from_millis(100));
    }

    #[test]
    fn events_already_seen_do_not_wake_the_next_wait() {
        let (path, mut file) = log_file("drained");
        let mut waker = Waker::new(&path);
        file.write_all(b"[]\n").expect("append");
        assert_eq!(
            waker.wait(&[], Duration::from_secs(10)).expect("wait"),
            Wake::Event
        );
        let woke = waker.wait(&[], Duration::from_millis(100)).expect("wait");
        assert_eq!(woke, Wake::Timeout);
    }

    #[test]
    fn a_child_exit_wakes_the_wait() {
        let (path, _file) = log_file("child");
        let mut waker = Waker::new(&path);
        let mut child = std::process::Command::new("sh")
            .args(["-c", "sleep 0.05"])
            .spawn()
            .expect("spawn");
        let started = Instant::now();
        let woke = waker
            .wait(&[child.id()], Duration::from_secs(10))
            .expect("wait");
        assert_eq!(woke, Wake::Event);
        assert!(started.elapsed() < Duration::from_secs(5));
        child.wait().expect("reap");
    }

    #[test]
    fn a_missing_log_falls_back_to_sleeping() {
        let path = std::env::temp_dir()
            .join("bbp-waker-missing")
            .join("none.log");
        let mut waker = Waker::new(&path);
        let woke = waker.wait(&[], Duration::from_millis(50)).expect("wait");
        assert_eq!(woke, Wake::Timeout);
    }
}
