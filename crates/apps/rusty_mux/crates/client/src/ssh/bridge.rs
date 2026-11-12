//! Reaching a server on another machine through `rusty-mux-ctl bridge`
//! run over SSH, for clients whose `ssh` cannot forward a Unix socket:
//! Windows' OpenSSH refuses a local Unix socket for `-L` (C79 spike). The
//! client connects to a local socket as it would to a local server; each
//! connection runs its own `ssh HOST CTL --socket REMOTE bridge`, joined
//! to it byte for byte. Each connection logs in again, so later logins
//! must not ask: a key or an agent is needed.
//!
//! The local socket lives in a fresh directory only its owner can enter,
//! and on Windows each connection is also checked to come from this user
//! (the server's two checks, R10), since the bridge carries it to the
//! server as the user SSH logs in as. The directory is removed, and no
//! more connections are taken, when the `Bridge` drops. A connection
//! already joined runs until either end closes it: its `ssh` ends with
//! the end of its input, as it does if this process dies.

use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::net::{UnixListener as Listener, UnixStream as Stream};

#[cfg(windows)]
use platform_windows::{WindowsUnixListener as Listener, WindowsUnixStream as Stream};

use super::{BATCH, Login, OWNED};

/// The remote `rusty-mux-ctl` when `RUSTY_MUX_REMOTE_CTL` does not name one.
pub const CTL: &str = "rusty-mux-ctl";

/// How often [`Bridge::wait_ready`] checks the first login.
const RETRY: Duration = Duration::from_millis(50);

/// The most copied at a time between a connection and its `ssh`.
const CHUNK: usize = 64 * 1024;

/// The arguments for `ssh` that run `ctl --socket remote bridge` on `host`
/// and nothing else. `ctl` is shell text for the remote shell, so it may be
/// `~/.local/bin/rusty-mux-ctl`, which a non-login shell's `PATH` may not
/// reach; `remote` is quoted as one word. `--` keeps a host that starts
/// with `-` from reading as an option. `remote` must be absolute, since the
/// remote shell would resolve a relative path against a directory the user
/// does not choose.
pub fn args(host: &str, ctl: &str, remote: &Path, login: Login) -> Result<Vec<OsString>, String> {
    if host.is_empty() {
        return Err("--ssh needs a host".into());
    }
    if ctl.trim().is_empty() {
        return Err("RUSTY_MUX_REMOTE_CTL is empty".into());
    }
    // The remote path is the server's, so it is Unix's notion of absolute.
    let path = remote
        .to_str()
        .ok_or("--ssh: the remote socket path is not UTF-8")?;
    if !path.starts_with('/') {
        return Err(format!("{path}: --ssh needs an absolute socket path"));
    }
    let mut args = vec![OsString::from("-T")];
    for option in OWNED {
        args.extend(["-o", option].map(OsString::from));
    }
    if login == Login::Batch {
        args.extend(["-o", BATCH].map(OsString::from));
    }
    args.extend(["--", host].map(OsString::from));
    args.push(format!("{ctl} --socket {} bridge", quote(path)).into());
    Ok(args)
}

/// `word` as one POSIX shell word: single quotes, with each `'` closed,
/// escaped and reopened.
fn quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', r"'\''"))
}

/// A local socket whose connections each reach the server through their
/// own `ssh` running `rusty-mux-ctl bridge`.
pub struct Bridge {
    socket: PathBuf,
    dir: PathBuf,
    /// The first login, until [`Bridge::wait_ready`] sees it through.
    login: Option<Child>,
    stop: Arc<AtomicBool>,
    accepter: Option<JoinHandle<()>>,
}

impl Bridge {
    /// Binds a new local socket whose connections run `program` (OpenSSH's
    /// `ssh`, or a stand-in) with [`args`], and starts a first bridge with
    /// stdin closed, logging in as `login` allows, to check that the login,
    /// `ctl` and the server all answer before the client connects. Later
    /// connections never ask. Every `ssh`'s stderr stays this process's,
    /// for a password prompt or a refusal.
    pub fn start(
        program: &OsStr,
        host: &str,
        ctl: &str,
        remote: &Path,
        login: Login,
    ) -> Result<Self, String> {
        let first = args(host, ctl, remote, login)?;
        let later = args(host, ctl, remote, Login::Batch)?;
        let dir = private_dir().map_err(|e| format!("making the bridge's directory: {e}"))?;
        let socket = dir.join("s");
        let listener = match listen(&socket) {
            Ok(listener) => listener,
            Err(why) => {
                remove(&dir);
                return Err(why);
            }
        };
        let login = Command::new(program)
            .args(first)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn();
        let login = match login {
            Ok(child) => child,
            Err(e) => {
                drop(listener);
                remove(&dir);
                return Err(format!("starting {}: {e}", Path::new(program).display()));
            }
        };
        let stop = Arc::new(AtomicBool::new(false));
        let run = Run {
            program: program.to_owned(),
            args: later,
            stop: Arc::clone(&stop),
        };
        let accepter = std::thread::spawn(move || run.accept(listener));
        Ok(Self {
            socket,
            dir,
            login: Some(login),
            stop,
            accepter: Some(accepter),
        })
    }

