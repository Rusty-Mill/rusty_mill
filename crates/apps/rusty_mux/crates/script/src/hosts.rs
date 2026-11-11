//! Saved hosts (C35): named servers `rusty-mux-ctl --host NAME` reaches,
//! each a list of addresses tried in order.
//!
//! The file is `rusty_mux/hosts.json` in the config directory
//! ([`hosts_path`]), apart from the hand-written config:
//!
//! ```json
//! {"hosts": [{"name": "box", "addresses": [
//!   {"socket": "/run/user/1000/rusty_mux/default.sock"},
//!   {"ssh": "me@box", "socket": "/run/user/1000/rusty_mux/default.sock", "udp_host": "box.example"}
//! ]}]}
//! ```
//!
//! [`parse_hosts`] reports every problem at once and [`to_json`] writes
//! the file back in one stable form. No I/O: ctl reads and writes the file.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use rusty_json::{Map, Value};

use crate::Problem;

/// The longest host name, in bytes.
pub const MAX_NAME: usize = 64;

/// A named server and the addresses that reach it, in the order to try.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Host {
    /// See [`is_host_name`].
    pub name: String,
    /// At least one, no two equal.
    pub addresses: Vec<Address>,
}

/// One way to reach a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Address {
    /// A local socket, by absolute path.
    Socket { socket: String },
    /// A socket on another machine, reached through `ssh DEST`.
    Ssh {
        /// What `ssh` gets after `--`, such as `me@box` or
        /// `ssh://me@box:2222`.
        dest: String,
        /// The socket's absolute path on that machine.
        socket: String,
        /// Where a long-lived client sends UDP instead of `dest`'s host.
        udp_host: Option<String>,
    },
}

impl Address {
    /// Why this address cannot be saved, if it cannot.
    pub fn check(&self) -> Result<(), String> {
        match self {
            Self::Socket { socket } => absolute("socket", socket),
            Self::Ssh {
                dest,
                socket,
                udp_host,
            } => {
                word("ssh", dest)?;
                absolute("socket", socket)?;
                udp_host.as_deref().map_or(Ok(()), |h| word("udp_host", h))
            }
        }
    }

    /// One line: `socket PATH` or `ssh DEST PATH [udp HOST]`.
    pub fn describe(&self) -> String {
        match self {
            Self::Socket { socket } => format!("socket {socket}"),
            Self::Ssh {
                dest,
                socket,
                udp_host: None,
            } => format!("ssh {dest} {socket}"),
            Self::Ssh {
                dest,
                socket,
                udp_host: Some(udp),
            } => format!("ssh {dest} {socket} udp {udp}"),
        }
    }

    /// The address as the file stores it.
    pub fn to_value(&self) -> Value {
        let mut map = Map::new();
        match self {
            Self::Socket { socket } => {
                map.insert("socket".into(), socket.as_str().into());
            }
            Self::Ssh {
                dest,
                socket,
                udp_host,
            } => {
                map.insert("ssh".into(), dest.as_str().into());
                map.insert("socket".into(), socket.as_str().into());
                if let Some(udp) = udp_host {
                    map.insert("udp_host".into(), udp.as_str().into());
                }
            }
        }
        map.into()
    }
}

impl Host {
    /// The host as the file stores it.
    pub fn to_value(&self) -> Value {
        let mut map = Map::new();
        map.insert("name".into(), self.name.as_str().into());
        let addresses: Vec<Value> = self.addresses.iter().map(Address::to_value).collect();
        map.insert("addresses".into(), addresses.into());
        map.into()
    }
}

/// The hosts file: `rusty_mux/hosts.json` under `$XDG_CONFIG_HOME`, else
/// under `$HOME/.config`. An empty or relative value is ignored, as the XDG
/// spec says, so the file is never written relative to the current folder.
/// `$RUSTY_MUX_CONFIG` names only the config file and does not move this one.
pub fn hosts_path(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let absolute = |name: &str| var(name).map(PathBuf::from).filter(|p| p.is_absolute());
    let base = absolute("XDG_CONFIG_HOME").or_else(|| Some(absolute("HOME")?.join(".config")))?;
    Some(base.join("rusty_mux").join("hosts.json"))
}

