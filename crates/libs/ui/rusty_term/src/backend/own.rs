//! Our own console host as a backend: `[console] host = "own"` (design
//! `docs/projects/term/conhost/` §4.5).
//!
//! rusty_conhost serves the shell's console in place of ConPTY. Only the
//! child's side differs: the host console's raw mode and size are the
//! ConPTY backend's. If the host can't start, the error is shown; there is
//! no silent fallback to ConPTY.
//!
//! This module is only compiled on Windows (gated in `backend/mod.rs`).

use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use platform::error::{ErrorKind, PlatformError};
use platform::process::{Child as _, Command, ExitStatus, Signal};
use platform::pty::PtyMaster as _;
use platform::term::WinSize;
use platform_windows::WindowsChild;
use rusty_conhost::{OwnPty, OwnPtyMaster};

use crate::backend::cmdline::split_command_line;
use crate::backend::windows::{WindowsBackend, shell_or_default};
use crate::backend::{Backend, BackendHandle};

/// How long the child gets to exit after a hang-up, or to report its exit
/// status, before it is killed or given up on. The ConPTY handle's bound,
/// and the console's own close grace.
const GRACE: Duration = Duration::from_secs(2);

/// How often [`exited_within`] looks at the child.
const POLL: Duration = Duration::from_millis(10);

/// Unit type implementing [`Backend`] on our own console host.
pub struct OwnBackend;

impl Backend for OwnBackend {
    fn spawn_shell(
        &self,
        cols: u16,
        rows: u16,
        shell: Option<&str>,
        args: &[String],
        cwd: Option<&Path>,
    ) -> Result<Box<dyn BackendHandle>, io::Error> {
        let shell = shell_or_default(shell);
        let mut words = split_command_line(&shell, |p| locate(p).is_some());
        if words.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("rusty_term: no program in the shell `{shell}`"),
            ));
        }
        let word = words.remove(0);
        // `Command`'s own search is PATH only; what CreateProcessW finds
        // before PATH is passed as a full path.
        let program = locate(&word).map_or_else(|| word.into(), PathBuf::into_os_string);
        let cwd = match cwd {
            Some(dir) => dir.to_path_buf(),
            None => std::env::current_dir()?,
        };
        let cmd = Command::new(program, cwd.into_os_string())
            .args(words.into_iter().chain(args.iter().cloned()));
        let (master, child) = OwnPty
            .spawn_concrete(&cmd, WinSize { rows, cols })
            .map_err(to_io)?;
        Ok(Box::new(OwnHandle {
            master: Arc::new(master),
            child: Some(child),
        }))
    }

    fn set_raw_mode(&self, enabled: bool) -> Result<(), io::Error> {
        WindowsBackend.set_raw_mode(enabled)
    }

    fn terminal_size(&self) -> Option<(u16, u16)> {
        WindowsBackend.terminal_size()
    }
}

/// The terminal's side of a console we host. The handle `spawn_shell`
/// returns also owns the child; clones share the console only.
struct OwnHandle {
    master: Arc<OwnPtyMaster>,
    child: Option<WindowsChild>,
}

impl BackendHandle for OwnHandle {
    fn read_into(&mut self, buf: &mut [u8]) -> Result<usize, io::Error> {
        self.master.read(buf).map_err(to_io)
    }

    fn output_ready(&self) -> bool {
        self.master.output_ready()
    }

    fn write(&mut self, data: &[u8]) -> Result<(), io::Error> {
        let mut rest = data;
        while !rest.is_empty() {
            let n = self.master.write(rest).map_err(to_io)?;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "rusty_conhost took no input",
                ));
            }
            rest = &rest[n..];
        }
        Ok(())
    }

    fn try_clone(&self) -> Result<Box<dyn BackendHandle>, io::Error> {
        Ok(Box::new(OwnHandle {
            master: Arc::clone(&self.master),
            child: None,
        }))
    }

    fn set_winsize(&mut self, cols: u16, rows: u16) -> Result<(), io::Error> {
        self.master.resize(WinSize { rows, cols }).map_err(to_io)
    }

    fn exit_token(&self) -> Option<Box<dyn FnOnce() + Send>> {
        self.child.as_ref()?;
        let waiter = self.master.exit_waiter();
        Some(Box::new(move || waiter.wait()))
    }

    fn reap_exit_status(&mut self) -> Option<i32> {
        // The exit token also fires when the console is torn down, which
        // can come just before the child's exit; wait out the grace.
        match exited_within(self.child.as_mut()?, GRACE) {
            Some(ExitStatus::Code(code)) => Some(code),
            _ => None,
        }
    }
}

