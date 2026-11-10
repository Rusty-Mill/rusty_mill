//! `ssh check|diagnose|setup DEST` (C48): whether `rusty-mux --ssh DEST`
//! will work, from what `ssh -G` resolves for it, and the `Host` block to
//! add when the user has none.
//!
//! Nothing here connects or writes. `ssh -G` resolves the options
//! `rusty-mux --ssh` passes and connects to nothing; known hosts are looked
//! up with `ssh-keygen -F`, which only reads.

use std::ffi::{OsStr, OsString};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use rusty_mux_client::ssh::{BATCH, OWNED};

use crate::Exit;
use crate::cli::{SshCommand, SshSetup};

/// The options `rusty-mux --ssh` passes, plus the `BatchMode` its
/// reconnects and ctl's own forwards add: the client library's, shared.
pub(crate) const OPTIONS: [&str; 7] = {
    let [a, b, c, d, e, f] = OWNED;
    [a, b, c, d, e, f, BATCH]
};

/// What the checks found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Finding {
    /// A problem makes `rusty-mux --ssh` fail; a warning only limits it.
    pub(crate) problem: bool,
    pub(crate) text: String,
    pub(crate) fix: String,
}

impl Finding {
    fn problem(text: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            problem: true,
            text: text.into(),
            fix: fix.into(),
        }
    }

    fn warning(text: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            problem: false,
            text: text.into(),
            fix: fix.into(),
        }
    }
}

/// `ssh -G` output: each key, lowercase, with its values in order.
#[derive(Debug, Default)]
pub(crate) struct Resolved {
    pairs: Vec<(String, String)>,
}

impl Resolved {
    pub(crate) fn parse(text: &str) -> Self {
        let pairs = text
            .lines()
            .filter_map(|line| line.split_once(' '))
            .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().to_owned()))
            .collect();
        Self { pairs }
    }

    fn all(&self, key: &str) -> impl Iterator<Item = &str> {
        self.pairs
            .iter()
            .filter(move |(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.all(key)
            .next()
            .filter(|v| !v.is_empty() && *v != "none")
    }
}

/// What the checks need from the machine, so tests can supply it.
pub(crate) struct Machine<'a> {
    pub(crate) home: Option<&'a Path>,
    /// Whether an agent is offered (`$SSH_AUTH_SOCK`).
    pub(crate) agent: bool,
    /// Whether `file` is in `known` for `host`; `None` when it cannot be
    /// told (no `ssh-keygen`).
    pub(crate) known: &'a dyn Fn(&Path, &str) -> Option<bool>,
}

/// The findings for a destination whose `ssh -G` resolved to `resolved`.
pub(crate) fn findings(resolved: &Resolved, machine: &Machine<'_>) -> Vec<Finding> {
    let mut found = Vec::new();
    if resolved.get("clearallforwardings") == Some("yes") {
        found.push(Finding::problem(
            "ClearAllForwardings is on, which drops the forward rusty-mux needs",
            "remove ClearAllForwardings for this host from your ssh config",
        ));
    }
    if let Some(finding) = known_host(resolved, machine) {
        found.push(finding);
    }
    let keys = resolved
        .all("identityfile")
        .any(|f| expand(f, machine.home).is_file());
    if !keys && !machine.agent {
        found.push(Finding::warning(
            "no identity file exists and no agent is running, so only the first login can use a password; reconnects fail",
            "create a key (ssh-keygen) and install it (ssh-copy-id), or start ssh-agent",
        ));
    }
    if resolved.get("proxyjump").is_some() || resolved.get("proxycommand").is_some() {
        found.push(Finding::warning(
            "the host is reached through a proxy, but UDP goes straight to its hostname",
            "pass rusty-mux --udp-host with a name the client can reach, or RUSTY_MUX_UDP=off",
        ));
    }
    if cfg!(windows) {
        found.push(Finding::warning(
            "rusty-mux --ssh runs on Unix only for now",
            "run rusty-mux from a Unix machine, or reach the server through rusty-mux-ctl bridge",
        ));
    }
    found
}

