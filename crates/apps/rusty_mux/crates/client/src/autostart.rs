//! Starting a server when nothing answers on the default socket (C69
//! slice (b), `docs/projects/mux/C69-LIFECYCLE.md`).
//!
//! Only `rusty-mux` and `rusty-mux-gui` call this, and only for the default
//! socket: a named socket or `$RUSTY_MUX_SOCKET` means the user runs the
//! server, and ctl, MCP and `--ssh` never start one. The server is
//! `rusty-mux-server --idle-exit 60 <socket>`, beside the client's own
//! program if it is there, else from `PATH`, detached from the client's
//! terminal and output. Two clients starting one each are settled by the
//! server's own bind: the second refuses the first's live name and exits,
//! and both clients connect to the first.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// How long a started server stays without blocks or clients.
pub const IDLE_EXIT_SECS: u64 = 60;
/// How long to wait for a started server to answer.
const PATIENCE: Duration = Duration::from_secs(5);
/// How often to try it meanwhile.
const RETRY: Duration = Duration::from_millis(25);

#[cfg(windows)]
const SERVER: &str = "rusty-mux-server.exe";
#[cfg(not(windows))]
const SERVER: &str = "rusty-mux-server";

/// Makes sure a server answers on `socket`, starting one if nothing does.
/// Returns whether it started one.
pub fn ensure(socket: &Path) -> Result<bool, String> {
    ensure_with(&server_program(), socket)
}

/// [`ensure`], starting `program` as the server.
fn ensure_with(program: &Path, socket: &Path) -> Result<bool, String> {
    if answers(socket) {
        return Ok(false);
    }
    let mut child = start(program, socket)?;
    let deadline = Instant::now() + PATIENCE;
    while !answers(socket) {
        // A server that lost the name to another client's refuses it only
        // once that one answers, so an exit with nothing answering is an
        // error.
        if let Ok(Some(status)) = child.try_wait()
            && !answers(socket)
        {
            return Err(format!(
                "the server it started exited ({status}); run rusty-mux-server by hand to see why"
            ));
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "started a server, but nothing answered on {} within {PATIENCE:?}",
                socket.display()
            ));
        }
        std::thread::sleep(RETRY);
    }
    Ok(true)
}

/// Whether a server answers on `socket` now.
fn answers(socket: &Path) -> bool {
    crate::attach::connect(socket).is_ok()
}

/// `rusty-mux-server` beside this program, else the bare name for `PATH`.
fn server_program() -> PathBuf {
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join(SERVER)))
        .filter(|path| path.is_file());
    beside.unwrap_or_else(|| PathBuf::from(SERVER))
}

/// Starts `program` serving `socket`, detached from this terminal: its own
/// process group on Unix (so Ctrl+C here doesn't reach it), no console
/// window on Windows; no stdio, so a client gone doesn't break its writes.
fn start(program: &Path, socket: &Path) -> Result<Child, String> {
    let mut command = Command::new(program);
    command
        .arg("--idle-exit")
        .arg(IDLE_EXIT_SECS.to_string())
        .arg(socket)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    detach(&mut command);
    command
        .spawn()
        .map_err(|e| format!("starting {}: {e}", program.display()))
}

#[cfg(unix)]
fn detach(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(windows)]
fn detach(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    /// `CREATE_NEW_PROCESS_GROUP`: Ctrl+C in the client's console stays there.
    const NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    /// `CREATE_NO_WINDOW`: a console of its own, never shown.
    const NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(NEW_PROCESS_GROUP | NO_WINDOW);
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn socket(tag: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("rmx-auto-{tag}-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn a_server_that_answers_is_left_alone() {
        let path = socket("answers");
        let _listener = std::os::unix::net::UnixListener::bind(&path).expect("bind");
        let missing = Path::new("/nonexistent/rusty-mux-server");
        assert_eq!(ensure_with(missing, &path), Ok(false));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_program_that_cannot_start_is_an_error() {
        let path = socket("missing");
        let err =
            ensure_with(Path::new("/nonexistent/rusty-mux-server"), &path).expect_err("no server");
        assert!(err.contains("starting"), "{err}");
    }

    #[test]
    fn a_server_that_exits_without_answering_is_an_error() {
        let path = socket("exits");
        let err = ensure_with(Path::new("/bin/false"), &path).expect_err("exited");
        assert!(err.contains("exited"), "{err}");
    }
}