    /// The local socket to connect to.
    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// Whether connections are still taken: false once accepting failed.
    pub fn running(&self) -> bool {
        self.accepter.as_ref().is_some_and(|a| !a.is_finished())
    }

    /// Waits until the first login's bridge has ended well, failing if it
    /// fails or `patience` runs out. A login can take a while: the user may
    /// be typing a password or accepting a host key. Gives up as soon as
    /// `cancelled` returns true. A second call returns at once.
    pub fn wait_ready(
        &mut self,
        patience: Duration,
        cancelled: impl Fn() -> bool,
    ) -> Result<(), String> {
        let Some(child) = &mut self.login else {
            return Ok(());
        };
        let deadline = Instant::now() + patience;
        let failed = loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => break None,
                Ok(Some(status)) => {
                    break Some(format!("ssh exited ({status}) before the bridge was up"));
                }
                Ok(None) => {}
                Err(e) => break Some(format!("waiting for ssh: {e}")),
            }
            if cancelled() {
                break Some("cancelled".into());
            }
            if Instant::now() >= deadline {
                break Some("the ssh bridge did not come up in time".into());
            }
            std::thread::sleep(RETRY);
        };
        if let Some(mut child) = self.login.take() {
            // Exited already when the bridge is up; else it goes with the error.
            let _ = child.kill();
            let _ = child.wait();
        }
        failed.map_or(Ok(()), Err)
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        if let Some(mut child) = self.login.take() {
            // Nothing is reported while the client shuts down.
            let _ = child.kill();
            let _ = child.wait();
        }
        self.stop.store(true, Ordering::Release);
        if let Some(accepter) = self.accepter.take() {
            // Wakes the accept; if the listener is gone, it has ended.
            if !accepter.is_finished() {
                let _ = Stream::connect(&self.socket);
            }
            let _ = accepter.join();
        }
        if let Err(e) = std::fs::remove_dir_all(&self.dir) {
            eprintln!("rusty-mux: removing {}: {e}", self.dir.display());
        }
    }
}

/// What the accepting thread needs.
struct Run {
    program: OsString,
    args: Vec<OsString>,
    stop: Arc<AtomicBool>,
}

impl Run {
    /// Takes connections until told to stop or accepting fails, joining
    /// each one from this user to its own `ssh`.
    fn accept(self, listener: Listener) {
        loop {
            let accepted = listener.accept();
            if self.stop.load(Ordering::Acquire) {
                return;
            }
            let stream = match accepted {
                Ok((stream, _)) => stream,
                Err(e) => {
                    eprintln!("rusty-mux: the ssh bridge stopped taking connections: {e}");
                    return;
                }
            };
            if let Err(why) = peer_check(&stream) {
                eprintln!("rusty-mux: the ssh bridge refused a connection: {why}");
                continue;
            }
            if let Err(why) = self.join(stream) {
                eprintln!("rusty-mux: the ssh bridge: {why}");
            }
        }
    }

    /// Starts `ssh` for `stream` and copies both ways on threads of their
    /// own until each side ends, then reaps it.
    fn join(&self, stream: Stream) -> Result<(), String> {
        let mut child = Command::new(&self.program)
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| format!("starting {}: {e}", Path::new(&self.program).display()))?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("ssh started without pipes".into());
        };
        let reader = match stream.try_clone() {
            Ok(reader) => reader,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("cloning a connection: {e}"));
            }
        };
        std::thread::spawn(move || outbound(reader, stdin));
        std::thread::spawn(move || {
            inbound(stdout, stream);
            // Its output has ended; it exits once its input does too.
            let _ = child.wait();
        });
        Ok(())
    }
}

