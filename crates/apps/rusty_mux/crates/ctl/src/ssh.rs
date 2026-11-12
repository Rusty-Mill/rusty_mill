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
    /// Whether `rusty-mux --ssh` goes through `rusty-mux-ctl bridge`, one
    /// login per connection, with no forward and no UDP: Windows (C79).
    pub(crate) bridged: bool,
}

/// The findings for a destination whose `ssh -G` resolved to `resolved`.
pub(crate) fn findings(resolved: &Resolved, machine: &Machine<'_>) -> Vec<Finding> {
    let mut found = Vec::new();
    if !machine.bridged && resolved.get("clearallforwardings") == Some("yes") {
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
            match machine.bridged {
                false => "no identity file exists and no agent is running, so only the first login can use a password; reconnects fail",
                true => "no identity file exists and no agent is running, so only the first login can use a password; every connection after it fails",
            },
            "create a key (ssh-keygen) and install it (ssh-copy-id), or start ssh-agent",
        ));
    }
    if !machine.bridged
        && (resolved.get("proxyjump").is_some() || resolved.get("proxycommand").is_some())
    {
        found.push(Finding::warning(
            "the host is reached through a proxy, but UDP goes straight to its hostname",
            "pass rusty-mux --udp-host with a name the client can reach, or RUSTY_MUX_UDP=off",
        ));
    }
    found
}

