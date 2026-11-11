//! `--host NAME` and `hosts list|add|remove` (C35): saved hosts from the
//! hosts file (see [`rusty_mux_script::hosts`]), and hosts declared in the
//! config (C49, term pack D198/D199). A declared host wins over a saved
//! one of the same name, and ctl never edits a declared one.
//!
//! `--host` tries a host's addresses in file order and uses the first that
//! connects. Only a failure to connect moves on, so no request is ever
//! sent twice. An `ssh` address is reached through the client library's
//! OpenSSH forward (term pack D194), logging in without asking; on Windows
//! it counts as unreachable and the next one is tried.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use rusty_json::Value;
use rusty_mux_script::Problem;
use rusty_mux_script::hosts::{self, Address, Declared, Host};

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
    match read(path)? {
        None => Ok(Vec::new()),
        Some(text) => hosts::parse_hosts(&text).map_err(|p| named(path, &p)),
    }
}

/// The hosts the config at `path` declares; none without a config file.
/// Only `hosts` and `home` are checked, so an unrelated typo in the config
/// never stops `--host`.
pub(crate) fn load_declared(path: Option<&Path>) -> Result<Declared, String> {
    let Some(path) = path else {
        return Ok(Declared::default());
    };
    match read(path)? {
        None => Ok(Declared::default()),
        Some(text) => rusty_mux_script::parse_declared(&text).map_err(|p| named(path, &p)),
    }
}

/// The text at `path`; `None` when there is no file.
fn read(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Every problem, one per line, each naming the file.
fn named(path: &Path, problems: &[Problem]) -> String {
    let lines: Vec<String> = problems
        .iter()
        .map(|p| format!("{}: {p}", path.display()))
        .collect();
    lines.join("\n")
}

/// Every host ctl knows: the config's, then the hosts file's.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Known {
    pub(crate) declared: Declared,
    pub(crate) saved: Vec<Host>,
}

/// One host as `hosts list` shows it.
struct Entry<'a> {
    host: &'a Host,
    /// Declared in the config rather than saved.
    config: bool,
    /// Saved, but a declared host has its name.
    shadowed: bool,
    /// The one the config's `home` names.
    home: bool,
}

impl Known {
    /// The hosts of the files at `hosts` and `config`; a missing file has
    /// none.
    pub(crate) fn load(hosts: Option<&Path>, config: Option<&Path>) -> Result<Self, String> {
        let declared = load_declared(config)?;
        let saved = match hosts {
            Some(path) => load(path)?,
            None => Vec::new(),
        };
        Ok(Self { declared, saved })
    }

    /// The host `--host name` uses: the declared one, else the saved one.
    pub(crate) fn find(&self, name: &str) -> Option<&Host> {
        self.declared
            .host(name)
            .or_else(|| self.saved.iter().find(|h| h.name == name))
    }

    /// Declared hosts, then saved ones, each marked.
    fn entries(&self) -> Vec<Entry<'_>> {
        let home = self.declared.home.as_deref();
        let declared = self.declared.hosts.iter().map(|host| (host, true, false));
        let saved = self
            .saved
            .iter()
            .map(|host| (host, false, self.declared.host(&host.name).is_some()));
        declared
            .chain(saved)
            .map(|(host, config, shadowed)| Entry {
                host,
                config,
                shadowed,
                home: !shadowed && home == Some(host.name.as_str()),
            })
            .collect()
    }

    /// What `--host` can reach, and the home: what `hosts-changed` watches.
    fn view(&self) -> (Vec<Host>, Option<String>) {
        let hosts = self
            .entries()
            .into_iter()
            .filter(|e| !e.shadowed)
            .map(|e| e.host.clone())
            .collect();
        (hosts, self.declared.home.clone())
    }
}

/// Why `config check` refuses `declared`'s home, if it does: it must name
/// a declared host or one saved in the file at `hosts`.
pub(crate) fn check_home(declared: &Declared, hosts: Option<&Path>) -> Result<(), String> {
    let Some(home) = declared.home.as_deref() else {
        return Ok(());
    };
    let known = Known {
        declared: declared.clone(),
        saved: hosts.map(load).transpose()?.unwrap_or_default(),
    };
    match known.find(home) {
        Some(_) => Ok(()),
        None => Err(format!(
            "home `{home}` names no host in the config or the hosts file"
        )),
    }
}