/// Whether `name` can name a host: 1 to [`MAX_NAME`] bytes of ASCII
/// letters, digits, `.`, `_` and `-`, not starting with `-` or `.`.
pub fn is_host_name(name: &str) -> bool {
    (1..=MAX_NAME).contains(&name.len())
        && !name.starts_with(['-', '.'])
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Adds `address` to the host called `name`, creating the host at the end
/// of the list. Refuses a bad name, a bad address and one the host
/// already has.
pub fn add(hosts: &mut Vec<Host>, name: &str, address: Address) -> Result<(), String> {
    if !is_host_name(name) {
        return Err(bad_name(name));
    }
    address.check()?;
    let Some(host) = hosts.iter_mut().find(|h| h.name == name) else {
        hosts.push(Host {
            name: name.to_owned(),
            addresses: vec![address],
        });
        return Ok(());
    };
    if host.addresses.contains(&address) {
        return Err(format!("{name} already has {}", address.describe()));
    }
    host.addresses.push(address);
    Ok(())
}

/// Removes the host called `name`; false when there is none.
pub fn remove(hosts: &mut Vec<Host>, name: &str) -> bool {
    let before = hosts.len();
    hosts.retain(|h| h.name != name);
    hosts.len() != before
}

/// The file's text for `hosts`: pretty JSON with a final newline.
pub fn to_json(hosts: &[Host]) -> String {
    let mut root = Map::new();
    let hosts: Vec<Value> = hosts.iter().map(Host::to_value).collect();
    root.insert("hosts".into(), hosts.into());
    let mut text = Value::from(root).to_json_string_pretty();
    text.push('\n');
    text
}

/// The hosts in `text`, in file order, or every problem in it. Unknown keys
/// are problems, so a typo is not silently ignored.
pub fn parse_hosts(text: &str) -> Result<Vec<Host>, Vec<Problem>> {
    let root = Value::from_json_str(text).map_err(|e| {
        vec![Problem {
            at: "hosts".into(),
            reason: format!("not JSON: {e}"),
        }]
    })?;
    let mut check = Check::default();
    let hosts = check.root(&root);
    match check.problems.is_empty() {
        true => Ok(hosts),
        false => Err(check.problems),
    }
}

/// The hosts a config declares (C49): its `hosts` array, as in the hosts
/// file, and its `home`, the name of a host from either file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Declared {
    /// In config order, names unique.
    pub hosts: Vec<Host>,
    /// See [`is_host_name`]; whether a host has it is checked by the
    /// caller, which can also read the hosts file.
    pub home: Option<String>,
}

impl Declared {
    /// The declared host called `name`.
    pub fn host(&self, name: &str) -> Option<&Host> {
        self.hosts.iter().find(|h| h.name == name)
    }
}

/// The `hosts` and `home` of a config object, adding every problem in
/// them to `problems`.
pub(crate) fn declared(map: &Map, problems: &mut Vec<Problem>) -> Declared {
    let mut check = Check::default();
    let hosts = map
        .get("hosts")
        .map_or_else(Vec::new, |items| check.list(items));
    let home = map.get("home").and_then(|value| {
        let name = check.string("home", Some(value))?;
        if !is_host_name(name) {
            check.problem("home", bad_name(name));
            return None;
        }
        Some(name.to_owned())
    });
    problems.append(&mut check.problems);
    Declared { hosts, home }
}

#[derive(Default)]
struct Check {
    problems: Vec<Problem>,
}

impl Check {
    fn problem(&mut self, at: &str, reason: impl Into<String>) {
        self.problems.push(Problem {
            at: at.to_owned(),
            reason: reason.into(),
        });
    }

    fn root(&mut self, root: &Value) -> Vec<Host> {
        let Some(map) = self.object("hosts", root, &["hosts"]) else {
            return Vec::new();
        };
        match map.get("hosts") {
            None => Vec::new(),
            Some(items) => self.list(items),
        }
    }

    /// A `hosts` array, at `hosts`, names unique.
    fn list(&mut self, items: &Value) -> Vec<Host> {
        let Some(items) = items.as_array() else {
            self.problem("hosts", "expected an array");
            return Vec::new();
        };
        let mut hosts: Vec<Host> = Vec::new();
        for (i, item) in items.iter().enumerate() {
            let at = format!("hosts[{i}]");
            let Some(host) = self.host(&at, item) else {
                continue;
            };
            if hosts.iter().any(|h| h.name == host.name) {
                self.problem(&at, format!("a second host called `{}`", host.name));
                continue;
            }
            hosts.push(host);
        }
        hosts
    }

    fn host(&mut self, at: &str, value: &Value) -> Option<Host> {
        let map = self.object(at, value, &["name", "addresses"])?;
        let name = self.string(&format!("{at}.name"), map.get("name"));
        if let Some(bad) = name.filter(|n| !is_host_name(n)) {
            self.problem(&format!("{at}.name"), bad_name(bad));
        }
        let addresses = self.addresses(&format!("{at}.addresses"), map.get("addresses"));
        let name = name.filter(|n| is_host_name(n))?;
        Some(Host {
            name: name.to_owned(),
            addresses: addresses?,
        })
    }