impl Drop for OwnHandle {
    /// The original hangs up, gives the child the grace to exit, then kills
    /// it, as the ConPTY handle does. A clone only drops its reference; the
    /// last one closes the console (`OwnPtyMaster`'s Drop).
    fn drop(&mut self) {
        let Some(child) = self.child.as_mut() else {
            return;
        };
        self.master.hang_up();
        if exited_within(child, GRACE).is_none() {
            // Drop can't report a failure; a child that outlives this is
            // already detached from a console that has closed.
            let _ = child.kill_single(Signal::Kill);
        }
    }
}

/// The child's exit status once it exits, or `None` if it is still running
/// after `grace` or its status can't be read. `WindowsChild` has no timed
/// wait, so this polls.
fn exited_within(child: &mut WindowsChild, grace: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + grace;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(POLL),
            Ok(None) | Err(_) => return None,
        }
    }
}

/// Where `CreateProcessW` finds `program` before it searches PATH, as a
/// full path. A name with a directory part is looked up from our current
/// directory; a bare name in rusty_term's own directory, the current
/// directory, System32, then the Windows directory. Each place is tried as
/// given, then with `.exe` added when the name has no extension. `None`
/// leaves the name to `Command`'s PATH search.
fn locate(program: &str) -> Option<PathBuf> {
    let path = Path::new(program);
    let bare = !program.contains(['\\', '/', ':']);
    let dirs = if bare {
        search_dirs()
    } else {
        vec![PathBuf::new()]
    };
    dirs.iter()
        .flat_map(|dir| with_exe(dir.join(path)))
        .find(|candidate| candidate.is_file())
        .and_then(|found| std::path::absolute(found).ok())
}

/// `path`, then `path.exe` when it has no extension.
fn with_exe(path: PathBuf) -> impl Iterator<Item = PathBuf> {
    let exe = path.extension().is_none().then(|| {
        let mut name = path.clone().into_os_string();
        name.push(OsStr::new(".exe"));
        PathBuf::from(name)
    });
    std::iter::once(path).chain(exe)
}

/// The places `CreateProcessW` searches for a bare name before PATH.
fn search_dirs() -> Vec<PathBuf> {
    let own = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    let windows = std::env::var_os("SystemRoot").map(PathBuf::from);
    own.into_iter()
        .chain(std::env::current_dir().ok())
        .chain(windows.as_ref().map(|dir| dir.join("System32")))
        .chain(windows)
        .collect()
}

/// A platform error as an `io::Error`, keeping its message and the kinds
/// the runtime tells apart.
fn to_io(e: PlatformError) -> io::Error {
    let kind = match e.kind {
        ErrorKind::NotFound => io::ErrorKind::NotFound,
        ErrorKind::PermissionDenied => io::ErrorKind::PermissionDenied,
        ErrorKind::InvalidInput => io::ErrorKind::InvalidInput,
        ErrorKind::BrokenPipe => io::ErrorKind::BrokenPipe,
        ErrorKind::Unsupported => io::ErrorKind::Unsupported,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, e)
}

#[cfg(test)]
mod tests {
    use super::OwnBackend;
    use crate::backend::{Backend as _, BackendHandle};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// Reads the handle on a detached thread into a shared buffer, so a
    /// hung read can't wedge the test harness.
    fn drain(h: &dyn BackendHandle) -> Arc<Mutex<Vec<u8>>> {
        let out = Arc::new(Mutex::new(Vec::new()));
        let mut rh = h.try_clone().expect("clone read handle");
        let sink = Arc::clone(&out);
        std::thread::spawn(move || {
            while let Ok(b) = rh.read() {
                if b.is_empty() {
                    break;
                }
                sink.lock().unwrap().extend_from_slice(&b);
            }
        });
        out
    }

