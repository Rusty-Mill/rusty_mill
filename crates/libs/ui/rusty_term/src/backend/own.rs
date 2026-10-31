//! Our own console host as a backend: `[console] host = "own"` (design
//! `docs/projects/term/conhost/` §4.5).
//!
//! rusty_conhost serves the shell's console in place of ConPTY. Only the
//! child's side differs: the host console's raw mode and size are the
//! ConPTY backend's. If the host can't start, the error is shown; there is
//! no silent fallback to ConPTY.
//!
//! This module is only compiled on Windows (gated in `backend/mod.rs`).

use std::ffi::OsString;
use std::io;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use platform::error::{ErrorKind, PlatformError};
use platform::process::{Child as _, Command, ExitStatus, Signal};
use platform::pty::PtyMaster as _;
use platform::term::WinSize;
use platform_windows::WindowsChild;
use rusty_conhost::{OwnPty, OwnPtyMaster};

use crate::backend::cmdline::{Reader, locate, reader, split_program};
use crate::backend::reaper::Reaper;
use crate::backend::windows::{WindowsBackend, push_launcher_args, shell_or_default};
use crate::backend::{Backend, BackendHandle};

/// How long the child gets to exit after a hang-up, or to report its exit
/// status, before it is killed or given up on. The ConPTY handle's bound,
/// and the console's own close grace.
const GRACE: Duration = Duration::from_secs(2);

/// How long Drop waits for the console to shut down: it closes 2 s after
/// the hang-up at the latest, and ends the clients still attached 5 s
/// after it (#1259); 1 s more covers its teardown.
const SHUTDOWN: Duration = Duration::from_secs(6);

/// How long a child the host ended gets to exit before Drop kills it.
const SETTLE: Duration = Duration::from_millis(250);

/// How often [`exited_within`] looks at the child.
const POLL: Duration = Duration::from_millis(10);

/// [`Backend`] on our own console host. Dropping it waits for the
/// consoles its handles hung up to shut down (#1259), so drop it last,
/// before the process exits.
pub struct OwnBackend {
    reaper: Arc<Reaper>,
}

impl OwnBackend {
    pub fn new() -> Self {
        Self {
            reaper: Arc::new(Reaper::new()),
        }
    }
}

impl Default for OwnBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for OwnBackend {
    fn drop(&mut self) {
        self.reaper.join();
    }
}

