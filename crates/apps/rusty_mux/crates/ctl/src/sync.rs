//! `hosts pull|push DEST` (C49b, term pack D202-D205, D208): saved hosts
//! merged between this machine and another, per name (see
//! [`rusty_mux_script::hosts::merge`]).
//!
//! One `ssh -- DEST sh -c '<script>'` reads the remote's `hosts.json` and
//! config.json with POSIX tools only, so the remote needs no
//! rusty-mux-ctl. Push then writes the remote's `hosts.json` with a second
//! call. The scripts are fixed and hold no single quote, so no user text
//! reaches the remote shell; push's only arguments are the checksum's
//! digits. Only the receiving side's `hosts.json` is written; never a
//! config.json.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};

use rusty_json::{Map, Value};
use rusty_mux_script::hosts::{self, Declared, Host, Merged, Outcome};

use crate::cli::{Sync, Way};
use crate::terminfo::{ended, started};
use crate::{Env, Exit};

/// Marks the script's own lines, so rc-file noise on the remote is skipped.
const MARK: &str = "rusty-mux-hosts:";

/// Script text setting `b` to the remote's config folder as `hosts_path`
/// finds it: an absolute `$XDG_CONFIG_HOME`, else `$HOME/.config` with an
/// absolute HOME. A macro so both scripts can `concat!` it.
macro_rules! base {
    () => {
        concat!(
            "case \"$XDG_CONFIG_HOME\" in /*) b=\"$XDG_CONFIG_HOME\";; ",
            "*) case \"$HOME\" in /*) b=\"$HOME/.config\";; ",
            "*) echo \"HOME is not an absolute path\" >&2; exit 1;; esac;; esac; ",
        )
    };
}

/// Prints, for `hosts` then `config`, a marker line with the file's
/// `cksum` (or `none`) and its path, then its bytes and a newline.
const READ: &str = concat!(
    "m=rusty-mux-hosts:; ",
    base!(),
    "f() { if [ -f \"$2\" ]; then k=$(cksum < \"$2\") || exit 1; ",
    "echo \"${m}$1 $k $2\"; cat \"$2\" || exit 1; echo; ",
    "else echo \"${m}$1 none $2\"; fi; }; ",
    "f hosts \"$b/rusty_mux/hosts.json\"; ",
    "f config \"${RUSTY_MUX_CONFIG:-$b/rusty_mux/config.json}\""
);

/// Writes stdin to the remote `hosts.json` through a private temp file in
/// its folder, then renames it over the file, unless the file's `cksum` is
/// no longer the arguments (`none` for no file); then it exits [`CHANGED`].
const WRITE: &str = concat!(
    base!(),
    "d=\"$b/rusty_mux\"; p=\"$d/hosts.json\"; umask 077; mkdir -p \"$d\" || exit 1; ",
    "t=$(mktemp \"$d/.hosts.json.XXXXXX\") || exit 1; ",
    "cat > \"$t\" || { rm -f \"$t\"; exit 1; }; ",
    "if [ -f \"$p\" ]; then k=$(cksum < \"$p\") || { rm -f \"$t\"; exit 1; }; else k=none; fi; ",
    "[ \"$k\" = \"$*\" ] || { rm -f \"$t\"; exit 3; }; ",
    "mv -f \"$t\" \"$p\" || { rm -f \"$t\"; exit 1; }"
);

/// The write script's exit when the file changed after it was read.
const CHANGED: i32 = 3;

/// The most the remote may print. Two real files are a few KiB; more
/// means something other than the script is talking.
const MAX_ANSWER: u64 = 1 << 20;

/// A file's `cksum` output: its CRC and its size in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Sum {
    crc: u32,
    size: usize,
}

/// One remote file: its path, and its checksum and text unless missing.
#[derive(Debug, PartialEq, Eq)]
struct File {
    path: String,
    body: Option<(Sum, String)>,
}

/// What the read script found on the remote.
#[derive(Debug, PartialEq, Eq)]
struct Answer {
    hosts: File,
    config: File,
}

