//! `hosts pull DEST` (C49b, term pack D202-D205): another machine's saved
//! hosts merged into this one's, per name (see
//! [`rusty_mux_script::hosts::merge`]).
//!
//! One `ssh -- DEST sh -c '<script>'` reads the remote `hosts.json` with
//! POSIX tools only, so the remote needs no rusty-mux-ctl. The script is
//! fixed and holds no single quote, so no user text reaches the remote
//! shell. Only this machine's `hosts.json` is written; neither config.json.

use std::ffi::OsString;
use std::io::Write;
use std::process::{Command, Stdio};

use rusty_json::{Map, Value};
use rusty_mux_script::hosts::{self, Merged, Outcome};

use crate::cli::Sync;
use crate::terminfo::{ended, started};
use crate::{Env, Exit};

/// Marks the script's own lines, so rc-file noise on the remote is skipped.
const MARK: &str = "rusty-mux-hosts:";

/// Prints the remote `hosts.json` path, found as `hosts_path` finds it,
/// then `file` and its text, or `none`.
const READ: &str = concat!(
    "m=rusty-mux-hosts:; case \"$XDG_CONFIG_HOME\" in /*) b=\"$XDG_CONFIG_HOME\";; ",
    "*) [ -n \"$HOME\" ] || { echo \"HOME is not set\" >&2; exit 1; }; b=\"$HOME/.config\";; esac; ",
    "p=\"$b/rusty_mux/hosts.json\"; echo \"${m}path $p\"; ",
    "if [ -f \"$p\" ]; then echo \"${m}file\"; cat \"$p\"; else echo \"${m}none\"; fi"
);

/// The remote `hosts.json`: its path, and its text unless it is missing.
#[derive(Debug, PartialEq, Eq)]
struct Remote {
    path: String,
    text: Option<String>,
}

/// Reads `sync.dest`'s saved hosts and merges them into this machine's,
/// printing one line per host that changed or was turned away. With
/// `--dry-run`, or when nothing changes, nothing is written. A local
/// config whose hosts do not parse refuses, since a declared name could
/// not be kept out (D200).
pub(crate) fn pull(
    sync: &Sync,
    env: &Env,
    json: bool,
    out: &mut dyn Write,
) -> Result<(), (Exit, String)> {
    let failed = |e: String| (Exit::Failed, e);
    let path = env
        .hosts
        .as_deref()
        .ok_or_else(|| failed("no hosts file: set XDG_CONFIG_HOME or HOME".to_owned()))?;
    let declared = crate::hosts::load_declared(env.config.as_deref()).map_err(|e| {
        failed(format!(
            "can't check the config's hosts, so nothing was pulled: {e}"
        ))
    })?;
    let ssh = env.ssh.clone().unwrap_or_else(|| "ssh".into());
    let remote = read_remote(&ssh, &sync.dest)?;
    let sender = match &remote.text {
        None => Vec::new(),
        Some(text) => hosts::parse_hosts(text).map_err(|problems| {
            let lines: Vec<String> = problems
                .iter()
                .map(|p| format!("{}:{}: {p}", sync.dest, remote.path))
                .collect();
            failed(lines.join("\n"))
        })?,
    };
    let saved = crate::hosts::load(path).map_err(failed)?;
    let merged = hosts::merge(&saved, &sender, &declared, sync.replace);
    let text = report(&merged, json);
    if !text.is_empty() {
        writeln!(out, "{text}").map_err(|e| failed(e.to_string()))?;
    }
    if sync.dry_run || merged.hosts == saved {
        return Ok(());
    }
    crate::hosts::save(path, &merged.hosts).map_err(failed)
}

/// The command line `ssh` gets to read `dest`'s hosts.
fn ssh_args(dest: &str) -> [String; 3] {
    ["--".to_owned(), dest.to_owned(), format!("sh -c '{READ}'")]
}

fn read_remote(ssh: &OsString, dest: &str) -> Result<Remote, (Exit, String)> {
    let output = Command::new(ssh)
        .args(ssh_args(dest))
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| started(ssh, e))?;
    ended(output.status.code(), dest)?;
    parse_remote(&String::from_utf8_lossy(&output.stdout)).ok_or_else(|| {
        (
            Exit::Failed,
            format!("{dest} did not run the sync script; it needs a POSIX sh"),
        )
    })
}

/// The script's answer in `text`, past any rc-file noise before it.
fn parse_remote(text: &str) -> Option<Remote> {
    let (_, after) = text.split_once(&format!("{MARK}path "))?;
    let (path, rest) = after.split_once('\n')?;
    let (_, rest) = rest.split_once(MARK)?;
    let text = match rest.split_once('\n') {
        Some(("file", body)) => Some(body.to_owned()),
        _ if rest.trim_end() == "none" => None,
        _ => return None,
    };
    Some(Remote {
        path: path.to_owned(),
        text,
    })
}

/// One line per reported host, or a JSON array of them.
fn report(merged: &Merged, json: bool) -> String {
    if json {
        let all: Vec<Value> = merged
            .report
            .iter()
            .map(|(name, outcome)| entry(name, outcome))
            .collect();
        return Value::from(all).to_json_string();
    }
    let lines: Vec<String> = merged
        .report
        .iter()
        .map(|(name, outcome)| match outcome {
            Outcome::Added => format!("added    {name}"),
            Outcome::Kept => format!("kept     {name} (differs; --replace takes theirs)"),
            Outcome::Replaced => format!("replaced {name}"),
            Outcome::Skipped(why) => format!("skipped  {name}: {why}"),
        })
        .collect();
    lines.join("\n")
}

fn entry(name: &str, outcome: &Outcome) -> Value {
    let (word, reason) = match outcome {
        Outcome::Added => ("added", None),
        Outcome::Kept => ("kept", None),
        Outcome::Replaced => ("replaced", None),
        Outcome::Skipped(why) => ("skipped", Some(*why)),
    };
    let mut map = Map::new();
    map.insert("name".into(), name.into());
    map.insert("outcome".into(), word.into());
    if let Some(reason) = reason {
        map.insert("reason".into(), reason.into());
    }
    map.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_script_holds_no_single_quote() {
        assert!(!READ.contains('\''));
        assert_eq!(ssh_args("me@box")[..2], ["--", "me@box"]);
    }

    #[test]
    fn the_answer_is_found_past_rc_noise() {
        let file = "motd\nnoise rusty-mux-hosts:path /h/.config/rusty_mux/hosts.json\nrusty-mux-hosts:file\n{\"hosts\": []}\n";
        assert_eq!(
            parse_remote(file),
            Some(Remote {
                path: "/h/.config/rusty_mux/hosts.json".into(),
                text: Some("{\"hosts\": []}\n".into()),
            })
        );
        let none = "rusty-mux-hosts:path /p\nrusty-mux-hosts:none\n";
        assert_eq!(parse_remote(none).map(|r| r.text), Some(None));
        assert_eq!(parse_remote("just a shell banner\n"), None);
        assert_eq!(
            parse_remote("rusty-mux-hosts:path /p\nrusty-mux-hosts:what\n"),
            None
        );
    }
}