    fn addresses(&mut self, at: &str, value: Option<&Value>) -> Option<Vec<Address>> {
        let Some(items) = value.and_then(Value::as_array).filter(|a| !a.is_empty()) else {
            self.problem(at, "expected an array of at least one address");
            return None;
        };
        let before = self.problems.len();
        let mut addresses: Vec<Address> = Vec::new();
        for (i, item) in items.iter().enumerate() {
            let at = format!("{at}[{i}]");
            let Some(address) = self.address(&at, item) else {
                continue;
            };
            if addresses.contains(&address) {
                self.problem(&at, format!("{} is listed twice", address.describe()));
                continue;
            }
            addresses.push(address);
        }
        (self.problems.len() == before).then_some(addresses)
    }

    fn address(&mut self, at: &str, value: &Value) -> Option<Address> {
        let map = self.object(at, value, &["socket", "ssh", "udp_host"])?;
        let socket = self.string(&format!("{at}.socket"), map.get("socket"));
        let address = match map.get("ssh") {
            None => {
                if map.contains_key("udp_host") {
                    self.problem(at, "`udp_host` needs `ssh`");
                }
                Address::Socket {
                    socket: socket?.to_owned(),
                }
            }
            Some(dest) => {
                let dest = self.string(&format!("{at}.ssh"), Some(dest));
                let udp_host = match map.get("udp_host") {
                    None => None,
                    Some(udp) => Some(self.string(&format!("{at}.udp_host"), Some(udp))?),
                };
                Address::Ssh {
                    dest: dest?.to_owned(),
                    socket: socket?.to_owned(),
                    udp_host: udp_host.map(str::to_owned),
                }
            }
        };
        if let Err(reason) = address.check() {
            self.problem(at, reason);
            return None;
        }
        Some(address)
    }

    /// `value` as an object with no keys but `allowed`.
    fn object<'v>(&mut self, at: &str, value: &'v Value, allowed: &[&str]) -> Option<&'v Map> {
        let Some(map) = value.as_object() else {
            self.problem(at, "expected an object");
            return None;
        };
        for key in map.keys().filter(|k| !allowed.contains(&k.as_str())) {
            self.problem(at, format!("unknown key `{key}`"));
        }
        Some(map)
    }

    /// A required string.
    fn string<'v>(&mut self, at: &str, value: Option<&'v Value>) -> Option<&'v str> {
        match value.map(Value::as_str) {
            Some(Some(text)) => Some(text),
            Some(None) => {
                self.problem(at, "expected a string");
                None
            }
            None => {
                self.problem(at, "missing");
                None
            }
        }
    }
}

fn bad_name(name: &str) -> String {
    format!(
        "`{name}` is not a host name: 1 to {MAX_NAME} letters, digits, `.`, `_` or `-`, \
         not starting with `-` or `.`"
    )
}

/// An absolute path, here or on the remote machine (where `/` starts one).
fn absolute(what: &str, path: &str) -> Result<(), String> {
    match Path::new(path).is_absolute() || path.starts_with('/') {
        true => Ok(()),
        false => Err(format!("{what} `{path}` is not an absolute path")),
    }
}