/// Runs a pull or a push and prints one line per host that changed or was
/// turned away. With `--dry-run`, or when nothing changes, nothing is
/// written. A receiving config whose hosts do not parse refuses, since a
/// declared name could not be kept out (D200).
pub(crate) fn run(
    sync: &Sync,
    env: &Env,
    json: bool,
    out: &mut dyn Write,
) -> Result<(), (Exit, String)> {
    let path = env
        .hosts
        .as_deref()
        .ok_or_else(|| failed("no hosts file: set XDG_CONFIG_HOME or HOME".to_owned()))?;
    let ssh = env.ssh.clone().unwrap_or_else(|| "ssh".into());
    let answer = read_remote(&ssh, &sync.dest)?;
    let mine = crate::hosts::load(path).map_err(failed)?;
    let merged = match sync.way {
        Way::Pull => {
            let declared = crate::hosts::load_declared(env.config.as_deref()).map_err(|e| {
                failed(format!(
                    "can't check the config's hosts, so nothing was pulled: {e}"
                ))
            })?;
            let theirs = remote_hosts(&sync.dest, &answer.hosts)?;
            let merged = hosts::merge(&mine, &theirs, &declared, sync.replace);
            if !sync.dry_run && merged.hosts != mine {
                crate::hosts::save(path, &merged.hosts).map_err(failed)?;
            }
            merged
        }
        Way::Push => {
            let declared = remote_declared(&sync.dest, &answer.config)?;
            let theirs = remote_hosts(&sync.dest, &answer.hosts)?;
            let merged = hosts::merge(&theirs, &mine, &declared, sync.replace);
            if !sync.dry_run && merged.hosts != theirs {
                let sum = answer.hosts.body.as_ref().map(|(sum, _)| *sum);
                write_remote(&ssh, &sync.dest, &merged.hosts, sum)?;
            }
            merged
        }
    };
    let text = report(&merged, sync.way, json);
    if !text.is_empty() {
        writeln!(out, "{text}").map_err(|e| failed(e.to_string()))?;
    }
    Ok(())
}

fn failed(reason: String) -> (Exit, String) {
    (Exit::Failed, reason)
}

/// The remote's saved hosts; none when it has no `hosts.json`.
fn remote_hosts(dest: &str, file: &File) -> Result<Vec<Host>, (Exit, String)> {
    let Some((_, text)) = &file.body else {
        return Ok(Vec::new());
    };
    hosts::parse_hosts(text).map_err(|problems| {
        let lines: Vec<String> = problems
            .iter()
            .map(|p| format!("{dest}:{}: {p}", file.path))
            .collect();
        failed(lines.join("\n"))
    })
}

/// The names the remote's config.json declares, which push never writes.
fn remote_declared(dest: &str, file: &File) -> Result<Declared, (Exit, String)> {
    let Some((_, text)) = &file.body else {
        return Ok(Declared::default());
    };
    rusty_mux_script::parse_declared(text).map_err(|problems| {
        let lines: Vec<String> = problems
            .iter()
            .map(|p| format!("{dest}:{}: {p}", file.path))
            .collect();
        failed(format!(
            "can't check the remote config's hosts, so nothing was pushed:\n{}",
            lines.join("\n")
        ))
    })
}

/// The command line `ssh` gets to run `script` on `dest` with `args`.
fn ssh_args(dest: &str, script: &str, args: &str) -> [String; 3] {
    [
        "--".to_owned(),
        dest.to_owned(),
        format!("sh -c '{script}' sh{args}"),
    ]
}

/// The write script's arguments: the checksum read, or `none`. Only digits
/// and `none` ever reach the remote shell.
fn sum_args(sum: Option<Sum>) -> String {
    match sum {
        Some(Sum { crc, size }) => format!(" {crc} {size}"),
        None => " none".to_owned(),
    }
}