impl Backend for OwnBackend {
    fn spawn_shell(
        &self,
        cols: u16,
        rows: u16,
        shell: Option<&str>,
        args: &[String],
        cwd: Option<&Path>,
    ) -> Result<Box<dyn BackendHandle>, io::Error> {
        let cwd = match cwd {
            Some(dir) => dir.to_path_buf(),
            None => std::env::current_dir()?,
        };
        let cmd = shell_command(&shell_or_default(shell), args, cwd.into_os_string())?;
        let (master, child) = OwnPty
            .spawn_concrete(&cmd, WinSize { rows, cols })
            .map_err(to_io)?;
        Ok(Box::new(OwnHandle {
            master: Arc::new(master),
            child: Some(child),
            reaper: Arc::clone(&self.reaper),
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
    /// Where the original's Drop leaves the console's shutdown.
    reaper: Arc<Reaper>,
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
            reaper: Arc::clone(&self.reaper),
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
    /// The original hangs up, and the backend's reaper waits for the
    /// console to shut down, which ends every client still attached 5 s
    /// after the hang-up, the child included. Drop returns at once, so
    /// closing a pane never stalls the caller; the backend's own Drop
    /// joins the wait before the process exits. Only a host that fails to
    /// shut down in time leaves the child to be killed. A clone only drops
    /// its reference; the last one closes the console (`OwnPtyMaster`'s
    /// Drop).
    fn drop(&mut self) {
        let Some(child) = self.child.take() else {
            return;
        };
        self.master.hang_up();
        let master = Arc::clone(&self.master);
        self.reaper.run(Box::new(move || shut_down(&master, child)));
    }
}

/// Waits for the hung-up console to shut down, then ends `child` if it
/// is still running.
fn shut_down(master: &OwnPtyMaster, mut child: WindowsChild) {
    let done = master.wait_shut_down(SHUTDOWN);
    end_survivor(&mut child, done);
}

/// Kill `child` if it still runs once the host is `done` (or gave up).
/// The host being done doesn't prove the child ended: an engine error or
/// panic closes the console without ending anyone, and an end can fail
/// (review of 8703b3e4). Once done, the host's own end gets [`SETTLE`]
/// to land first, so its exit code stands.
fn end_survivor(child: &mut WindowsChild, done: bool) {
    let settle = if done { SETTLE } else { Duration::ZERO };
    if exited_within(child, settle).is_some() {
        return;
    }
    if let Err(e) = child.kill_single(Signal::Kill) {
        // Nothing can report it; a child that outlives this is already
        // detached from a console that has closed.
        eprintln!("rusty_term: the shell outlived its console and can't be killed: {e}");
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

/// The `Command` that runs `shell` and then `args` as `CreateProcessW`
/// would. The shell's own arguments go on verbatim, as ConPTY's would;
/// only the trailing `args` are quoted, for whoever reads them.
fn shell_command(shell: &str, args: &[String], cwd: OsString) -> io::Result<Command> {
    let Some((word, rest)) = split_program(shell, |p| locate(p).is_some()) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("rusty_term: no program in the shell `{shell}`"),
        ));
    };
    let reader = reader(word);
    if reader == Reader::Batch {
        return Ok(Command::new(cmd_exe()?, cwd).raw_args(batch_line(shell, args)?));
    }
    // `Command`'s own search is PATH only; what CreateProcessW finds
    // before PATH is passed as a full path.
    let program = locate(word).map_or_else(|| word.into(), PathBuf::into_os_string);
    let cmd = Command::new(program, cwd);
    if reader == Reader::Cmd {
        return Ok(with_raw(cmd, &cmd_tail(rest, args)?));
    }
    let rest: Vec<u16> = rest.encode_utf16().collect();
    Ok(with_raw(cmd.args(args), &rest))
}

/// `cmd` with `raw` as its raw arguments, unless `raw` is empty.
fn with_raw(cmd: Command, raw: &[u16]) -> Command {
    if raw.is_empty() {
        cmd
    } else {
        cmd.raw_args(OsString::from_wide(raw))
    }
}

/// cmd.exe's parameters: its own `rest` from the shell, then `args` quoted
/// by cmd's rules. `Command` would quote argv for the C runtime, which cmd
/// doesn't use, so they go in the raw text too.
fn cmd_tail(rest: &str, args: &[String]) -> io::Result<Vec<u16>> {
    let mut tail: Vec<u16> = rest.encode_utf16().collect();
    push_launcher_args(&mut tail, 0, Reader::Cmd, args)?;
    if rest.is_empty() && !tail.is_empty() {
        tail.remove(0); // the space before the first argument
    }
    Ok(tail)
}

/// The cmd.exe `CreateProcessW` runs a batch file with.
fn cmd_exe() -> io::Result<OsString> {
    let root = std::env::var_os("SystemRoot").ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "rusty_term: SystemRoot is not set")
    })?;
    Ok(PathBuf::from(root)
        .join(r"system32\cmd.exe")
        .into_os_string())
}