/// A problem when no known-hosts file has the host's key, so a batch
/// reconnect cannot accept it.
fn known_host(resolved: &Resolved, machine: &Machine<'_>) -> Option<Finding> {
    // `accept-new` takes an unknown key on its own; `false` takes any.
    if matches!(
        resolved.get("stricthostkeychecking"),
        Some("false" | "no" | "off" | "accept-new")
    ) {
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
        .chain(resolved.all("globalknownhostsfile"))
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

/// How deep `Include`s nest before ssh itself gives up (OpenSSH's
/// `READCONF_MAX_DEPTH`).
const INCLUDE_DEPTH: u8 = 16;

/// Whether `config`, or a file it includes, names `dest` in a `Host` line
/// or a `Match host`/`originalhost` list, without a wildcard, ignoring
/// ASCII case as ssh does. `Include`
/// patterns are expanded the way ssh does for `*` and `?` in the file name.
/// `ssh_dir` is `~/.ssh`, which every relative `Include` is read against,
/// in nested files too. A missing file names nothing, as for ssh; a file
/// that is there but can't be read is an error, since it may name `dest`.
pub(crate) fn has_host(
    config: &Path,
    ssh_dir: &Path,
    dest: &str,
    skip: Option<&Path>,
    depth: u8,
) -> Result<bool, String> {
    if skip == Some(config) {
        return Ok(false);
    }
    if depth > INCLUDE_DEPTH {
        return Err(format!("{}: Includes nest too deep", config.display()));
    }
    let text = match std::fs::read_to_string(config) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(format!("{}: {e}", config.display())),
    };
    for line in text.lines().map(str::trim) {
        // `Key value`, `Key=value` and `Key = value` all name a key.
        let (key, value) = match line.split_once(|c: char| c == '=' || c.is_whitespace()) {
            Some((key, value)) => (key, value.trim_start_matches(['=', ' ', '\t'])),
            None => (line, ""),
        };
        let words = || value.split_whitespace().map(|w| w.trim_matches('"'));
        let found = match key.to_ascii_lowercase().as_str() {
            "host" => words().any(|w| w.eq_ignore_ascii_case(dest)),
            "match" => matched_hosts(words()).any(|w| w.eq_ignore_ascii_case(dest)),
            "include" => {
                for word in words() {
                    for file in included(ssh_dir, word)? {
                        if has_host(&file, ssh_dir, dest, skip, depth + 1)? {
                            return Ok(true);
                        }
                    }
                }
                false
            }
            _ => false,
        };
        if found {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The names in a `Match` line's `host` and `originalhost` lists.
fn matched_hosts<'a>(words: impl Iterator<Item = &'a str>) -> impl Iterator<Item = &'a str> {
    let mut words = words.peekable();
    std::iter::from_fn(move || {
        while let Some(word) = words.next() {
            let criterion = word.trim_start_matches('!').to_ascii_lowercase();
            if matches!(criterion.as_str(), "host" | "originalhost") {
                return words.next();
            }
        }
        None
    })
    .flat_map(|list| list.split(','))
}

/// The files an `Include` of `word` reads: `~/` (or `~\`) is the home
/// above `dir` (`~/.ssh`), and a relative path is in `~/.ssh`. `*` and `?`
/// in the file name match as in glob(3), a leading `.` only when the
/// pattern starts with one; anywhere else, or `[`, or another user's
/// `~user/`, they are refused rather than guessed at.
fn included(dir: &Path, word: &str) -> Result<Vec<PathBuf>, String> {
    let home = word.strip_prefix("~/").or_else(|| word.strip_prefix("~\\"));
    let path = match home {
        Some(rest) => dir.parent().unwrap_or(dir).join(rest),
        None if word.starts_with('~') => {
            return Err(format!("`Include {word}`: can't check another user's home"));
        }
        None => dir.join(word),
    };
    let wild = |s: &str| s.contains(['*', '?', '[']);
    if !wild(word) {
        return Ok(vec![path]);
    }
    let pattern = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let parent = path.parent().unwrap_or(dir);
    if !wild(&pattern) || pattern.contains('[') || wild(&parent.to_string_lossy()) {
        return Err(format!("`Include {word}`: can't check that pattern"));
    }
    let entries = match std::fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", parent.display())),
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", parent.display()))?;
        let file = entry.file_name().to_string_lossy().into_owned();
        let hidden = file.starts_with('.') && !pattern.starts_with('.');
        if !hidden && glob(&pattern, &file) {
            files.push(entry.path());
        }
    }
    files.sort();
    Ok(files)
}

/// Whether `name` matches `pattern`, where `*` is any run and `?` any one
/// character.
fn glob(pattern: &str, name: &str) -> bool {
    let (p, n): (Vec<char>, Vec<char>) = (pattern.chars().collect(), name.chars().collect());
    let (mut i, mut j) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while j < n.len() {
        match p.get(i) {
            Some('*') => {
                star = Some((i, j));
                i += 1;
            }
            Some(&c) if c == '?' || c == n[j] => {
                i += 1;
                j += 1;
            }
            _ => match star {
                Some((si, sj)) => {
                    i = si + 1;
                    j = sj + 1;
                    star = Some((si, sj + 1));
                }
                None => return false,
            },
        }
    }
    p[i..].iter().all(|&c| c == '*')
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
        SshCommand::Setup(setup) if setup.write => return write_setup(setup, ssh, config, out),
        SshCommand::Setup(setup) => return print_setup(setup, config, out).map_err(fail),
        SshCommand::Remove { dest, write } => return remove(dest, *write, config, out),
    };
    let version = version(ssh);
    let resolved = match resolve(ssh, dest) {
        Ok(resolved) => resolved,
        Err(reason) => {
            writeln!(out, "problem: {reason}").map_err(fail)?;
            writeln!(out, "  ssh -V: {version}").map_err(fail)?;
            writeln!(out, "  fix: {}", resolve_fix(&reason)).map_err(fail)?;
            return Ok(Exit::Failed);
        }
    };
    let keygen = keygen(ssh);
    let known = |file: &Path, host: &str| known(&keygen, file, host);
    let machine = Machine {
        home,
        agent: std::env::var_os("SSH_AUTH_SOCK").is_some_and(|s| !s.is_empty()),
        known: &known,
        bridged: cfg!(windows),
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

/// The fix for an `ssh -G` failure: only an option ssh does not know
/// points at an old OpenSSH.
fn resolve_fix(reason: &str) -> &'static str {
    let unknown = [
        "Bad configuration option",
        "unsupported option",
        "forkafterauthentication",
    ];
    match unknown.iter().any(|u| {
        reason
            .to_ascii_lowercase()
            .contains(&u.to_ascii_lowercase())
    }) {
        true => "rusty-mux --ssh needs OpenSSH 8.7 or later",
        false => "fix what ssh reports above (a missing ssh, or an error in the config)",
    }
}

/// The ssh folder `config` sits in.
fn ssh_dir(config: Option<&Path>) -> Result<&Path, (Exit, String)> {
    config.and_then(Path::parent).ok_or((
        Exit::Failed,
        "no home folder: set HOME (USERPROFILE on Windows)".to_owned(),
    ))
}

/// `setup --write`: the block into our file, read through one Include line.
fn write_setup(
    setup: &SshSetup,
    ssh: &OsStr,
    config: Option<&Path>,
    out: &mut dyn Write,
) -> Result<Exit, (Exit, String)> {
    let dir = ssh_dir(config)?;
    let files = crate::ssh_write::Files::under(dir);
    crate::ssh_write::alias(&setup.dest).map_err(|e| (Exit::Usage, e))?;
    // Our Include comes first, so our block would win over theirs.
    let has = has_host(&files.config, dir, &setup.dest, Some(&files.ours), 0).map_err(|e| {
        (
            Exit::Failed,
            format!(
                "can't tell whether your ssh config already has {}, so nothing was written: {e}",
                setup.dest
            ),
        )
    })?;
    if has {
        return Err((
            Exit::Failed,
            format!(
                "{} already has a Host entry for {}; edit that one, or pick another name",
                files.config.display(),
                setup.dest
            ),
        ));
    }
    let done = crate::ssh_write::setup(&files, &setup.dest, &block(setup), ssh)
        .map_err(|e| (Exit::Failed, e))?;
    let what = format!("{} in {}", setup.dest, files.ours.display());
    written(&done, &what, &files, out)
}

/// `remove`: the block out of our file, or what that would do.
fn remove(
    dest: &str,
    write: bool,
    config: Option<&Path>,
    out: &mut dyn Write,
) -> Result<Exit, (Exit, String)> {
    let files = crate::ssh_write::Files::under(ssh_dir(config)?);
    let fail = |e: io::Error| (Exit::Failed, e.to_string());
    if !write {
        writeln!(
            out,
            "would take {dest}'s block out of {}, and the Include line out of {} once no block is left; add --write to do it",
            files.ours.display(),
            files.config.display()
        )
        .map_err(fail)?;
        return Ok(Exit::Ok);
    }
    let done = crate::ssh_write::remove(&files, dest).map_err(|e| (Exit::Failed, e))?;
    let what = format!("{dest} removed from {}", files.ours.display());
    written(&done, &what, &files, out)
}

fn written(
    done: &crate::ssh_write::Done,
    what: &str,
    files: &crate::ssh_write::Files,
    out: &mut dyn Write,
) -> Result<Exit, (Exit, String)> {
    let fail = |e: io::Error| (Exit::Failed, e.to_string());
    match done {
        crate::ssh_write::Done::Nothing => writeln!(out, "nothing to change: {what}"),
        crate::ssh_write::Done::Changed { backup: None } => writeln!(out, "done: {what}"),
        crate::ssh_write::Done::Changed { backup: Some(b) } => writeln!(
            out,
            "done: {what}; {} was first copied to {}",
            files.config.display(),
            b.display()
        ),
    }
    .map_err(fail)?;
    Ok(Exit::Ok)
}

/// `setup`: the block to add, or that the config already has the host.
fn print_setup(setup: &SshSetup, config: Option<&Path>, out: &mut dyn Write) -> io::Result<Exit> {
    let dir = config.and_then(Path::parent).unwrap_or(Path::new("."));
    let found = config.map(|c| (c, has_host(c, dir, &setup.dest, None, 0)));
    match found {
        Some((config, Ok(true))) => writeln!(
            out,
            "{} already has a Host entry for {}; nothing to add",
            config.display(),
            setup.dest
        )?,
        Some((_, Err(e))) => writeln!(
            out,
            "# Check your config has no Host entry for {} first: {e}\n{}",
            setup.dest,
            block(setup)
        )?,
        _ => writeln!(out, "{}", block(setup))?,
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
        check_on(text, home, agent, known, false)
    }

    fn check_on(
        text: &str,
        home: Option<&Path>,
        agent: bool,
        known: Option<bool>,
        bridged: bool,
    ) -> Vec<Finding> {
        let answer = move |_: &Path, _: &str| known;
        let machine = Machine {
            home,
            agent,
            known: &answer,
            bridged,
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

    /// Through the bridge, no forward is cleared and no UDP goes past a
    /// proxy, but every connection logs in.
    #[test]
    fn a_bridged_check_skips_the_forward_and_udp() {
        let text = format!("{GOOD}proxyjump bastion\n")
            .replace("clearallforwardings no", "clearallforwardings yes");
        let home = Some(Path::new("/nonexistent-home"));
        let found = check_on(&text, home, false, Some(true), true);
        assert!(problems(&found).is_empty(), "{found:?}");
        let texts: Vec<&str> = found.iter().map(|f| f.text.as_str()).collect();
        assert_eq!(texts.len(), 1, "{texts:?}");
        assert!(texts[0].contains("every connection after it"), "{texts:?}");
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
    fn write_refuses_a_pattern_and_a_host_the_user_already_has() {
        let dir = std::env::temp_dir().join(format!("rusty-mux-ssh-own-{}", std::process::id()));
        let _fresh = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("made");
        let config = dir.join("config");
        std::fs::write(&config, "Host box\n    User me\n").expect("written");
        let setup = |dest: &str| SshSetup {
            dest: dest.into(),
            hostname: None,
            user: Some("root".into()),
            port: None,
            identity: None,
            write: true,
        };
        let mut out = Vec::new();
        let ssh = OsStr::new("/nonexistent/ssh");
        let theirs = write_setup(&setup("box"), ssh, Some(&config), &mut out);
        assert!(matches!(theirs, Err((Exit::Failed, ref e)) if e.contains("already has")));
        let pattern = write_setup(&setup("*"), ssh, Some(&config), &mut out);
        assert!(matches!(pattern, Err((Exit::Usage, _))));
        assert_eq!(
            std::fs::read_to_string(&config).expect("read"),
            "Host box\n    User me\n"
        );
        assert!(!dir.join("rusty_mux").exists(), "nothing written");
    }

    #[test]
    fn the_block_holds_only_what_was_given() {
        let setup = SshSetup {
            dest: "box".into(),
            hostname: Some("box.example".into()),
            user: None,
            port: None,
            identity: Some("~/.ssh/box".into()),
            write: false,
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
        assert_eq!(
            has_host(&dir.join("config"), &dir, "box", None, 0),
            Ok(true)
        );
        assert_eq!(
            has_host(&dir.join("config"), &dir, "bo", None, 0),
            Ok(false)
        );
        assert_eq!(
            has_host(&dir.join("missing"), &dir, "box", None, 0),
            Ok(false)
        );
        let _gone = std::fs::remove_dir_all(dir);
    }

    fn conf_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rusty-mux-ssh-{name}-{}", std::process::id()));
        let _fresh = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("config.d")).expect("dir");
        dir
    }

    #[test]
    fn a_host_is_found_through_a_wildcard_include_and_a_match() {
        let dir = conf_dir("glob");
        let config = dir.join("config");
        std::fs::write(&config, "Include config.d/*.conf\n").expect("write");
        std::fs::write(dir.join("config.d/work.conf"), "Host box\n").expect("write");
        std::fs::write(dir.join("config.d/.hidden.conf"), "Host secret\n").expect("write");
        std::fs::write(dir.join("config.d/notes.txt"), "Host txt\n").expect("write");
        assert_eq!(has_host(&config, &dir, "box", None, 0), Ok(true));
        assert_eq!(
            has_host(&config, &dir, "secret", None, 0),
            Ok(false),
            "no leading dot"
        );
        assert_eq!(
            has_host(&config, &dir, "txt", None, 0),
            Ok(false),
            "*.conf only"
        );
        std::fs::write(&config, "Match user me originalhost a,box exec true\n").expect("write");
        assert_eq!(has_host(&config, &dir, "box", None, 0), Ok(true));
        std::fs::write(&config, "Match !host box\n").expect("write");
        assert_eq!(
            has_host(&config, &dir, "box", None, 0),
            Ok(true),
            "named, even negated"
        );
        std::fs::write(&config, "Match exec box\n").expect("write");
        assert_eq!(has_host(&config, &dir, "box", None, 0), Ok(false));
        std::fs::write(&config, "Host BOX\nMatch host Lab\n").expect("write");
        assert_eq!(
            has_host(&config, &dir, "box", None, 0),
            Ok(true),
            "any case"
        );
        assert_eq!(
            has_host(&config, &dir, "lab", None, 0),
            Ok(true),
            "any case"
        );
        std::fs::write(&config, "Include config.d/.h*\n").expect("write");
        assert_eq!(
            has_host(&config, &dir, "secret", None, 0),
            Ok(true),
            "a dot pattern matches dot files"
        );
        let _gone = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_include_that_cannot_be_checked_is_an_error() {
        let dir = conf_dir("unreadable");
        let config = dir.join("config");
        std::fs::write(dir.join("config.d/bad"), [0xff, 0xfe]).expect("write");
        std::fs::write(&config, "Include config.d/bad\n").expect("write");
        assert!(
            has_host(&config, &dir, "box", None, 0).is_err(),
            "not UTF-8"
        );
        std::fs::write(&config, "Include */x\n").expect("write");
        assert!(
            has_host(&config, &dir, "box", None, 0).is_err(),
            "a pattern in a folder"
        );
        std::fs::write(&config, "Include config\n").expect("write");
        assert!(
            has_host(&config, &dir, "box", None, 0).is_err(),
            "itself, forever"
        );
        std::fs::write(&config, "Include ~other/.ssh/x\n").expect("write");
        assert!(
            has_host(&config, &dir, "box", None, 0).is_err(),
            "another user's home"
        );
        std::fs::write(&config, "Include config.d/missing\nInclude nothing/*\n").expect("write");
        assert_eq!(
            has_host(&config, &dir, "box", None, 0),
            Ok(false),
            "missing is nothing"
        );
        let _gone = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn glob_matches_stars_and_question_marks() {
        for (pattern, name, want) in [
            ("*", "a", true),
            ("*.conf", "x.conf", true),
            ("*.conf", "x.confs", false),
            ("a?c", "abc", true),
            ("a?c", "ac", false),
            ("a*b*c", "aXbYbZc", true),
            ("a*b", "aXbY", false),
        ] {
            assert_eq!(glob(pattern, name), want, "{pattern} {name}");
        }
    }
}