fn read_remote(ssh: &OsString, dest: &str) -> Result<Answer, (Exit, String)> {
    let mut child = Command::new(ssh)
        .args(ssh_args(dest, READ, ""))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| started(ssh, e))?;
    let answer = read_capped(&mut child, dest);
    let status = child.wait().map_err(|e| failed(e.to_string()))?;
    let answer = answer?;
    ended(status.code(), dest)?;
    parse_answer(&answer).ok_or_else(|| {
        failed(format!(
            "{dest} did not run the sync script; it needs a POSIX sh"
        ))
    })
}

/// Sends `hosts` to `dest`'s write script, which refuses if the remote
/// file is no longer the one `sum` describes.
fn write_remote(
    ssh: &OsString,
    dest: &str,
    hosts: &[Host],
    sum: Option<Sum>,
) -> Result<(), (Exit, String)> {
    let mut child = Command::new(ssh)
        .args(ssh_args(dest, WRITE, &sum_args(sum)))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| started(ssh, e))?;
    let sent = match child.stdin.take() {
        Some(mut stdin) => stdin.write_all(hosts::to_json(hosts).as_bytes()),
        None => Ok(()),
    };
    let status = child.wait().map_err(|e| failed(e.to_string()))?;
    if status.code() == Some(CHANGED) {
        return Err(failed(format!(
            "{dest}'s hosts.json changed after it was read, so nothing was pushed; run push again"
        )));
    }
    ended(status.code(), dest)?;
    sent.map_err(|e| failed(format!("sending to {dest}: {e}")))
}

/// The child's stdout, up to [`MAX_ANSWER`] bytes. Past that the child is
/// killed, so a runaway remote can't fill memory.
fn read_capped(child: &mut Child, dest: &str) -> Result<Vec<u8>, (Exit, String)> {
    let mut answer = Vec::new();
    let Some(stdout) = child.stdout.take() else {
        return Ok(answer);
    };
    stdout
        .take(MAX_ANSWER + 1)
        .read_to_end(&mut answer)
        .map_err(|e| failed(format!("reading from {dest}: {e}")))?;
    if answer.len() as u64 <= MAX_ANSWER {
        return Ok(answer);
    }
    // Already exited is fine: either way it is stopped.
    let _ = child.kill();
    Err(failed(format!(
        "{dest} printed more than {MAX_ANSWER} bytes; nothing was synced"
    )))
}

/// The script's answer in `bytes`, past any rc-file noise before it.
fn parse_answer(bytes: &[u8]) -> Option<Answer> {
    let (hosts, rest) = parse_file(bytes, "hosts")?;
    let (config, _) = parse_file(rest, "config")?;
    Some(Answer { hosts, config })
}

/// The file the `tag` marker describes, and the bytes after it.
fn parse_file<'a>(bytes: &'a [u8], tag: &str) -> Option<(File, &'a [u8])> {
    let marker = format!("{MARK}{tag} ");
    let start = find(bytes, marker.as_bytes())? + marker.len();
    let end = start + bytes[start..].iter().position(|&b| b == b'\n')?;
    let line = std::str::from_utf8(&bytes[start..end]).ok()?;
    let rest = &bytes[end + 1..];
    if let Some(path) = line.strip_prefix("none ") {
        let file = File {
            path: path.to_owned(),
            body: None,
        };
        return Some((file, rest));
    }
    let mut words = line.splitn(3, ' ');
    let sum = Sum {
        crc: digits(words.next()?)?,
        size: digits(words.next()?)?,
    };
    let path = words.next()?.to_owned();
    let text = rest.get(..sum.size)?;
    let text = String::from_utf8(text.to_vec()).ok()?;
    let rest = rest.get(sum.size..)?;
    let rest = rest.strip_prefix(b"\n")?;
    let file = File {
        path,
        body: Some((sum, text)),
    };
    Some((file, rest))
}

