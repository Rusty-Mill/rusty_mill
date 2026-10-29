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
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// How often [`Forward::wait_ready`] tries the local socket.
const RETRY: Duration = Duration::from_millis(50);

/// The arguments for `ssh` that forward `local` to `remote` on `host` and
/// do nothing else. `--` keeps a host that starts with `-` from reading as
/// an option. `ssh -L` splits its argument at `:`, so neither path may
/// hold one.
pub fn args(host: &str, local: &Path, remote: &Path) -> Result<Vec<OsString>, String> {
    if host.is_empty() {
        return Err("--ssh needs a host".into());
    }
    for path in [local, remote] {
        if path.as_os_str().as_encoded_bytes().contains(&b':') {
            return Err(format!(
                "{}: ssh cannot forward a path with `:`",
                path.display()
            ));
        }
    }
    let mut forward = local.as_os_str().to_owned();
    forward.push(":");
    forward.push(remote);
    let mut args: Vec<OsString> = [
        "-N",
        "-T",
        "-o",
        "ExitOnForwardFailure=yes",
        "-o",
        "StreamLocalBindUnlink=yes",
        "-L",
    ]
    .map(OsString::from)
    .into();
    args.push(forward);
    args.extend(["--", host].map(OsString::from));
    Ok(args)
}

/// A running `ssh` forward and the private directory of its local socket.
pub struct Forward {
    child: Child,
    dir: PathBuf,
    socket: PathBuf,
}

impl Forward {
    /// Starts `program` (OpenSSH's `ssh`, or a stand-in) forwarding a new
    /// local socket to `remote` on `host`. Its stderr stays this process's,
    /// for a password prompt or a refusal.
    pub fn start(program: &OsStr, host: &str, remote: &Path) -> Result<Self, String> {
        let dir = private_dir().map_err(|e| format!("making the forward's directory: {e}"))?;
        let socket = dir.join("s");
        let spawned = args(host, &socket, remote).and_then(|args| {
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

    /// Waits until `ssh` has bound the local socket, which it does only
    /// after logging in, failing if it exits first or `patience` runs out.
    /// A login can take a while: the user may be typing a password. The
    /// socket is not probed, since each connection to it reaches the server.
    pub fn wait_ready(&mut self, patience: Duration) -> Result<(), String> {
        let deadline = Instant::now() + patience;
        loop {
            if self.socket.exists() {
                return Ok(());
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

/// A new directory in the temp directory that only this user can enter.
fn private_dir() -> io::Result<PathBuf> {
    let name = format!(
        "rusty-mux-ssh-{}-{}",
        std::process::id(),
        rusty_uuid::Uuid::new_v4().simple()
    );
    let dir = std::env::temp_dir().join(name);
    std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_forward_is_all_ssh_does() {
        let args = args("box", Path::new("/tmp/d/s"), Path::new("/run/mux.sock")).unwrap();
        let args: Vec<&str> = args.iter().map(|a| a.to_str().unwrap()).collect();
        assert_eq!(
            args,
            [
                "-N",
                "-T",
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
    fn a_host_that_looks_like_an_option_stays_a_host() {
        let args = args("-oProxyCommand=x", Path::new("/l"), Path::new("/r")).unwrap();
        let end = &args[args.len() - 2..];
        assert_eq!(end, ["--", "-oProxyCommand=x"]);
    }

    #[test]
    fn paths_with_a_colon_and_an_empty_host_are_refused() {
        assert!(args("box", Path::new("/l"), Path::new("/a:b")).is_err());
        assert!(args("box", Path::new("/l:x"), Path::new("/r")).is_err());
        assert!(args("", Path::new("/l"), Path::new("/r")).is_err());
    }

    #[test]
    fn the_private_dir_is_the_owners_alone() {
        let dir = private_dir().unwrap();
        let mode = std::os::unix::fs::PermissionsExt::mode(
            &std::fs::metadata(&dir).unwrap().permissions(),
        );
        std::fs::remove_dir(&dir).unwrap();
        assert_eq!(mode & 0o777, 0o700, "{mode:o}");
    }
}