/// What `CreateProcessW` hands cmd.exe for the batch `shell` and `args`:
/// `/c `, then the whole line, wrapped in one more pair of quotes when it
/// starts with one (measured on Windows). `args` are quoted by cmd's rules.
fn batch_line(shell: &str, args: &[String]) -> io::Result<OsString> {
    let mut line: Vec<u16> = shell
        .trim_start_matches([' ', '\t'])
        .encode_utf16()
        .collect();
    push_launcher_args(&mut line, 0, Reader::Batch, args)?;
    let quote = u16::from(b'"');
    let wrap = line.first() == Some(&quote);
    let mut out: Vec<u16> = "/c ".encode_utf16().collect();
    out.extend(wrap.then_some(quote));
    out.extend(line);
    out.extend(wrap.then_some(quote));
    Ok(OsString::from_wide(&out))
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
    use crate::backend::windows::WindowsBackend;
    use crate::backend::{Backend, BackendHandle};
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
        let mut h = OwnBackend::new()
            .spawn_shell(80, 24, Some("cmd.exe /c echo own_ok& exit 7"), &[], None)
            .expect("spawn on our own host");
        let out = drain(h.as_ref());
        wait_for(&out, b"own_ok", "the child's output");
        let exited = h.exit_token().expect("the original has an exit token");
        exited();
        assert_eq!(h.reap_exit_status(), Some(7));
    }

    /// The shell's arguments reach cmd.exe as written, as under ConPTY;
    /// trailing arguments follow them, quoted.
    #[test]
    fn cmd_reads_the_shell_arguments_verbatim() {
        let args = ["tail one".to_owned()];
        let mut h = OwnBackend::new()
            .spawn_shell(80, 24, Some(r#"cmd.exe /c echo a"b c"d"#), &args, None)
            .expect("spawn on our own host");
        let out = drain(h.as_ref());
        wait_for(&out, br#"a"b c"d "tail one""#, "cmd's verbatim echo");
        h.exit_token().expect("exit token")();
        assert_eq!(h.reap_exit_status(), Some(0));
    }

    /// A launcher argument can't close cmd's quotes and run a command
    /// (#1279), on either backend: cmd echoes it whole, and the shell exits 0.
    #[test]
    fn a_launcher_argument_cannot_inject_into_cmd_on_either_backend() {
        let backends: [&dyn Backend; 2] = [&OwnBackend::new(), &WindowsBackend];
        for backend in backends {
            let args = [r#"x\" & echo INJECTED& exit 9 & \"y"#.to_owned()];
            let mut h = backend
                .spawn_shell(80, 24, Some("cmd.exe /c echo"), &args, None)
                .expect("spawn");
            let out = drain(h.as_ref());
            wait_for(
                &out,
                br#""x\"" & echo INJECTED& exit 9 & \""y""#,
                "cmd's inert echo",
            );
            h.exit_token().expect("exit token")();
            assert_eq!(h.reap_exit_status(), Some(0));
        }
    }

    /// Delayed expansion turned on by a launcher argument still refuses
    /// a `!` argument, on either backend.
    #[test]
    fn a_launcher_v_on_refuses_bang_on_either_backend() {
        let backends: [&dyn Backend; 2] = [&OwnBackend::new(), &WindowsBackend];
        for backend in backends {
            let args: Vec<String> = ["/v:on", "/c", "echo", "!PATH!"]
                .iter()
                .map(|a| (*a).to_owned())
                .collect();
            let err = backend
                .spawn_shell(80, 24, Some("cmd"), &args, None)
                .err()
                .expect("refused");
            assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        }
    }

    /// `rusty_term -- cmd /c ...`: plain words stay bare, so cmd still
    /// finds its switch and command, and a quoted first word keeps its
    /// quotes through cmd's quote stripping.
    #[test]
    fn cmd_switches_and_a_quoted_command_reach_cmd_on_either_backend() {
        let backends: [&dyn Backend; 2] = [&OwnBackend::new(), &WindowsBackend];
        for backend in backends {
            let args: Vec<String> = ["/c", "echo a b", "&", "exit", "5"]
                .iter()
                .map(|a| (*a).to_owned())
                .collect();
            let mut h = backend
                .spawn_shell(80, 24, Some("cmd"), &args, None)
                .expect("spawn");
            let _out = drain(h.as_ref());
            h.exit_token().expect("exit token")();
            assert_eq!(
                h.reap_exit_status(),
                Some(1),
                "`&` and `exit 5` were not run"
            );
        }
    }

    /// Input typed into the console reaches an interactive cmd.exe, and a
    /// resize is accepted while it runs.
    #[test]
    fn own_host_input_round_trips_through_cmd() {
        let mut h = OwnBackend::new()
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
        let h = OwnBackend::new()
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

    /// Closing a pane doesn't wait for its console to shut down (review
    /// P2 10): the handle's Drop returns at once, and the backend's Drop
    /// waits for the shutdown, which ends the child.
    #[test]
    fn a_dropped_handle_leaves_the_shutdown_to_the_backend() {
        let backend = OwnBackend::new();
        let h = backend
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
        assert!(
            start.elapsed() < Duration::from_millis(100),
            "the handle's Drop hands the wait off"
        );
        drop(backend);
        assert!(
            start.elapsed() < super::SHUTDOWN + Duration::from_secs(1),
            "the backend's Drop is bounded"
        );
        rx.recv_timeout(Duration::from_secs(1))
            .expect("the child ended by the time the backend dropped");
    }

    /// The host being done doesn't stand for the child's end (review of
    /// 8703b3e4): a child still running then, as after an engine error,
    /// is killed.
    #[test]
    fn a_child_alive_after_the_host_is_done_is_killed() {
        let cmd = super::shell_command(
            "ping.exe -n 60 127.0.0.1",
            &[],
            std::env::temp_dir().into_os_string(),
        )
        .expect("command");
        let (master, mut child) = rusty_conhost::OwnPty
            .spawn_concrete(&cmd, platform::term::WinSize { rows: 24, cols: 80 })
            .expect("spawn");
        let start = Instant::now();
        super::end_survivor(&mut child, true);
        assert!(
            start.elapsed() >= super::SETTLE,
            "the host's end came first"
        );
        let status = super::exited_within(&mut child, Duration::from_secs(5));
        assert!(status.is_some(), "killed");
        drop(master);
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
        let mut h = OwnBackend::new()
            .spawn_shell(80, 24, Some(&shell), &[], None)
            .expect("spawn by an unquoted path with spaces");
        h.exit_token().expect("exit token")();
        assert_eq!(h.reap_exit_status(), Some(4));
        drop(h);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A batch file runs through cmd.exe as CreateProcessW runs it: its
    /// arguments reach `%*` as written, trailing arguments quoted.
    #[test]
    fn a_batch_shell_with_arguments_runs_through_cmd() {
        let dir = std::env::temp_dir().join("rusty term own batch");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let script = dir.join("t.cmd");
        std::fs::write(&script, "@echo ARGS=[%*]\r\n").expect("script");
        let shell = format!(r#""{}" -x "a b""#, script.display());
        let args = ["tail one".to_owned()];
        let mut h = OwnBackend::new()
            .spawn_shell(80, 24, Some(&shell), &args, None)
            .expect("spawn a batch shell");
        let out = drain(h.as_ref());
        wait_for(
            &out,
            br#"ARGS=[-x "a b" "tail one"]"#,
            "the batch file's %*",
        );
        h.exit_token().expect("exit token")();
        assert_eq!(h.reap_exit_status(), Some(0));
        drop(h);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_batch_line_is_wrapped_only_when_it_starts_with_a_quote() {
        let line = |shell: &str, args: &[&str]| {
            let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
            super::batch_line(shell, &args)
                .expect("batch line")
                .into_string()
                .expect("utf-8")
        };
        assert_eq!(line(r"C:\t.cmd -x", &[]), r"/c C:\t.cmd -x");
        assert_eq!(
            line(r#""C:\a b\t.cmd" -x"#, &["c d"]),
            r#"/c ""C:\a b\t.cmd" -x "c d"""#
        );
        let refused = super::batch_line(r"C:\t.cmd", &["%PATH%".to_owned()]);
        assert_eq!(
            refused.map_err(|e| e.kind()).err(),
            Some(std::io::ErrorKind::InvalidInput)
        );
        assert_eq!(line(r"C:\t.cmd", &["a\"b", "c"]), r#"/c C:\t.cmd "a""b" c"#);
    }

    /// The own host's command for `shell` and `args`: its raw text and argv.
    fn own_command(shell: &str, args: &[&str]) -> std::io::Result<(String, usize)> {
        let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
        let cmd = super::shell_command(shell, &args, std::env::temp_dir().into_os_string())?;
        let raw = cmd
            .raw_args
            .unwrap_or_default()
            .into_string()
            .expect("utf-8");
        Ok((raw, cmd.argv.len()))
    }

    /// Review of d21ea929: a drive-relative cmd is read as cmd (P1), and a
    /// launcher `/v:on` refuses a `!` argument (P2), on the own host too.
    #[test]
    fn own_host_reads_drive_relative_cmd_and_launcher_options() {
        let (raw, argv) =
            own_command("C:cmd.exe /c echo", &[r#"x\" & calc & \"y"#]).expect("quoted");
        assert_eq!(raw, r#"/c echo "x\"" & calc & \""y""#);
        assert_eq!(argv, 0, "cmd's arguments go in the raw text");
        let err = own_command("cmd", &["/v:on", "/c", "echo", "!SECRET!"]).expect_err("refused");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
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

    /// An extensionless file doesn't hide the program of that name further
    /// on, by bare name or by path; `name.exe` beside it does win.
    #[test]
    fn an_extensionless_file_does_not_hide_the_program() {
        let dir = std::env::temp_dir().join("rusty_term_own_locate");
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(dir.join("cmd"), "not a program").expect("sentinel");
        let _ = std::fs::remove_file(dir.join("cmd.exe"));
        let system = super::locate("cmd").expect("System32 cmd");
        let dirs = [
            dir.clone(),
            system.parent().expect("System32").to_path_buf(),
        ];
        let found = crate::backend::cmdline::locate_in("cmd", &dirs);
        let by_path = super::locate(&dir.join("cmd").to_string_lossy());
        std::fs::copy(&system, dir.join("cmd.exe")).expect("copy");
        let beside = crate::backend::cmdline::locate_in("cmd", &dirs);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(found.as_ref(), Some(&system), "the sentinel is skipped");
        assert_eq!(by_path, None, "a path is tried with .exe only");
        assert_eq!(beside, Some(dir.join("cmd.exe")), "the first place wins");
    }

    /// A clone owns no child: no exit token, no status.
    #[test]
    fn a_clone_owns_no_child() {
        let h = OwnBackend::new()
            .spawn_shell(80, 24, Some("cmd.exe /c exit 0"), &[], None)
            .expect("spawn");
        let mut clone = h.try_clone().expect("clone");
        assert!(clone.exit_token().is_none());
        assert_eq!(clone.reap_exit_status(), None);
    }
}