/// `word` as a number, if it is only ASCII digits.
fn digits<T: std::str::FromStr>(word: &str) -> Option<T> {
    if word.is_empty() || !word.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    word.parse().ok()
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// One line per reported host, or a JSON array of them.
fn report(merged: &Merged, way: Way, json: bool) -> String {
    if json {
        let all: Vec<Value> = merged
            .report
            .iter()
            .map(|(name, outcome)| entry(name, outcome))
            .collect();
        return Value::from(all).to_json_string();
    }
    let winner = match way {
        Way::Pull => "theirs",
        Way::Push => "ours",
    };
    let lines: Vec<String> = merged
        .report
        .iter()
        .map(|(name, outcome)| match outcome {
            Outcome::Added => format!("added    {name}"),
            Outcome::Kept => format!("kept     {name} (differs; --replace takes {winner})"),
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
    fn the_scripts_hold_no_single_quote() {
        for script in [READ, WRITE] {
            assert!(!script.contains('\''));
        }
        assert_eq!(ssh_args("me@box", READ, "")[..2], ["--", "me@box"]);
        let sum = Some(Sum { crc: 7, size: 12 });
        assert_eq!(sum_args(sum), " 7 12");
        assert_eq!(sum_args(None), " none");
    }

    #[test]
    fn the_answer_is_found_past_rc_noise() {
        let body = "{\"hosts\": []}\n";
        let answer = format!(
            "motd\nnoise rusty-mux-hosts:hosts 99 {} /h/.config/rusty_mux/hosts.json\n{body}\n\
             rusty-mux-hosts:config none /h/c.json\n",
            body.len()
        );
        let parsed = parse_answer(answer.as_bytes()).expect("parses");
        assert_eq!(parsed.hosts.path, "/h/.config/rusty_mux/hosts.json");
        let sum = Sum {
            crc: 99,
            size: body.len(),
        };
        assert_eq!(parsed.hosts.body, Some((sum, body.to_owned())));
        assert_eq!(
            parsed.config,
            File {
                path: "/h/c.json".into(),
                body: None
            }
        );
    }

    #[test]
    fn a_body_is_taken_by_size_even_if_it_holds_a_marker() {
        let body = "rusty-mux-hosts:config none /fake";
        let answer = format!(
            "rusty-mux-hosts:hosts 1 {} /p\n{body}\nrusty-mux-hosts:config none /c\n",
            body.len()
        );
        let parsed = parse_answer(answer.as_bytes()).expect("parses");
        assert_eq!(parsed.config.path, "/c");
    }

    #[test]
    fn a_bad_answer_is_none() {
        for bad in [
            "just a shell banner\n",
            "rusty-mux-hosts:hosts none /p\n",
            "rusty-mux-hosts:hosts x 3 /p\nabc\nrusty-mux-hosts:config none /c\n",
            "rusty-mux-hosts:hosts 1 99 /p\nshort\nrusty-mux-hosts:config none /c\n",
            "rusty-mux-hosts:hosts 1 -3 /p\nabc\nrusty-mux-hosts:config none /c\n",
        ] {
            assert_eq!(parse_answer(bad.as_bytes()), None, "{bad}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_scripts_refuse_a_relative_home() {
        let run = |script: &str, home: &str| {
            Command::new("sh")
                .args(["-c", script, "sh", "none"])
                .env_remove("XDG_CONFIG_HOME")
                .env("HOME", home)
                .stdin(Stdio::null())
                .output()
                .map(|o| o.status.code())
        };
        assert_eq!(run(READ, "relative/home").ok(), Some(Some(1)));
        assert_eq!(run(WRITE, "relative/home").ok(), Some(Some(1)));
        assert_eq!(run(READ, "/nonexistent-home").ok(), Some(Some(0)));
    }

    #[cfg(unix)]
    #[test]
    fn a_runaway_remote_is_cut_off() {
        let mut child = Command::new("yes")
            .stdout(Stdio::piped())
            .spawn()
            .expect("yes runs");
        let read = read_capped(&mut child, "box");
        child.wait().expect("yes stops");
        assert!(read.is_err_and(|(_, e)| e.contains("more than")));
    }
}