/// The client's bytes to `ssh`; its end closes `ssh`'s input, which ends
/// the remote bridge's writes to the server.
fn outbound(mut from: Stream, mut to: ChildStdin) {
    // A failed write means ssh is gone, which `inbound` sees too.
    let _ = pump(&mut from, &mut to);
}

/// The server's bytes, from `ssh`, to the client; their end is passed on
/// as the end of the connection's reads.
fn inbound(mut from: ChildStdout, mut to: Stream) {
    // A failed write means the client is gone; ssh ends with its input.
    let _ = pump(&mut from, &mut to);
    let _ = to.shutdown(Shutdown::Write);
}

/// Copies `from` to `to` until `from` ends, flushing after every read.
fn pump(from: &mut impl Read, to: &mut impl Write) -> io::Result<()> {
    let mut chunk = vec![0u8; CHUNK];
    loop {
        let n = match from.read(&mut chunk) {
            Ok(0) => return Ok(()),
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        to.write_all(&chunk[..n])?;
        to.flush()?;
    }
}

/// Binds `socket` for this user alone.
#[cfg(unix)]
fn listen(socket: &Path) -> Result<Listener, String> {
    // The directory is 0700, which is what keeps others out here.
    Listener::bind(socket).map_err(|e| format!("binding {}: {e}", socket.display()))
}

/// Binds `socket` for this user alone: the Windows form of 0600, one entry
/// with full control for this user, as the server sets on its own.
#[cfg(windows)]
fn listen(socket: &Path) -> Result<Listener, String> {
    let listener =
        Listener::bind(socket).map_err(|e| format!("binding {}: {e}", socket.display()))?;
    platform_windows::fs::restrict_to_owner(socket)
        .map_err(|e| format!("restricting {}: {e}", socket.display()))?;
    Ok(listener)
}

/// Unix leaves the check to the directory's mode, as `ssh -L` does.
#[cfg(unix)]
fn peer_check(_: &Stream) -> Result<(), String> {
    Ok(())
}

/// The peer's process, then its token user, compared with ours, as the
/// server checks its own connections.
#[cfg(windows)]
fn peer_check(stream: &Stream) -> Result<(), String> {
    let pid = stream.peer_pid().map_err(|e| format!("peer: {e}"))?;
    match platform_windows::is_current_user(pid) {
        Ok(true) => Ok(()),
        Ok(false) => Err(format!("pid {pid} runs as another user")),
        Err(e) => Err(format!("pid {pid}: {e}")),
    }
}

#[cfg(unix)]
use super::forward::private_dir;

/// A new directory in the temp directory that only this user can enter.
/// Creating it fails if anything is there already, a junction included,
/// so the directory restricted is the one made here.
#[cfg(windows)]
fn private_dir() -> io::Result<PathBuf> {
    let name = format!(
        "rusty-mux-ssh-{}-{}",
        std::process::id(),
        rusty_uuid::Uuid::new_v4().simple()
    );
    let dir = std::path::absolute(std::env::temp_dir().join(name))?;
    std::fs::create_dir(&dir)?;
    platform_windows::fs::restrict_dir_to_owner(&dir).map_err(|e| {
        remove(&dir);
        io::Error::other(format!("restricting {}: {e}", dir.display()))
    })?;
    Ok(dir)
}

/// Removes `dir`, saying so if it cannot.
fn remove(dir: &Path) {
    if let Err(e) = std::fs::remove_dir_all(dir) {
        eprintln!("rusty-mux: removing {}: {e}", dir.display());
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn strings(args: &[OsString]) -> Vec<&str> {
        args.iter().map(|a| a.to_str().unwrap()).collect()
    }

    #[test]
    fn the_bridge_is_all_ssh_runs() {
        let args = args("box", CTL, Path::new("/run/mux.sock"), Login::Interactive).unwrap();
        assert_eq!(
            strings(&args),
            [
                "-T",
                "-o",
                "ControlMaster=no",
                "-o",
                "ControlPath=none",
                "-o",
                "ControlPersist=no",
                "-o",
                "ForkAfterAuthentication=no",
                "-o",
                "ExitOnForwardFailure=yes",
                "-o",
                "StreamLocalBindUnlink=yes",
                "--",
                "box",
                "rusty-mux-ctl --socket '/run/mux.sock' bridge",
            ]
        );
    }

    #[test]
    fn a_batch_login_never_asks() {
        let args = args("box", CTL, Path::new("/r"), Login::Batch).unwrap();
        let args = strings(&args);
        let at = args.iter().position(|a| *a == "BatchMode=yes").unwrap();
        assert_eq!(args[at - 1], "-o");
        assert!(at < args.iter().position(|a| *a == "--").unwrap());
    }

    #[test]
    fn the_remote_path_is_one_shell_word() {
        let args = args("box", "~/bin/ctl", Path::new("/a b/it's;x"), Login::Batch).unwrap();
        assert_eq!(
            args.last().unwrap(),
            r"~/bin/ctl --socket '/a b/it'\''s;x' bridge"
        );
    }

    #[test]
    fn bad_arguments_are_refused() {
        let remote = Path::new("/r");
        assert!(args("", CTL, remote, Login::Batch).is_err());
        assert!(args("box", " ", remote, Login::Batch).is_err());
        assert!(args("box", CTL, Path::new("mux.sock"), Login::Batch).is_err());
        let end = args("-oProxyCommand=x", CTL, remote, Login::Batch).unwrap();
        assert_eq!(
            &end[end.len() - 3..end.len() - 1],
            ["--", "-oProxyCommand=x"]
        );
    }

    /// A stand-in `ssh` that runs `body` in `sh`, ignoring its arguments.
    fn stand_in(body: &str) -> (PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let dir = private_dir().unwrap();
        let program = dir.join("ssh");
        std::fs::write(&program, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        (dir, program)
    }

    /// Each connection reaches its own `ssh`: here `cat`, which sends back
    /// what it is sent, and ends the connection when its input does.
    #[test]
    fn each_connection_is_joined_to_its_own_ssh() {
        let (dir, ssh) = stand_in("exec cat");
        let mut bridge =
            Bridge::start(ssh.as_os_str(), "box", CTL, Path::new("/r"), Login::Batch).unwrap();
        bridge
            .wait_ready(Duration::from_secs(10), || false)
            .unwrap();
        assert!(bridge.running());
        for word in [&b"one"[..], b"two"] {
            let mut conn = Stream::connect(bridge.socket()).unwrap();
            conn.set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            conn.write_all(word).unwrap();
            conn.shutdown(Shutdown::Write).unwrap();
            let mut back = Vec::new();
            conn.read_to_end(&mut back).unwrap();
            assert_eq!(back, word);
        }
        let private = bridge.dir.clone();
        drop(bridge);
        assert!(!private.exists(), "{} was left behind", private.display());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A first login that fails (a refused key, no `ctl`, no server) fails
    /// the wait, and the directory still goes with the bridge.
    #[test]
    fn a_failed_first_login_fails_and_cleans_up() {
        let mut bridge = Bridge::start(
            OsStr::new("false"),
            "box",
            CTL,
            Path::new("/r"),
            Login::Batch,
        )
        .unwrap();
        let err = bridge
            .wait_ready(Duration::from_secs(10), || false)
            .unwrap_err();
        assert!(err.contains("ssh exited"), "{err}");
        let private = bridge.dir.clone();
        drop(bridge);
        assert!(!private.exists(), "{} was left behind", private.display());
    }

    #[test]
    fn a_cancelled_wait_gives_up_at_once() {
        let (dir, ssh) = stand_in("exec sleep 30");
        let mut bridge =
            Bridge::start(ssh.as_os_str(), "box", CTL, Path::new("/r"), Login::Batch).unwrap();
        let started = Instant::now();
        let err = bridge
            .wait_ready(Duration::from_secs(10), || true)
            .unwrap_err();
        assert_eq!(err, "cancelled");
        assert!(started.elapsed() < Duration::from_secs(1));
        drop(bridge);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_missing_ssh_is_reported_and_leaves_nothing() {
        let program = OsStr::new("/nonexistent/rusty-mux-test-ssh");
        let err = Bridge::start(program, "box", CTL, Path::new("/r"), Login::Batch)
            .err()
            .unwrap();
        assert!(err.contains("starting /nonexistent"), "{err}");
    }

    #[test]
    fn the_private_dir_is_the_owners_alone() {
        use std::os::unix::fs::PermissionsExt;
        let dir = private_dir().unwrap();
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        std::fs::remove_dir(&dir).unwrap();
        assert!(dir.is_absolute(), "{}", dir.display());
        assert_eq!(mode & 0o777, 0o700, "{mode:o}");
    }
}
