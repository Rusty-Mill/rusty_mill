//! `terminfo check|setup|remove DEST` (C47): rusty_term's terminfo entry in
//! the remote user's `~/.terminfo`, so programs reached over SSH from a
//! rusty_term tab know its capabilities.
//!
//! Each verb runs one `ssh -- DEST sh -c '<script>'`. The scripts are fixed
//! and hold no single quote, so no user text reaches the remote shell and
//! any login shell runs them. `tic -o` names the directory, so even root
//! never writes the system's.

use std::ffi::OsString;
use std::io::{self, Write};
use std::process::{Command, Stdio};

use crate::Exit;
use crate::cli::{Terminfo, TerminfoVerb};

/// rusty_term's entries, a copy of `rusty_term/extra/rusty_term.terminfo`
/// (ctl must not link rusty_term).
pub(crate) const SOURCE: &str = include_str!("terminfo/rusty_term.terminfo");

/// Marks the check's own output lines, so rc-file noise around them on the
/// remote is skipped, even noise with no newline before a marker.
const MARK: &str = "rusty-mux-terminfo:";

/// Prints where `rusty_term` is found, then whether `tic` exists, each line
/// after [`MARK`].
const CHECK: &str = "m=rusty-mux-terminfo:; if infocmp -A \"$HOME/.terminfo\" -x rusty_term >/dev/null 2>&1; \
then echo ${m}home; elif infocmp -x rusty_term >/dev/null 2>&1; then echo ${m}elsewhere; \
else echo ${m}missing; fi; if command -v tic >/dev/null 2>&1; then echo ${m}tic; else echo ${m}no-tic; fi";

/// Compiles the source on stdin into `~/.terminfo`, through a temporary
/// file, since not every `tic` reads stdin.
const SETUP: &str = concat!(
    "[ -n \"$HOME\" ] || { echo \"HOME is not set\" >&2; exit 1; }; ",
    "t=$(mktemp) || exit 1; cat >\"$t\" && mkdir -p \"$HOME/.terminfo\" \
&& tic -x -o \"$HOME/.terminfo\" \"$t\"; s=$?; rm -f \"$t\"; exit $s"
);

/// Removes our two entries, in Linux's and macOS's directory layouts.
const REMOVE: &str = concat!(
    "[ -n \"$HOME\" ] || { echo \"HOME is not set\" >&2; exit 1; }; ",
    "rm -f \"$HOME/.terminfo/r/rusty_term\" \"$HOME/.terminfo/r/rusty_term-256color\" \
\"$HOME/.terminfo/72/rusty_term\" \"$HOME/.terminfo/72/rusty_term-256color\""
);

/// The exit code ssh gives when it cannot connect or log in.
const SSH_FAILED: i32 = 255;

/// The command line `ssh` gets for `verb` on `dest`.
pub(crate) fn ssh_args(verb: TerminfoVerb, dest: &str) -> [String; 3] {
    let script = match verb {
        TerminfoVerb::Check => CHECK,
        TerminfoVerb::Setup => SETUP,
        TerminfoVerb::Remove => REMOVE,
    };
    [
        "--".to_owned(),
        dest.to_owned(),
        format!("sh -c '{script}'"),
    ]
}

/// Runs `command` with `ssh` (`$RUSTY_MUX_SSH`, else `ssh`). `check` exits
/// 0 only when the entry is in the remote `~/.terminfo`; `setup` and
/// `remove` print the command first and with `--dry-run` stop there. ssh
/// failing to connect or log in exits 4.
pub(crate) fn run(
    command: &Terminfo,
    ssh: &OsString,
    out: &mut dyn Write,
) -> Result<Exit, (Exit, String)> {
    let args = ssh_args(command.verb, &command.dest);
    let fail = |e: io::Error| (Exit::Failed, e.to_string());
    if command.verb == TerminfoVerb::Check {
        return check(ssh, &args, out);
    }
    writeln!(out, "{} {}", ssh.to_string_lossy(), shown(&args)).map_err(fail)?;
    if command.dry_run {
        return Ok(Exit::Ok);
    }
    let input = match command.verb {
        TerminfoVerb::Setup => Stdio::piped(),
        TerminfoVerb::Check | TerminfoVerb::Remove => Stdio::null(),
    };
    let mut child = Command::new(ssh)
        .args(&args)
        .stdin(input)
        .spawn()
        .map_err(|e| started(ssh, e))?;
    // A failed write (the remote end gone) is judged after ssh exits, so
    // an unreachable host still exits 4.
    let written = child
        .stdin
        .take()
        .map_or(Ok(()), |mut stdin| stdin.write_all(SOURCE.as_bytes()));
    let status = child.wait().map_err(fail)?;
    ended(status.code(), &command.dest)?;
    written.map_err(fail).map(|()| Exit::Ok)
}

fn check(ssh: &OsString, args: &[String], out: &mut dyn Write) -> Result<Exit, (Exit, String)> {
    let output = Command::new(ssh)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| started(ssh, e))?;
    ended(output.status.code(), &args[1])?;
    let text = String::from_utf8_lossy(&output.stdout);
    let (report, exit) = report(&text).ok_or_else(|| {
        (
            Exit::Failed,
            format!("unexpected answer from the remote check: {}", text.trim()),
        )
    })?;
    writeln!(out, "{report}").map_err(|e| (Exit::Failed, e.to_string()))?;
    Ok(exit)
}