/// The config's home host, if it names one: where a call with no `--host`,
/// `--socket` or `$RUSTY_MUX_SOCKET` goes. A config whose `hosts` or
/// `home` does not parse fails the call rather than guess.
pub(crate) fn home(env: &Env) -> Result<Option<String>, (Exit, String)> {
    load_declared(env.config.as_deref())
        .map(|declared| declared.home)
        .map_err(|e| (Exit::Failed, e))
}

/// The refusal to change a host the config declares.
fn declared_there(name: &str, env: &Env) -> (Exit, String) {
    let config = env
        .config
        .as_deref()
        .map_or("the config".to_owned(), |p| p.display().to_string());
    (
        Exit::Usage,
        format!("host `{name}` is declared in {config}; edit it there"),
    )
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
/// A config whose `hosts` or `home` does not parse is ignored with a
/// warning on `err`, so the saved hosts can still be listed and removed;
/// `add` refuses, since it could shadow a declared name.
pub(crate) fn run(
    command: &HostsCommand,
    env: &Env,
    json: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<(), (Exit, String)> {
    let failed = |e: String| (Exit::Failed, e);
    let declared = match load_declared(env.config.as_deref()) {
        Ok(declared) => declared,
        // Without the config, `add` cannot tell a declared name (D200).
        Err(reason) if matches!(command, HostsCommand::Add { .. }) => {
            return Err((
                Exit::Usage,
                format!("can't check the config's hosts, so nothing was added: {reason}"),
            ));
        }
        Err(reason) => {
            // Nothing better to do if stderr itself is gone.
            let _ = writeln!(err, "rusty-mux-ctl: ignoring the config's hosts: {reason}");
            Declared::default()
        }
    };
    let saved = match env.hosts.as_deref() {
        Some(path) => load(path).map_err(failed)?,
        None => Vec::new(),
    };
    let mut known = Known { declared, saved };
    let mut note = None;
    match command {
        HostsCommand::List => {
            let text = listing(&known, json);
            if !text.is_empty() {
                writeln!(out, "{text}").map_err(|e| failed(e.to_string()))?;
            }
            return Ok(());
        }
        HostsCommand::Add { name, address } => {
            if known.declared.host(name).is_some() {
                return Err(declared_there(name, env));
            }
            hosts::add(&mut known.saved, name, address.clone()).map_err(|e| (Exit::Usage, e))?;
        }
        HostsCommand::Remove(name) => {
            let declared = known.declared.host(name).is_some();
            if !hosts::remove(&mut known.saved, name) {
                return Err(match declared {
                    true => declared_there(name, env),
                    false => (Exit::Usage, format!("no host `{name}`")),
                });
            }
            if declared {
                note = Some(format!(
                    "removed the saved `{name}`; the one in the config stays"
                ));
            }
        }
    }
    let path = env
        .hosts
        .as_deref()
        .ok_or_else(|| failed("no hosts file: set XDG_CONFIG_HOME or HOME".to_owned()))?;
    save(path, &known.saved).map_err(failed)?;
    match note {
        Some(note) => writeln!(out, "{note}").map_err(|e| failed(e.to_string())),
        None => Ok(()),
    }
}

/// One line per host, its name then each address, a declared host's
/// marked `(config)`, the home's `(home)` and a saved one a declared one
/// hides `(saved, hidden by config)`; or the hosts as a JSON array, each
/// with its `source` and, when true, `home` and `shadowed`.
fn listing(known: &Known, json: bool) -> String {
    let entries = known.entries();
    if json {
        let all: Vec<Value> = entries.iter().map(entry_value).collect();
        return Value::from(all).to_json_string();
    }
    let lines: Vec<String> = entries
        .iter()
        .map(|entry| {
            let addresses = entry.host.addresses.iter().map(Address::describe);
            let marks = [
                (entry.config, "(config)"),
                (entry.home, "(home)"),
                (entry.shadowed, "(saved, hidden by config)"),
            ];
            let marks = marks
                .iter()
                .filter(|(on, _)| *on)
                .map(|(_, m)| (*m).to_owned());
            let words: Vec<String> = std::iter::once(entry.host.name.clone())
                .chain(addresses)
                .chain(marks)
                .collect();
            words.join("  ")
        })
        .collect();
    lines.join("\n")
}

fn entry_value(entry: &Entry<'_>) -> Value {
    let mut value = entry.host.to_value();
    if let Value::Object(map) = &mut value {
        let source = if entry.config { "config" } else { "saved" };
        map.insert("source".into(), source.into());
        for (on, key) in [(entry.home, "home"), (entry.shadowed, "shadowed")] {
            if on {
                map.insert(key.into(), true.into());
            }
        }
    }
    value
}

/// The socket of the first of host `name`'s addresses that connects, each
/// given [`SOCKET_PATIENCE`] within `stop`. An unknown host is a usage
/// error, a bad hosts file a failure, and no address connecting leaves the
/// host unreachable, with each address's reason.
#[cfg(any(unix, windows))]
pub(crate) fn resolve(name: &str, env: &Env, stop: &Stop) -> Result<Reached, (Exit, String)> {
    let files: Vec<String> = [&env.config, &env.hosts]
        .into_iter()
        .flatten()
        .map(|p| p.display().to_string())
        .collect();
    if files.is_empty() {
        return Err((
            Exit::Usage,
            "--host needs a hosts file: set XDG_CONFIG_HOME or HOME".to_owned(),
        ));
    }
    let known =
        Known::load(env.hosts.as_deref(), env.config.as_deref()).map_err(|e| (Exit::Failed, e))?;
    let host = known.find(name).ok_or_else(|| {
        (
            Exit::Usage,
            format!("no host `{name}` in {}", files.join(" or ")),
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

    let program = env.ssh.clone().unwrap_or_else(|| "ssh".into());
    let mut forward = Forward::start(&program, dest, remote, Login::Batch)?;
    let patience = stop
        .left()
        .map_or(SSH_PATIENCE, |left| left.min(SSH_PATIENCE));
    forward.wait_ready(patience, || stop.is_cancelled())?;
    Ok(Reached {
        socket: forward.socket().to_path_buf(),
        _forward: Some(forward),
    })
}

#[cfg(windows)]
fn forward(_: &str, _: &Path, _: &Env, _: &Stop) -> Result<Reached, String> {
    Err(NO_SSH.to_owned())
}

/// Watches the hosts file and the config for `events --follow` and
/// `watch`: the hosts `--host` can reach and the home are read at the
/// start, then at most every [`CHECK_EVERY`], parsed again only when a
/// file's size or modification time moved.
pub(crate) struct Tracker {
    hosts: Option<PathBuf>,
    config: Option<PathBuf>,
    stamps: Stamps,
    view: (Vec<Host>, Option<String>),
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
    /// Starts from the files as they are now; no change is reported for
    /// them.
    pub(crate) fn new(hosts: Option<PathBuf>, config: Option<PathBuf>) -> Self {
        let stamps = stamps(&hosts, &config);
        let (view, broken) = match Known::load(hosts.as_deref(), config.as_deref()) {
            Ok(known) => (known.view(), None),
            Err(reason) => ((Vec::new(), None), Some(reason)),
        };
        Self {
            hosts,
            config,
            stamps,
            view,
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
        let stamps = stamps(&self.hosts, &self.config);
        if stamps == self.stamps {
            return Change::None;
        }
        self.stamps = stamps;
        match Known::load(self.hosts.as_deref(), self.config.as_deref()) {
            Ok(known) => {
                self.warned = false;
                let view = known.view();
                match view == self.view {
                    true => Change::None,
                    false => {
                        self.view = view;
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

/// Each file's [`stamp`].
type Stamps = [Option<(u64, SystemTime)>; 2];

fn stamps(hosts: &Option<PathBuf>, config: &Option<PathBuf>) -> Stamps {
    [hosts, config].map(|path| path.as_deref().and_then(stamp))
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
        let mut tracker = Tracker::new(Some(path.clone()), None);
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
        let mut tracker = Tracker::new(Some(path.clone()), None);
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
        let known = Known {
            declared: Declared::default(),
            saved: list,
        };
        assert_eq!(
            listing(&known, false),
            "box  socket /a.sock  ssh me@box /b.sock"
        );
        let json = listing(&known, true);
        assert!(json.starts_with("[{\"addresses\":"), "{json}");
        assert!(json.contains("\"source\":\"saved\""), "{json}");
        assert!(
            !json.contains("home") && !json.contains("shadowed"),
            "{json}"
        );
        assert_eq!(listing(&Known::default(), false), "");
    }

    fn known(declared: &[(&str, &str)], home: Option<&str>, saved: &[(&str, &str)]) -> Known {
        let hosts = |pairs: &[(&str, &str)]| {
            let mut list = Vec::new();
            for (name, path) in pairs {
                hosts::add(&mut list, name, socket(path)).expect("added");
            }
            list
        };
        Known {
            declared: Declared {
                hosts: hosts(declared),
                home: home.map(str::to_owned),
            },
            saved: hosts(saved),
        }
    }

    #[test]
    fn a_declared_host_wins_and_is_marked() {
        let both = known(
            &[("box", "/c.sock")],
            Some("box"),
            &[("box", "/s.sock"), ("lab", "/l.sock")],
        );
        assert_eq!(
            both.find("box").map(|h| &h.addresses[0]),
            Some(&socket("/c.sock"))
        );
        assert_eq!(
            both.find("lab").map(|h| &h.addresses[0]),
            Some(&socket("/l.sock"))
        );
        assert_eq!(both.find("nope"), None);
        assert_eq!(
            listing(&both, false),
            "box  socket /c.sock  (config)  (home)\n\
             box  socket /s.sock  (saved, hidden by config)\n\
             lab  socket /l.sock"
        );
        let json = listing(&both, true);
        assert!(json.contains("\"source\":\"config\""), "{json}");
        assert_eq!(json.matches("\"home\":true").count(), 1, "{json}");
        assert_eq!(json.matches("\"shadowed\":true").count(), 1, "{json}");
        let saved_home = known(&[], Some("lab"), &[("lab", "/l.sock")]);
        assert_eq!(listing(&saved_home, false), "lab  socket /l.sock  (home)");
    }

    #[test]
    fn a_home_must_name_a_host() {
        let dir = temp_dir("home");
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("hosts.json");
        let declared = known(&[("box", "/c.sock")], Some("lab"), &[]).declared;
        let reason = check_home(&declared, Some(&path)).expect_err("no lab");
        assert!(reason.contains("home `lab`"), "{reason}");
        save(&path, &known(&[], None, &[("lab", "/l.sock")]).saved).expect("saved");
        assert_eq!(check_home(&declared, Some(&path)), Ok(()));
        assert_eq!(check_home(&Declared::default(), None), Ok(()));
        let _gone = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_tracker_watches_the_config_hosts_too() {
        let dir = temp_dir("track-config");
        std::fs::create_dir_all(&dir).expect("dir");
        let hosts_file = dir.join("hosts.json");
        let config = dir.join("config.json");
        let mut tracker = Tracker::new(Some(hosts_file.clone()), Some(config.clone()));
        let mut due = |text: &str| {
            std::fs::write(&config, text).expect("written");
            tracker.next = Instant::now();
            tracker.check()
        };
        let declared = r#"{"hosts": [{"name": "box", "addresses": [{"socket": "/c.sock"}]}]}"#;
        assert_eq!(due(declared), Change::Changed, "a declared host");
        let with_rule =
            r#"{"on": [], "hosts": [{"name": "box", "addresses": [{"socket": "/c.sock"}]}]}"#;
        assert_eq!(
            due(with_rule),
            Change::None,
            "the rest of the config is not watched"
        );
        let homed =
            r#"{"home": "box", "hosts": [{"name": "box", "addresses": [{"socket": "/c.sock"}]}]}"#;
        assert_eq!(due(homed), Change::Changed, "a new home");
        save(&hosts_file, &known(&[], None, &[("box", "/s.sock")]).saved).expect("saved");
        assert_eq!(
            due(homed),
            Change::None,
            "a hidden saved host changes nothing"
        );
        assert!(matches!(due("{"), Change::Broken(_)));
        let _gone = std::fs::remove_dir_all(dir);
    }
}