/// A problem when no known-hosts file has the host's key, so a batch
/// reconnect cannot accept it.
fn known_host(resolved: &Resolved, machine: &Machine<'_>) -> Option<Finding> {
    if resolved.get("stricthostkeychecking") == Some("false") {
        return None;
    }
    let name = resolved
        .get("hostkeyalias")
        .or_else(|| resolved.get("hostname"))?;
    let lookup = match resolved.get("port") {
        Some("22") | None => name.to_owned(),
        Some(port) => format!("[{name}]:{port}"),
    };
    let files: Vec<PathBuf> = resolved
        .all("userknownhostsfile")
        .flat_map(str::split_whitespace)
        .map(|f| expand(f, machine.home))
        .collect();
    let mut answers = files.iter().map(|f| (machine.known)(f, &lookup));
    match answers.try_fold(false, |seen, a| a.map(|a| seen || a)) {
        Some(true) | None => None,
        Some(false) => Some(Finding::problem(
            format!(
                "no known-hosts file has {lookup}'s key, so a reconnect (BatchMode) cannot accept it"
            ),
            format!("log in once with ssh {lookup} and accept the key"),
        )),
    }
}

/// `path` with a leading `~` replaced by `home`.
fn expand(path: &str, home: Option<&Path>) -> PathBuf {
    match (path.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(path),
    }
}

/// The `Host` block `setup` prints, between markers so a later tool can
/// find it again.
pub(crate) fn block(setup: &SshSetup) -> String {
    let dest = &setup.dest;
    let mut lines = vec![
        format!("# BEGIN rusty-mux-ctl ssh setup {dest}"),
        format!("Host {dest}"),
    ];
    let values = [
        ("HostName", &setup.hostname),
        ("User", &setup.user),
        ("Port", &setup.port),
        ("IdentityFile", &setup.identity),
    ];
    for (key, value) in values {
        if let Some(value) = value {
            lines.push(format!("    {key} {value}"));
        }
    }
    if setup.identity.is_some() {
        lines.push("    IdentitiesOnly yes".to_owned());
    }
    lines.push(format!("# END rusty-mux-ctl ssh setup {dest}"));
    lines.join("\n")
}

/// Whether a `Host` line in `config`, or a file it includes by a plain
/// path, names `dest` without a wildcard.
pub(crate) fn has_host(config: &Path, dest: &str, depth: u8) -> bool {
    let Ok(text) = std::fs::read_to_string(config) else {
        return false;
    };
    let dir = config.parent().unwrap_or(Path::new("."));
    text.lines().map(str::trim).any(|line| {
        let mut words = line.split_whitespace();
        let key = words.next().unwrap_or("").to_ascii_lowercase();
        match key.as_str() {
            "host" => words.any(|w| w == dest),
            "include" if depth < 4 => words
                .filter(|w| !w.contains(['*', '?']))
                .any(|w| has_host(&included(dir, w), dest, depth + 1)),
            _ => false,
        }
    })
}

/// Where an `Include` of `word` in a file in `dir` (`~/.ssh`) points: `~`
/// is the home above it, and a relative path is in `~/.ssh`.
fn included(dir: &Path, word: &str) -> PathBuf {
    match word.strip_prefix("~/") {
        Some(rest) => dir.parent().unwrap_or(dir).join(rest),
        None => dir.join(word),
    }
}