/// What `check` prints for the remote script's two lines, and its exit.
fn report(text: &str) -> Option<(String, Exit)> {
    let mut lines = text
        .lines()
        .filter_map(|l| Some(l.split_once(MARK)?.1.trim()));
    let (place, exit) = match lines.next()? {
        "home" => ("installed in ~/.terminfo", Exit::Ok),
        "elsewhere" => ("not in ~/.terminfo, but found elsewhere", Exit::Failed),
        "missing" => ("missing", Exit::Failed),
        _ => return None,
    };
    let tic = match lines.next()? {
        "tic" => "tic: present",
        "no-tic" => "tic: missing, so setup cannot compile the entry",
        _ => return None,
    };
    Some((format!("rusty_term: {place}\n{tic}"), exit))
}

pub(crate) fn started(ssh: &OsString, e: io::Error) -> (Exit, String) {
    (
        Exit::Failed,
        format!("cannot run {}: {e}", ssh.to_string_lossy()),
    )
}

/// `Ok` for exit 0; ssh's own failure is unreachable, anything else failed.
pub(crate) fn ended(code: Option<i32>, dest: &str) -> Result<(), (Exit, String)> {
    match code {
        Some(0) => Ok(()),
        Some(SSH_FAILED) => Err((Exit::Unreachable, format!("ssh could not reach {dest}"))),
        Some(code) => Err((Exit::Failed, format!("the remote command exited {code}"))),
        None => Err((Exit::Failed, "ssh was killed".to_owned())),
    }
}

/// `args` as a POSIX shell would take them back: each one single-quoted,
/// so a pasted line expands nothing locally (`$HOME` stays the remote's).
fn shown(args: &[String]) -> String {
    args.iter()
        .map(|arg| quoted(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `arg` as is when no shell would change it, else in single quotes with
/// each `'` written as `'\''`.
fn quoted(arg: &str) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || "@%+=:,./_-".contains(c);
    if !arg.is_empty() && arg.chars().all(plain) {
        return arg.to_owned();
    }
    format!("'{}'", arg.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_copy_matches_rusty_terms_source() {
        let original = include_str!("../../../../../libs/ui/rusty_term/extra/rusty_term.terminfo");
        assert_eq!(SOURCE, original);
    }

    #[test]
    fn no_script_holds_a_single_quote_and_tic_names_its_directory() {
        for script in [CHECK, SETUP, REMOVE] {
            assert!(!script.contains('\''), "{script}");
        }
        assert!(SETUP.contains("tic -x -o \"$HOME/.terminfo\""));
        assert!(!REMOVE.contains('*'), "only our two entries");
    }

    #[test]
    fn the_destination_comes_after_a_double_dash() {
        let [dashes, dest, remote] = ssh_args(TerminfoVerb::Check, "me@box");
        assert_eq!((dashes.as_str(), dest.as_str()), ("--", "me@box"));
        assert!(remote.starts_with("sh -c '") && remote.ends_with('\''));
    }

    #[test]
    fn check_reads_both_lines() {
        let marked = |a: &str, b: &str| format!("{MARK}{a}\n{MARK}{b}\n");
        let (text, exit) = report(&marked("home", "tic")).expect("read");
        assert_eq!(exit, Exit::Ok);
        assert!(text.contains("installed in ~/.terminfo"), "{text}");
        assert_eq!(
            report(&marked("missing", "no-tic")).map(|r| r.1),
            Some(Exit::Failed)
        );
        assert_eq!(
            report(&marked("elsewhere", "tic")).map(|r| r.1),
            Some(Exit::Failed)
        );
        let noisy = format!("motd banner\n{}", marked("home", "tic"));
        assert_eq!(
            report(&noisy).map(|r| r.1),
            Some(Exit::Ok),
            "rc noise skipped"
        );
        let joined = format!("\x1b]0;title\x07{}", marked("home", "tic"));
        assert_eq!(
            report(&joined).map(|r| r.1),
            Some(Exit::Ok),
            "noise with no newline before the first marker"
        );
        assert!(CHECK.contains(MARK), "the script prints the marker");
        assert!(report("home\ntic").is_none(), "unmarked");
        assert!(report(&format!("{MARK}home")).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn the_shown_command_expands_nothing_locally() {
        let args = ssh_args(TerminfoVerb::Remove, "me@box");
        let line = shown(&args);
        assert!(line.starts_with("-- me@box 'sh -c '\\''[ -n "), "{line}");
        // Every `$` sits inside single quotes, which a shell never expands.
        let out = std::process::Command::new("sh")
            .args(["-c", &format!("HOME=/local; printf '%s\\n' {line}")])
            .output()
            .expect("run sh");
        let words: Vec<&str> = std::str::from_utf8(&out.stdout)
            .expect("utf-8")
            .lines()
            .collect();
        assert_eq!(words, ["--", "me@box", args[2].as_str()]);
    }

    #[test]
    fn ssh_failing_is_unreachable() {
        assert_eq!(
            ended(Some(255), "b").map_err(|e| e.0),
            Err(Exit::Unreachable)
        );
        assert_eq!(ended(Some(1), "b").map_err(|e| e.0), Err(Exit::Failed));
        assert!(ended(Some(0), "b").is_ok());
    }
}
