//! `--host NAME` and `hosts list|add|remove` (C35): saved hosts from the
//! hosts file (see [`rusty_mux_script::hosts`]).
//!
//! `--host` tries a host's addresses in file order and uses the first that
//! connects. Only a failure to connect moves on, so no request is ever
//! sent twice. An `ssh` address is reached through the client library's
//! OpenSSH forward (term pack D194), logging in without asking; on Windows
//! it counts as unreachable and the next one is tried.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use rusty_mux_script::hosts::{self, Address, Host};

use crate::cli::HostsCommand;
use crate::{Env, Exit, Stop};

/// How long `--host` waits for one socket address to connect.
pub(crate) const SOCKET_PATIENCE: Duration = Duration::from_secs(2);

/// How often `events --follow` and `watch` look at the hosts file.
pub(crate) const CHECK_EVERY: Duration = Duration::from_secs(1);

/// The kind `events --follow` and `watch` print when the hosts change.
pub(crate) const CHANGED: &str = "hosts-changed";

/// How long `--host` waits for one ssh address's forward to come up: a
/// login with a key or an agent, never a prompt (term pack D178).
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) const SSH_PATIENCE: Duration = Duration::from_secs(10);

/// Why an `ssh` address is skipped where ctl cannot reach one.
#[cfg_attr(unix, allow(dead_code))]
const NO_SSH: &str = "ssh addresses need Unix";

/// The address of a host that answered: the socket to use and, for an ssh
/// address, the forward that reaches it, which must live while the socket
/// is used. Dropping it stops the forward.
pub(crate) struct Reached {
    pub(crate) socket: PathBuf,
    #[cfg(unix)]
    _forward: Option<rusty_mux_client::ssh::Forward>,
}

impl Reached {
    /// A socket on this machine.
    pub(crate) fn local(socket: PathBuf) -> Self {
        Self {
            socket,
            #[cfg(unix)]
            _forward: None,
        }
    }
}

/// The hosts file, as [`hosts::hosts_path`] finds it.
pub(crate) fn default_path() -> Option<PathBuf> {
    hosts::hosts_path(|name| std::env::var_os(name))
}

/// The hosts at `path`; none when the file does not exist. The error names
/// the file and lists every problem, one per line.
pub(crate) fn load(path: &Path) -> Result<Vec<Host>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    hosts::parse_hosts(&text).map_err(|problems| {
        let lines: Vec<String> = problems
            .iter()
            .map(|p| format!("{}: {p}", path.display()))
            .collect();
        lines.join("\n")
    })
}

/// Replaces the file at `path` with `hosts`: written to a temporary file
/// beside it, synced, then renamed over it, so a reader sees the old list
/// or the new one. On Unix the directory is made 0700 and the file 0600.
/// Of two concurrent writes, the last one wins.
pub(crate) fn save(path: &Path, hosts: &[Host]) -> Result<(), String> {
    let fail = |e: io::Error| format!("{}: {e}", path.display());
    let dir = path
        .parent()
        .ok_or_else(|| format!("{}: no directory", path.display()))?;
    create_private_dir(dir).map_err(fail)?;
    let temp = dir.join(format!(".hosts.json.{}.tmp", std::process::id()));
    let written = write_private(&temp, hosts::to_json(hosts).as_bytes())
        .and_then(|()| std::fs::rename(&temp, path));
    if written.is_err() {
        let _gone = std::fs::remove_file(&temp);
    }
    written.map_err(fail)
}

fn create_private_dir(dir: &Path) -> io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir)
}

fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// `hosts list|add|remove`.
pub(crate) fn run(
    command: &HostsCommand,
    env: &Env,
    json: bool,
    out: &mut dyn Write,
) -> Result<(), (Exit, String)> {
    let path = env.hosts.as_deref().ok_or((
        Exit::Failed,
        "no hosts file: set XDG_CONFIG_HOME or HOME".to_owned(),
    ))?;
    let mut list = load(path).map_err(|e| (Exit::Failed, e))?;
    match command {
        HostsCommand::List => {
            let text = listing(&list, json);
            if !text.is_empty() {
                writeln!(out, "{text}").map_err(|e| (Exit::Failed, e.to_string()))?;
            }
            return Ok(());
        }
        HostsCommand::Add { name, address } => {
            hosts::add(&mut list, name, address.clone()).map_err(|e| (Exit::Usage, e))?;
        }
        HostsCommand::Remove(name) => {
            if !hosts::remove(&mut list, name) {
                return Err((Exit::Usage, format!("no host `{name}`")));
            }
        }
    }
    save(path, &list).map_err(|e| (Exit::Failed, e))
}

/// One line per host, its name then each address; or the hosts as a JSON
/// array.
fn listing(list: &[Host], json: bool) -> String {
    if json {
        let all: Vec<rusty_json::Value> = list.iter().map(Host::to_value).collect();
        return rusty_json::Value::from(all).to_json_string();
    }
    let lines: Vec<String> = list
        .iter()
        .map(|host| {
            let addresses: Vec<String> = host.addresses.iter().map(Address::describe).collect();
            format!("{}  {}", host.name, addresses.join("  "))
        })
        .collect();
    lines.join("\n")
}

/// The socket of the first of host `name`'s addresses that connects, each
/// given [`SOCKET_PATIENCE`] within `stop`. An unknown host is a usage
/// error, a bad hosts file a failure, and no address connecting leaves the
/// host unreachable, with each address's reason.
#[cfg(any(unix, windows))]
pub(crate) fn resolve(name: &str, env: &Env, stop: &Stop) -> Result<Reached, (Exit, String)> {
    let path = env.hosts.as_deref().ok_or((
        Exit::Usage,
        "--host needs a hosts file: set XDG_CONFIG_HOME or HOME".to_owned(),
    ))?;
    let list = load(path).map_err(|e| (Exit::Failed, e))?;
    let host = list.iter().find(|h| h.name == name).ok_or_else(|| {
        (
            Exit::Usage,
            format!("no host `{name}` in {}", path.display()),
        )
    })?;
    let mut reasons = Vec::new();
    for address in &host.addresses {
        match reach(address, env, stop) {
            Ok(reached) => return Ok(reached),
            Err(reason) => reasons.push(format!("  {}: {reason}", address.describe())),
        }
    }
    Err((
        Exit::Unreachable,
        format!("host `{name}`: no address answered\n{}", reasons.join("\n")),
    ))
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn resolve(_: &str, _: &Env, _: &Stop) -> Result<Reached, (Exit, String)> {
    Err((
        Exit::Unreachable,
        "rusty-mux-ctl needs Unix sockets".to_owned(),
    ))
}

/// The socket `address` reaches: a local one once a connection to it
/// succeeds, an ssh one once its forward is up.
#[cfg(any(unix, windows))]
fn reach(address: &Address, env: &Env, stop: &Stop) -> Result<Reached, String> {
    match address {
        Address::Socket { socket } => probe(PathBuf::from(socket), stop).map(Reached::local),
        Address::Ssh { dest, socket, .. } => forward(dest, Path::new(socket), env, stop),
    }
}

/// `socket`, once a connection to it succeeds within [`SOCKET_PATIENCE`].
#[cfg(any(unix, windows))]
fn probe(socket: PathBuf, stop: &Stop) -> Result<PathBuf, String> {
    let within = stop.sooner(Instant::now() + SOCKET_PATIENCE);
    match crate::conn::connect(&socket, &within) {
        Ok(_probe) => Ok(socket),
        Err(e) if e.kind() == io::ErrorKind::TimedOut => Err("no answer within 2 s".into()),
        Err(e) => Err(crate::unreachable(&socket, &e)),
    }
}

/// `remote` on `dest`, through an OpenSSH forward (`$RUSTY_MUX_SSH`, else
/// `ssh`) that logs in without asking, once it is up within
/// [`SSH_PATIENCE`] and `stop`'s deadline. The connection itself is not
/// probed: the first one through the forward is the call's.
#[cfg(unix)]
fn forward(dest: &str, remote: &Path, env: &Env, stop: &Stop) -> Result<Reached, String> {
    use rusty_mux_client::ssh::{Forward, Login};
    use std::sync::atomic::AtomicBool;

    let program = env.ssh.clone().unwrap_or_else(|| "ssh".into());
    let mut forward = Forward::start(&program, dest, remote, Login::Batch)?;
    let patience = stop
        .left()
        .map_or(SSH_PATIENCE, |left| left.min(SSH_PATIENCE));
    forward.wait_ready(patience, &AtomicBool::new(false))?;
    Ok(Reached {
        socket: forward.socket().to_path_buf(),
        _forward: Some(forward),
    })
}

#[cfg(windows)]
fn forward(_: &str, _: &Path, _: &Env, _: &Stop) -> Result<Reached, String> {
    Err(NO_SSH.to_owned())
}

/// Watches the hosts file for `events --follow` and `watch`: the list is
/// read at the start, then at most every [`CHECK_EVERY`], parsed again only
/// when the file's size or modification time moved.
pub(crate) struct Tracker {
    path: PathBuf,
    stamp: Option<(u64, SystemTime)>,
    list: Vec<Host>,
    next: Instant,
    warned: bool,
    /// Why the file did not parse when tracking began: the first check
    /// reports it.
    broken: Option<String>,
}

/// What a [`Tracker::check`] found.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Change {
    None,
    /// The parsed list differs from the last one seen.
    Changed,
    /// The file no longer parses: the reason, given once until it does.
    Broken(String),
}