/// Runs `ssh check|diagnose|setup` with `ssh` as the program and the
/// user's ssh config at `config`.
pub(crate) fn run(
    command: &SshCommand,
    ssh: &OsStr,
    config: Option<&Path>,
    home: Option<&Path>,
    out: &mut dyn Write,
) -> Result<Exit, (Exit, String)> {
    let fail = |e: io::Error| (Exit::Failed, e.to_string());
    let (dest, verbose) = match command {
        SshCommand::Check(dest) => (dest, false),
        SshCommand::Diagnose(dest) => (dest, true),
        SshCommand::Setup(setup) => return print_setup(setup, config, out).map_err(fail),
    };
    let version = version(ssh);
    let resolved = match resolve(ssh, dest) {
        Ok(resolved) => resolved,
        Err(reason) => {
            writeln!(out, "problem: {reason}").map_err(fail)?;
            writeln!(out, "  ssh -V: {version}").map_err(fail)?;
            writeln!(out, "  fix: rusty-mux --ssh needs OpenSSH 8.7 or later").map_err(fail)?;
            return Ok(Exit::Failed);
        }
    };
    let keygen = keygen(ssh);
    let known = |file: &Path, host: &str| known(&keygen, file, host);
    let machine = Machine {
        home,
        agent: std::env::var_os("SSH_AUTH_SOCK").is_some_and(|s| !s.is_empty()),
        known: &known,
    };
    let found = findings(&resolved, &machine);
    if verbose {
        writeln!(out, "{}", details(ssh, &version, config, dest, &resolved)).map_err(fail)?;
    }
    for finding in &found {
        let kind = if finding.problem {
            "problem"
        } else {
            "warning"
        };
        writeln!(out, "{kind}: {}", finding.text).map_err(fail)?;
        if verbose {
            writeln!(out, "  fix: {}", finding.fix).map_err(fail)?;
        }
    }
    if found.iter().any(|f| f.problem) {
        return Ok(Exit::Failed);
    }
    writeln!(out, "ok: rusty-mux --ssh {dest} should work").map_err(fail)?;
    Ok(Exit::Ok)
}

/// `setup`: the block to add, or that the config already has the host.
fn print_setup(setup: &SshSetup, config: Option<&Path>, out: &mut dyn Write) -> io::Result<Exit> {
    match config.filter(|c| has_host(c, &setup.dest, 0)) {
        Some(config) => writeln!(
            out,
            "{} already has a Host entry for {}; nothing to add",
            config.display(),
            setup.dest
        )?,
        None => writeln!(out, "{}", block(setup))?,
    }
    Ok(Exit::Ok)
}

/// What `diagnose` prints before its findings: every value they used.
fn details(
    ssh: &OsStr,
    version: &str,
    config: Option<&Path>,
    dest: &str,
    resolved: &Resolved,
) -> String {
    let mut lines = vec![
        format!("ssh: {} ({version})", ssh.to_string_lossy()),
        format!(
            "config: {}",
            config.map_or("-".to_owned(), |c| c.display().to_string())
        ),
    ];
    for key in [
        "hostname",
        "user",
        "port",
        "identityfile",
        "proxyjump",
        "proxycommand",
        "userknownhostsfile",
    ] {
        let values: Vec<&str> = resolved.all(key).collect();
        let shown = if values.is_empty() {
            "-".to_owned()
        } else {
            values.join(", ")
        };
        lines.push(format!("{key}: {shown}"));
    }
    let options: Vec<String> = OWNED.iter().map(|o| format!("-o {o}")).collect();
    lines.push(format!(
        "rusty-mux runs: {} -N -T {} -L LOCAL:REMOTE -- {dest}",
        ssh.to_string_lossy(),
        options.join(" ")
    ));
    lines.join("\n")
}

/// `ssh -G` for `dest` with [`OPTIONS`]; connects to nothing.
fn resolve(ssh: &OsStr, dest: &str) -> Result<Resolved, String> {
    let mut command = Command::new(ssh);
    command.arg("-G");
    for option in OPTIONS {
        command.args(["-o", option]);
    }
    let output = command
        .args(["--", dest])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run {}: {e}", ssh.to_string_lossy()))?;
    if !output.status.success() {
        let said = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ssh -G {dest} failed: {}", said.trim()));
    }
    Ok(Resolved::parse(&String::from_utf8_lossy(&output.stdout)))
}

/// `ssh -V`'s line, or why it could not be read.
fn version(ssh: &OsStr) -> String {
    match Command::new(ssh).arg("-V").stdin(Stdio::null()).output() {
        Ok(out) => String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        Err(e) => format!("unknown: {e}"),
    }
}

/// `ssh-keygen` beside `ssh` when `ssh` is a path, else the one on `PATH`.
fn keygen(ssh: &OsStr) -> OsString {
    let path = Path::new(ssh);
    let beside = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| {
            p.join(if cfg!(windows) {
                "ssh-keygen.exe"
            } else {
                "ssh-keygen"
            })
        });
    beside
        .filter(|p| p.is_file())
        .map_or_else(|| "ssh-keygen".into(), PathBuf::into_os_string)
}

