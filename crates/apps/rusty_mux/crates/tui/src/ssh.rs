//! `--ssh HOST`: reaches a server on another machine through one OpenSSH
//! forward of a local Unix socket to the server's. The client then connects
//! to the local socket as it would to a local server, once per connection,
//! while SSH logs in once. The SSH login is the identity: the server sees
//! the forward's connections as coming from the user `ssh` logged in as.
//!
//! The local socket lives in a fresh directory only its owner can enter,
//! which is removed, with the forward stopped, when the [`Forward`] drops.

use std::ffi::{OsStr, OsString};
use std::io;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// How often [`Forward::wait_ready`] tries the local socket.
const RETRY: Duration = Duration::from_millis(50);

/// The options that keep the forward owned by the `ssh` this client starts
/// and reaps. A user's config may share a master connection or fork after
/// login, either of which would leave the listener with a process `Drop`
/// never stops. Options given with `-o` win over the config, whose host,
/// user, key and jump settings still apply. `ForkAfterAuthentication`
/// needs OpenSSH 8.7 or later.
const OWNED: [&str; 6] = [
    "ControlMaster=no",
    "ControlPath=none",
    "ControlPersist=no",
    "ForkAfterAuthentication=no",
    "ExitOnForwardFailure=yes",
    "StreamLocalBindUnlink=yes",
];

/// How `ssh` may log in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Login {
    /// It may ask on the terminal, for a password or a host key.
    Interactive,
    /// It must not ask (`BatchMode=yes`): a reconnect, while the client
    /// reads the terminal, logs in with a key or an agent or fails.
    Batch,
}

/// The arguments for `ssh` that forward `local` to `remote` on `host` and
/// do nothing else. `--` keeps a host that starts with `-` from reading as
/// an option. Both paths must be absolute: ssh refuses a bare name, and
/// would resolve a relative one on the server against a directory the
/// user does not choose.
pub fn args(
    host: &str,
    local: &Path,
    remote: &Path,
    login: Login,
) -> Result<Vec<OsString>, String> {
    if host.is_empty() {
        return Err("--ssh needs a host".into());
    }
    for path in [local, remote] {
        if !path.is_absolute() {
            return Err(format!(
                "{}: --ssh needs an absolute socket path",
                path.display()
            ));
        }
    }
    let mut forward = escape(local);
    forward.push(b':');
    forward.extend(escape(remote));
    let mut args = vec![OsString::from("-N"), OsString::from("-T")];
    for option in OWNED {
        args.extend(["-o", option].map(OsString::from));
    }
    if login == Login::Batch {
        args.extend(["-o", "BatchMode=yes"].map(OsString::from));
    }
    args.push("-L".into());
    args.push(OsString::from_vec(forward));
    args.extend(["--", host].map(OsString::from));
    Ok(args)
}

/// `path` as one field of `ssh -L`, which would otherwise split it at `:`,
/// drop each `\`, and expand `%` tokens such as `%h`.
fn escape(path: &Path) -> Vec<u8> {
    let mut field = Vec::new();
    for &byte in path.as_os_str().as_bytes() {
        match byte {
            b'\\' | b':' => field.extend([b'\\', byte]),
            b'%' => field.extend(b"%%"),
            _ => field.push(byte),
        }
    }
    field
}

/// A running `ssh` forward and the private directory of its local socket.
pub struct Forward {
    child: Child,
    dir: PathBuf,
    socket: PathBuf,
}