impl Tracker {
    /// Starts from the list in the file now; no change is reported for it.
    pub(crate) fn new(path: PathBuf) -> Self {
        let stamp = stamp(&path);
        let (list, broken) = match load(&path) {
            Ok(list) => (list, None),
            Err(reason) => (Vec::new(), Some(reason)),
        };
        Self {
            path,
            stamp,
            list,
            next: Instant::now() + CHECK_EVERY,
            warned: broken.is_some(),
            broken,
        }
    }

    /// Looks at the file if [`CHECK_EVERY`] has passed since the last look.
    pub(crate) fn check(&mut self) -> Change {
        if let Some(reason) = self.broken.take() {
            return Change::Broken(reason);
        }
        let now = Instant::now();
        if now < self.next {
            return Change::None;
        }
        self.next = now + CHECK_EVERY;
        let stamp = stamp(&self.path);
        if stamp == self.stamp {
            return Change::None;
        }
        self.stamp = stamp;
        match load(&self.path) {
            Ok(list) => {
                self.warned = false;
                match list == self.list {
                    true => Change::None,
                    false => {
                        self.list = list;
                        Change::Changed
                    }
                }
            }
            Err(_) if self.warned => Change::None,
            Err(reason) => {
                self.warned = true;
                Change::Broken(reason)
            }
        }
    }
}

/// The file's size and modification time; `None` when it is missing.
fn stamp(path: &Path) -> Option<(u64, SystemTime)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