/// Whether `file` holds `host`'s key (`ssh-keygen -F`, which only reads):
/// `None` when that cannot be told.
fn known(keygen: &OsStr, file: &Path, host: &str) -> Option<bool> {
    if !file.is_file() {
        return Some(false);
    }
    let status = Command::new(keygen)
        .args([
            OsStr::new("-F"),
            OsStr::new(host),
            OsStr::new("-f"),
            file.as_os_str(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;
    Some(status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "user me\nhostname box.example\nport 22\nidentityfile ~/.ssh/id_ed25519\n\
        clearallforwardings no\nstricthostkeychecking ask\n\
        userknownhostsfile ~/.ssh/known_hosts ~/.ssh/known_hosts2\n";

    fn check(text: &str, home: Option<&Path>, agent: bool, known: Option<bool>) -> Vec<Finding> {
        let answer = move |_: &Path, _: &str| known;
        let machine = Machine {
            home,
            agent,
            known: &answer,
        };
        findings(&Resolved::parse(text), &machine)
    }

    fn problems(found: &[Finding]) -> Vec<&str> {
        found
            .iter()
            .filter(|f| f.problem)
            .map(|f| f.text.as_str())
            .collect()
    }

    #[test]
    fn a_known_host_with_an_agent_has_no_problem() {
        assert!(problems(&check(GOOD, None, true, Some(true))).is_empty());
        // ssh-keygen missing: the lookup is skipped, not failed.
        assert!(problems(&check(GOOD, None, true, None)).is_empty());
    }

    #[test]
    fn an_unknown_host_key_is_a_problem_named_with_its_port() {
        let found = check(GOOD, None, true, Some(false));
        assert_eq!(problems(&found).len(), 1);
        assert!(
            found[0].text.contains("box.example's key"),
            "{:?}",
            found[0]
        );
        let odd = GOOD.replace("port 22", "port 2222");
        let found = check(&odd, None, true, Some(false));
        assert!(
            found[0].text.contains("[box.example]:2222"),
            "{:?}",
            found[0]
        );
        let off = GOOD.replace("stricthostkeychecking ask", "stricthostkeychecking false");
        assert!(problems(&check(&off, None, true, Some(false))).is_empty());
    }

    #[test]
    fn cleared_forwardings_are_a_problem() {
        let text = GOOD.replace("clearallforwardings no", "clearallforwardings yes");
        assert_eq!(problems(&check(&text, None, true, Some(true))).len(), 1);
    }

    #[test]
    fn no_key_and_no_agent_and_a_proxy_are_warnings() {
        let text = format!("{GOOD}proxyjump bastion\n");
        let found = check(
            &text,
            Some(Path::new("/nonexistent-home")),
            false,
            Some(true),
        );
        assert!(problems(&found).is_empty());
        let warnings: Vec<&str> = found
            .iter()
            .filter(|f| !f.problem)
            .map(|f| f.text.as_str())
            .collect();
        assert!(
            warnings.iter().any(|w| w.contains("no identity file")),
            "{warnings:?}"
        );
        assert!(warnings.iter().any(|w| w.contains("proxy")), "{warnings:?}");
    }

    #[test]
    fn the_block_holds_only_what_was_given() {
        let setup = SshSetup {
            dest: "box".into(),
            hostname: Some("box.example".into()),
            user: None,
            port: None,
            identity: Some("~/.ssh/box".into()),
        };
        assert_eq!(
            block(&setup),
            "# BEGIN rusty-mux-ctl ssh setup box\nHost box\n    HostName box.example\n    \
             IdentityFile ~/.ssh/box\n    IdentitiesOnly yes\n# END rusty-mux-ctl ssh setup box"
        );
    }

    #[test]
    fn an_existing_host_is_found_through_an_include() {
        let dir = std::env::temp_dir().join(format!("rusty-mux-ssh-conf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(dir.join("config"), "Include extra\nHost *\n  User x\n").expect("write");
        std::fs::write(dir.join("extra"), "Host other box\n  HostName b\n").expect("write");
        assert!(has_host(&dir.join("config"), "box", 0));
        assert!(!has_host(&dir.join("config"), "bo", 0));
        assert!(!has_host(&dir.join("missing"), "box", 0));
        let _gone = std::fs::remove_dir_all(dir);
    }
}