impl Forward {
    /// Starts `program` (OpenSSH's `ssh`, or a stand-in) forwarding a new
    /// local socket to `remote` on `host`, logging in as `login` allows.
    /// Its stderr stays this process's, for a password prompt or a refusal.
    pub fn start(program: &OsStr, host: &str, remote: &Path, login: Login) -> Result<Self, String> {
        let dir = private_dir().map_err(|e| format!("making the forward's directory: {e}"))?;
        let socket = dir.join("s");
        let spawned = args(host, &socket, remote, login).and_then(|args| {
            Command::new(program)
                .args(args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .spawn()
                .map_err(|e| format!("starting {}: {e}", Path::new(program).display()))
        });
        match spawned {
            Ok(child) => Ok(Self { child, dir, socket }),
            Err(why) => {
                // Nothing is in it yet; a failure leaves an empty directory.
                let _ = std::fs::remove_dir(&dir);
                Err(why)
            }
        }
    }

    /// The local socket to connect to.
    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// Whether `ssh` is still running: once it exits, the forward is gone.
    pub fn running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Waits until `ssh` has bound the local socket, which it does only
    /// after logging in, failing if it exits first or `patience` runs out.
    /// A login can take a while: the user may be typing a password. The
    /// socket is not probed, since each connection to it reaches the server.
    /// Gives up as soon as `cancel` is set.
    pub fn wait_ready(&mut self, patience: Duration, cancel: &AtomicBool) -> Result<(), String> {
        let deadline = Instant::now() + patience;
        loop {
            if self.socket.exists() {
                return Ok(());
            }
            if cancel.load(Ordering::Acquire) {
                return Err("cancelled".into());
            }
            if let Some(status) = self.child.try_wait().map_err(|e| e.to_string())? {
                return Err(format!("ssh exited ({status}) before the forward was up"));
            }
            if Instant::now() >= deadline {
                return Err("the ssh forward did not come up in time".into());
            }
            std::thread::sleep(RETRY);
        }
    }
}

impl Drop for Forward {
    fn drop(&mut self) {
        // It may have exited already; there is nothing to report either
        // way while the client shuts down.
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Err(e) = std::fs::remove_dir_all(&self.dir) {
            eprintln!("rusty-mux: removing {}: {e}", self.dir.display());
        }
    }
}

/// The name `ssh` would connect to for `host`, after the user's config
/// (`HostName`, aliases), from `ssh -G`, which resolves the options and
/// connects to nothing. A `ProxyJump` is not followed: UDP goes straight
/// to this name, and the client falls back to SSH if nothing answers.
pub fn hostname(program: &OsStr, host: &str) -> Result<String, String> {
    let out = Command::new(program)
        .args(["-G", "--", host])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| format!("starting {} -G: {e}", Path::new(program).display()))?;
    if !out.status.success() {
        return Err(format!("ssh -G {host} failed ({})", out.status));
    }
    parse_hostname(&String::from_utf8_lossy(&out.stdout))
        .ok_or_else(|| format!("ssh -G {host} printed no hostname"))
}

/// The value of the `hostname` line in `ssh -G` output.
fn parse_hostname(config: &str) -> Option<String> {
    config.lines().find_map(|line| {
        let (key, value) = line.split_once(' ')?;
        (key == "hostname" && !value.is_empty()).then(|| value.to_owned())
    })
}