/// A non-empty value with no control characters that cannot pass for an
/// option.
fn word(what: &str, text: &str) -> Result<(), String> {
    if text.is_empty() || text.starts_with('-') || text.chars().any(char::is_control) {
        return Err(format!(
            "{what} `{text}` must be non-empty, not start with `-` and hold no control characters"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn socket(path: &str) -> Address {
        Address::Socket {
            socket: path.into(),
        }
    }

    #[test]
    fn a_file_round_trips_in_order() {
        let mut hosts = Vec::new();
        add(&mut hosts, "box", socket("/a.sock")).expect("added");
        let ssh = Address::Ssh {
            dest: "me@box".into(),
            socket: "/run/b.sock".into(),
            udp_host: Some("box.example".into()),
        };
        add(&mut hosts, "box", ssh.clone()).expect("added");
        add(&mut hosts, "alpha", socket("/c.sock")).expect("added");
        let text = to_json(&hosts);
        assert_eq!(parse_hosts(&text), Ok(hosts.clone()));
        assert_eq!(hosts[0].addresses, vec![socket("/a.sock"), ssh]);
        assert_eq!(hosts[1].name, "alpha", "file order, not sorted");
        assert!(text.ends_with("}\n"), "{text}");
    }

    #[test]
    fn every_problem_is_reported() {
        let text = r#"{"hosts": [
            {"name": "-x", "addresses": [{"socket": "rel"}]},
            {"name": "ok", "addresses": []},
            {"name": "two", "addresses": [{"socket": "/a"}, {"socket": "/a"}]},
            {"name": "u", "addresses": [{"socket": "/a", "udp_host": "h"}], "port": 1},
            {"name": "s", "addresses": [{"ssh": "-oProxyCommand=x", "socket": "/a"}]},
            {"name": "s", "addresses": [{"socket": "/b"}]},
            {"name": "s", "addresses": [{"socket": "/c"}]}
        ], "extra": 1}"#;
        let problems = parse_hosts(text).expect_err("bad");
        let shown: Vec<String> = problems.iter().map(ToString::to_string).collect();
        let want = [
            "hosts: unknown key `extra`",
            "hosts[0].name: `-x` is not a host name",
            "hosts[0].addresses[0]: socket `rel` is not an absolute path",
            "hosts[1].addresses: expected an array of at least one address",
            "hosts[2].addresses[1]: socket /a is listed twice",
            "hosts[3]: unknown key `port`",
            "hosts[3].addresses[0]: `udp_host` needs `ssh`",
            "hosts[4].addresses[0]: ssh `-oProxyCommand=x` must be non-empty",
        ];
        for want in want {
            assert!(
                shown.iter().any(|s| s.starts_with(want)),
                "{want} missing from {shown:#?}"
            );
        }
        // The fourth `s` is valid but the fifth repeats its name.
        assert!(
            shown
                .iter()
                .any(|s| s == "hosts[6]: a second host called `s`")
        );
    }

    #[test]
    fn an_empty_object_is_no_hosts() {
        assert_eq!(parse_hosts("{}"), Ok(Vec::new()));
        assert!(parse_hosts("[]").is_err());
        assert!(parse_hosts("not json").is_err());
    }

    #[test]
    fn host_names() {
        let longest = "a".repeat(MAX_NAME);
        for good in ["box", "a.b_c-d", "9", longest.as_str()] {
            assert!(is_host_name(good), "{good}");
        }
        let too_long = "a".repeat(MAX_NAME + 1);
        for bad in ["", "-a", ".a", "a b", "é", "a/b", too_long.as_str()] {
            assert!(!is_host_name(bad), "{bad}");
        }
    }

    #[test]
    fn add_refuses_a_duplicate_and_remove_reports_a_miss() {
        let mut hosts = Vec::new();
        add(&mut hosts, "box", socket("/a")).expect("added");
        assert!(add(&mut hosts, "box", socket("/a")).is_err());
        assert!(add(&mut hosts, "bad name", socket("/a")).is_err());
        assert!(add(&mut hosts, "box", socket("a")).is_err());
        assert!(!remove(&mut hosts, "other"));
        assert!(remove(&mut hosts, "box"));
        assert!(hosts.is_empty());
    }

    /// `path` made absolute on this platform: Windows needs a drive.
    fn abs(path: &str) -> String {
        match cfg!(windows) {
            true => format!("C:{path}"),
            false => path.to_owned(),
        }
    }

    #[test]
    fn the_file_lives_beside_the_config() {
        let var = |name: &str| match name {
            "HOME" => Some(OsString::from(abs("/home/me"))),
            "RUSTY_MUX_CONFIG" => Some(OsString::from(abs("/elsewhere/c.json"))),
            _ => None,
        };
        assert_eq!(
            hosts_path(var),
            Some(PathBuf::from(abs("/home/me/.config/rusty_mux/hosts.json")))
        );
        let xdg = |name: &str| (name == "XDG_CONFIG_HOME").then(|| OsString::from(abs("/x")));
        assert_eq!(
            hosts_path(xdg),
            Some(PathBuf::from(abs("/x/rusty_mux/hosts.json")))
        );
        assert_eq!(hosts_path(|_| None), None);
    }

    #[test]
    fn an_empty_or_relative_base_is_ignored() {
        let var = |xdg: String, home: String| {
            move |name: &str| match name {
                "XDG_CONFIG_HOME" => Some(OsString::from(&xdg)),
                "HOME" => Some(OsString::from(&home)),
                _ => None,
            }
        };
        let home = Some(PathBuf::from(abs("/home/me/.config/rusty_mux/hosts.json")));
        assert_eq!(hosts_path(var(String::new(), abs("/home/me"))), home);
        assert_eq!(hosts_path(var("cfg".into(), abs("/home/me"))), home);
        assert_eq!(hosts_path(var(String::new(), String::new())), None);
        assert_eq!(hosts_path(var(String::new(), "me".into())), None);
    }
}