/// The line `events` and `watch` print for [`CHANGED`]: an event with no
/// sequence number and no fields.
pub(crate) fn changed_line(json: bool) -> String {
    match json {
        true => format!("{{\"kind\":\"{CHANGED}\"}}"),
        false => CHANGED.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rusty-mux-hosts-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _fresh = std::fs::remove_dir_all(&dir);
        dir
    }

    fn socket(path: &str) -> Address {
        Address::Socket {
            socket: path.into(),
        }
    }

    #[test]
    fn save_creates_the_directory_and_load_reads_it_back() {
        let dir = temp_dir("save");
        let path = dir.join("rusty_mux").join("hosts.json");
        assert_eq!(load(&path), Ok(Vec::new()), "a missing file is no hosts");
        let mut list = Vec::new();
        hosts::add(&mut list, "box", socket("/a.sock")).expect("added");
        save(&path, &list).expect("saved");
        assert_eq!(load(&path), Ok(list));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).expect("meta").permissions().mode() & 0o777;
            assert_eq!(mode(&path), 0o600);
            assert_eq!(mode(path.parent().expect("dir")), 0o700);
        }
        let leftovers = std::fs::read_dir(path.parent().expect("dir"))
            .expect("dir")
            .count();
        assert_eq!(leftovers, 1, "no temporary file left behind");
        let _gone = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_bad_file_names_itself() {
        let dir = temp_dir("bad");
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("hosts.json");
        std::fs::write(&path, r#"{"hosts": [{"name": "x"}]}"#).expect("written");
        let reason = load(&path).expect_err("bad");
        assert!(reason.starts_with(&path.display().to_string()), "{reason}");
        assert!(reason.contains("hosts[0].addresses"), "{reason}");
        let _gone = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_tracker_reports_a_change_once_and_a_broken_file_once() {
        let dir = temp_dir("track");
        let path = dir.join("rusty_mux").join("hosts.json");
        let mut tracker = Tracker::new(path.clone());
        let due = |tracker: &mut Tracker| {
            tracker.next = Instant::now();
            tracker.check()
        };
        assert_eq!(due(&mut tracker), Change::None, "nothing at the start");
        let mut list = Vec::new();
        hosts::add(&mut list, "box", socket("/a.sock")).expect("added");
        save(&path, &list).expect("saved");
        assert_eq!(due(&mut tracker), Change::Changed);
        assert_eq!(due(&mut tracker), Change::None, "reported once");
        std::fs::write(&path, "{ not json").expect("written");
        assert!(matches!(due(&mut tracker), Change::Broken(_)));
        std::fs::write(&path, "{ still not json at all").expect("written");
        assert_eq!(due(&mut tracker), Change::None, "warned once");
        // The same list again, rewritten: no change.
        save(&path, &list).expect("saved");
        assert_eq!(due(&mut tracker), Change::None);
        std::fs::remove_file(&path).expect("removed");
        assert_eq!(due(&mut tracker), Change::Changed, "no file is no hosts");
        assert!(tracker.check() == Change::None, "not due yet");
        let _gone = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_file_broken_when_tracking_starts_is_reported_first() {
        let dir = temp_dir("broken-start");
        let path = dir.join("hosts.json");
        std::fs::create_dir_all(&dir).expect("made");
        std::fs::write(&path, "{ not json").expect("written");
        let mut tracker = Tracker::new(path.clone());
        assert!(matches!(tracker.check(), Change::Broken(_)), "at once");
        tracker.next = Instant::now();
        assert_eq!(tracker.check(), Change::None, "warned once");
        let mut list = Vec::new();
        hosts::add(&mut list, "box", socket("/a.sock")).expect("added");
        save(&path, &list).expect("saved");
        tracker.next = Instant::now();
        assert_eq!(tracker.check(), Change::Changed, "fixed");
        let _gone = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn listing_is_one_line_per_host() {
        let mut list = Vec::new();
        hosts::add(&mut list, "box", socket("/a.sock")).expect("added");
        let ssh = Address::Ssh {
            dest: "me@box".into(),
            socket: "/b.sock".into(),
            udp_host: None,
        };
        hosts::add(&mut list, "box", ssh).expect("added");
        assert_eq!(
            listing(&list, false),
            "box  socket /a.sock  ssh me@box /b.sock"
        );
        assert!(listing(&list, true).starts_with("[{\"addresses\":"));
        assert_eq!(listing(&[], false), "");
    }
}