/// A new directory in the temp directory that only this user can enter.
pub(crate) fn private_dir() -> io::Result<PathBuf> {
    let name = format!(
        "rusty-mux-ssh-{}-{}",
        std::process::id(),
        rusty_uuid::Uuid::new_v4().simple()
    );
    // The forward needs an absolute path, and TMPDIR may be relative.
    let dir = std::env::current_dir()?
        .join(std::env::temp_dir())
        .join(name);
    std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[OsString]) -> Vec<&str> {
        args.iter().map(|a| a.to_str().unwrap()).collect()
    }

    #[test]
    fn the_forward_is_all_ssh_does() {
        let args = args(
            "box",
            Path::new("/tmp/d/s"),
            Path::new("/run/mux.sock"),
            Login::Interactive,
        )
        .unwrap();
        assert_eq!(
            strings(&args),
            [
                "-N",
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
                "-L",
                "/tmp/d/s:/run/mux.sock",
                "--",
                "box",
            ]
        );
    }

    #[test]
    fn the_hostname_comes_from_ssh_g() {
        let config = "user nano\nhostname box.example.net\nport 22\n";
        assert_eq!(parse_hostname(config).as_deref(), Some("box.example.net"));
        assert_eq!(parse_hostname("user nano\nhostnames x\n"), None);
    }

    #[test]
    fn a_batch_login_never_asks() {
        let args = args("box", Path::new("/l"), Path::new("/r"), Login::Batch).unwrap();
        let args = strings(&args);
        let at = args.iter().position(|a| *a == "BatchMode=yes").unwrap();
        assert_eq!(args[at - 1], "-o");
        assert!(at < args.iter().position(|a| *a == "-L").unwrap());
    }

    #[test]
    fn a_host_that_looks_like_an_option_stays_a_host() {
        let args = args(
            "-oProxyCommand=x",
            Path::new("/l"),
            Path::new("/r"),
            Login::Interactive,
        )
        .unwrap();
        let end = &args[args.len() - 2..];
        assert_eq!(end, ["--", "-oProxyCommand=x"]);
    }

    #[test]
    fn paths_ssh_would_rewrite_are_escaped() {
        let args = args(
            "box",
            Path::new("/l:\\"),
            Path::new("/a%h:b\\c"),
            Login::Interactive,
        )
        .unwrap();
        let forward = args[args.len() - 3].to_str().unwrap();
        assert_eq!(forward, r"/l\:\\:/a%%h\:b\\c");
    }

    #[test]
    fn relative_paths_and_an_empty_host_are_refused() {
        assert!(
            args(
                "box",
                Path::new("/l"),
                Path::new("mux.sock"),
                Login::Interactive
            )
            .is_err()
        );
        assert!(
            args(
                "box",
                Path::new("/l"),
                Path::new("./mux.sock"),
                Login::Interactive
            )
            .is_err()
        );
        assert!(args("box", Path::new("l"), Path::new("/r"), Login::Interactive).is_err());
        assert!(args("", Path::new("/l"), Path::new("/r"), Login::Interactive).is_err());
    }

    /// Asks the real OpenSSH (`ssh -G` resolves the options and connects to
    /// nothing) what the arguments mean under a config that multiplexes and
    /// forks after login, with paths its `-L` grammar would rewrite. Runs
    /// only where `ssh` is installed.
    #[test]
    fn real_ssh_reads_one_owned_forward_of_the_exact_paths() {
        let Some(ssh) = installed_ssh() else {
            eprintln!("no ssh on PATH; skipping the OpenSSH check");
            return;
        };
        let dir = private_dir().unwrap();
        let config = dir.join("config");
        std::fs::write(
            &config,
            "Host box\n ControlMaster auto\n ControlPath /tmp/cm-%C\n \
             ControlPersist yes\n ForkAfterAuthentication yes\n",
        )
        .unwrap();
        let (local, remote) = (dir.join(r"a:b\c%h[d"), Path::new(r"/run/m\x:y%%z"));
        let mut cmd = Command::new(ssh);
        cmd.arg("-F").arg(&config).arg("-G");
        cmd.args(args("box", &local, remote, Login::Interactive).unwrap());
        let out = cmd.stdin(Stdio::null()).output().unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let lines: Vec<&str> = text.lines().collect();
        for want in [
            "controlmaster false",
            "controlpersist no",
            "forkafterauthentication no",
        ] {
            assert!(lines.contains(&want), "no {want:?} in {text}");
        }
        assert!(
            !lines.iter().any(|l| l.starts_with("controlpath ")),
            "{text}"
        );
        let forward = format!("localforward {} {}", local.display(), remote.display());
        assert!(
            lines.contains(&forward.as_str()),
            "no {forward:?} in {text}"
        );
    }

    fn installed_ssh() -> Option<PathBuf> {
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path)
            .map(|dir| dir.join("ssh"))
            .find(|ssh| ssh.is_file())
    }

    /// An `ssh` that exits at once (a refused login, a bad option) fails
    /// the wait, and the directory still goes with the forward.
    #[test]
    fn an_ssh_that_exits_before_the_forward_fails_and_cleans_up() {
        let mut forward =
            Forward::start(OsStr::new("false"), "box", Path::new("/r"), Login::Batch).unwrap();
        let err = forward
            .wait_ready(Duration::from_secs(10), &AtomicBool::new(false))
            .unwrap_err();
        assert!(err.contains("ssh exited"), "{err}");
        let dir = forward.dir.clone();
        drop(forward);
        assert!(!dir.exists(), "{} was left behind", dir.display());
    }

    /// A cancelled wait gives up before the forward comes up, and the
    /// forward still cleans up.
    #[test]
    fn a_cancelled_wait_gives_up_at_once() {
        let mut forward =
            Forward::start(OsStr::new("sleep"), "box", Path::new("/r"), Login::Batch).unwrap();
        let started = Instant::now();
        let err = forward
            .wait_ready(Duration::from_secs(10), &AtomicBool::new(true))
            .unwrap_err();
        assert_eq!(err, "cancelled");
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn a_missing_ssh_is_reported() {
        let program = OsStr::new("/nonexistent/rusty-mux-test-ssh");
        let err = Forward::start(program, "box", Path::new("/r"), Login::Batch)
            .err()
            .unwrap();
        assert!(err.contains("starting /nonexistent"), "{err}");
    }

    #[test]
    fn the_private_dir_is_the_owners_alone() {
        let dir = private_dir().unwrap();
        let mode = std::os::unix::fs::PermissionsExt::mode(
            &std::fs::metadata(&dir).unwrap().permissions(),
        );
        std::fs::remove_dir(&dir).unwrap();
        assert!(dir.is_absolute(), "{}", dir.display());
        assert_eq!(mode & 0o777, 0o700, "{mode:o}");
    }
}