    fn wait_for(out: &Mutex<Vec<u8>>, needle: &[u8], why: &str) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if out
                .lock()
                .unwrap()
                .windows(needle.len())
                .any(|w| w == needle)
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let buf = out.lock().unwrap().clone();
        panic!(
            "timed out waiting for {why}: {:?}",
            String::from_utf8_lossy(&buf)
        );
    }

    /// A child on our own host spawns, its output reaches the reader, its
    /// exit fires the exit token and its exit code is reported.
    #[test]
    fn own_host_child_output_exit_and_status_reach_the_terminal() {
        let mut h = OwnBackend
            .spawn_shell(80, 24, Some("cmd.exe /c echo own_ok& exit 7"), &[], None)
            .expect("spawn on our own host");
        let out = drain(h.as_ref());
        wait_for(&out, b"own_ok", "the child's output");
        let exited = h.exit_token().expect("the original has an exit token");
        exited();
        assert_eq!(h.reap_exit_status(), Some(7));
    }

    /// Input typed into the console reaches an interactive cmd.exe, and a
    /// resize is accepted while it runs.
    #[test]
    fn own_host_input_round_trips_through_cmd() {
        let mut h = OwnBackend
            .spawn_shell(80, 24, Some("cmd.exe"), &[], None)
            .expect("spawn cmd.exe on our own host");
        let out = drain(h.as_ref());
        h.set_winsize(100, 30).expect("resize");
        h.write(b"echo typed_ok\r").expect("write");
        wait_for(&out, b"typed_ok", "the typed command's echo");
        h.write(b"exit 3\r").expect("write exit");
        h.exit_token().expect("exit token")();
        assert_eq!(h.reap_exit_status(), Some(3));
    }

    /// Dropping the original hangs up: a child still running gets
    /// CTRL_CLOSE, or is killed after the grace, and Drop returns soon after.
    #[test]
    fn dropping_the_original_ends_a_running_child() {
        let h = OwnBackend
            .spawn_shell(80, 24, Some("ping.exe -n 60 127.0.0.1"), &[], None)
            .expect("spawn");
        let exited = h.exit_token().expect("exit token");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            exited();
            let _ = tx.send(());
        });
        let start = Instant::now();
        drop(h);
        assert!(start.elapsed() < Duration::from_secs(5), "Drop is bounded");
        rx.recv_timeout(Duration::from_secs(5))
            .expect("the child ended after the hang-up");
    }

    /// A program named by a full path with spaces, followed by arguments,
    /// is found as CreateProcessW finds it.
    #[test]
    fn an_unquoted_path_with_spaces_runs() {
        let windir = std::env::var("SystemRoot").expect("SystemRoot");
        let dir = std::env::temp_dir().join("rusty term own host");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let cmd = dir.join("cmd copy.exe");
        std::fs::copy(format!(r"{windir}\System32\cmd.exe"), &cmd).expect("copy cmd");
        let shell = format!("{} /c exit 4", cmd.display());
        let mut h = OwnBackend
            .spawn_shell(80, 24, Some(&shell), &[], None)
            .expect("spawn by an unquoted path with spaces");
        h.exit_token().expect("exit token")();
        assert_eq!(h.reap_exit_status(), Some(4));
        drop(h);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn locate_finds_system_programs_and_adds_exe() {
        let found = super::locate("cmd").expect("cmd is in System32");
        assert!(found.is_absolute());
        assert!(
            found
                .to_string_lossy()
                .to_ascii_lowercase()
                .ends_with(r"\cmd.exe")
        );
        assert!(super::locate("no-such-program-rusty").is_none());
    }

    /// A clone owns no child: no exit token, no status.
    #[test]
    fn a_clone_owns_no_child() {
        let h = OwnBackend
            .spawn_shell(80, 24, Some("cmd.exe /c exit 0"), &[], None)
            .expect("spawn");
        let mut clone = h.try_clone().expect("clone");
        assert!(clone.exit_token().is_none());
        assert_eq!(clone.reap_exit_status(), None);
    }
}
