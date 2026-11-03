//! `completion SHELL` scripts run in the real shells: each script is asked
//! to complete the same command lines. A shell that is not installed is
//! skipped with a note; CI has bash on Linux and PowerShell on Windows.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Command lines, each ending where the cursor is, and what the shell
/// should offer there, sorted.
const CASES: &[(&str, &[&str])] = &[
    ("rusty-mux-ctl terminal.fo", &["terminal.format"]),
    (
        "rusty-mux-ctl --socket s --json terminal.fo",
        &["terminal.format"],
    ),
    ("rusty-mux-ctl terminal.format --block x --tr", &["--trim"]),
    (
        "rusty-mux-ctl terminal.format --block x --trim --unw",
        &["--unwrap"],
    ),
    ("rusty-mux-ctl list + acq", &["acquire-lease"]),
    (
        "rusty-mux-ctl help terminal.p",
        &["terminal.process", "terminal.pwd"],
    ),
    ("rusty-mux-ctl api ll", &["llms"]),
    ("rusty-mux-ctl completion po", &["powershell"]),
];

/// After a flag that takes a value nothing is offered. Run in an empty
/// directory, since PowerShell offers files when a completer offers
/// nothing.
const VALUE_CASE: &str = "rusty-mux-ctl terminal.format --block ";

fn ctl() -> &'static str {
    env!("CARGO_BIN_EXE_rusty-mux-ctl")
}

/// A fresh empty directory holding the script for `shell`.
fn setup(shell: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "rusty-mux-completion-{shell}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let output = Command::new(ctl())
        .args(["completion", shell])
        .output()
        .expect("run rusty-mux-ctl");
    assert!(output.status.success(), "completion {shell}");
    let extension = if shell == "powershell" { "ps1" } else { shell };
    let script = dir.join(format!("script.{extension}"));
    std::fs::write(&script, &output.stdout).expect("write script");
    let work = dir.join("work");
    std::fs::create_dir_all(&work).expect("work dir");
    (script, work)
}

fn installed(program: &str) -> bool {
    let found = Command::new(program)
        .arg(if program.contains("powershell") || program == "pwsh" {
            "-Help"
        } else {
            "--version"
        })
        .output()
        .is_ok();
    if !found {
        eprintln!("{program} is not installed; skipping its completion test");
    }
    found
}

/// Runs `program` with `args` and the line in `$LINE`, in `work`, and
/// returns the sorted words it printed.
fn complete(program: &str, args: &[&str], work: &Path, line: &str) -> Vec<String> {
    let output = Command::new(program)
        .args(args)
        .env("LINE", line)
        .current_dir(work)
        .output()
        .expect("run shell");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{program} {line:?}: {stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut words: Vec<String> = stdout.split_whitespace().map(str::to_owned).collect();
    words.sort();
    words
}

fn check(program: &str, args: &[&str], work: &Path) {
    for (line, expected) in CASES {
        assert_eq!(
            complete(program, args, work, line),
            *expected,
            "{program}: {line:?}"
        );
    }
    assert!(
        complete(program, args, work, VALUE_CASE).is_empty(),
        "{program}: {VALUE_CASE:?}"
    );
}

#[test]
#[cfg(unix)]
fn bash_completes() {
    if !installed("bash") {
        return;
    }
    let (script, work) = setup("bash");
    let program = format!(
        "source '{}'; read -ra COMP_WORDS <<< \"$LINE\"; \
         [[ $LINE == *' ' ]] && COMP_WORDS+=(''); \
         COMP_CWORD=$((${{#COMP_WORDS[@]}} - 1)); _rusty_mux_ctl; \
         printf '%s\\n' \"${{COMPREPLY[@]}}\"",
        script.display()
    );
    check("bash", &["-c", &program], &work);
}

#[test]
#[cfg(unix)]
fn zsh_completes() {
    if !installed("zsh") {
        return;
    }
    let (script, work) = setup("zsh");
    // zsh's completion system needs a terminal; stand in for the two
    // builtins the script calls, filtering by prefix as compadd does.
    let program = format!(
        "compdef() {{ :; }}; \
         compadd() {{ shift; local w; for w; do [[ $w == \"$PREFIX\"* ]] && print -r -- $w; done; }}; \
         _files() {{ print FILES; }}; \
         source '{}'; words=(${{=LINE}}); [[ $LINE == *' ' ]] && words+=(''); \
         CURRENT=${{#words}}; PREFIX=$words[CURRENT]; _rusty_mux_ctl; true",
        script.display()
    );
    check("zsh", &["-fc", &program], &work);
}

#[test]
#[cfg(unix)]
fn fish_completes() {
    if !installed("fish") {
        return;
    }
    let (script, work) = setup("fish");
    let program = format!(
        "source '{}'; complete -C\"$LINE\" | string replace -r '\\t.*' ''; true",
        script.display()
    );
    check("fish", &["--no-config", "-c", &program], &work);
}

#[test]
fn nu_completes() {
    if !installed("nu") {
        return;
    }
    let (script, work) = setup("nu");
    // A completer's words are filtered by Nushell's prompt; filter them
    // here by the word under the cursor.
    let program = format!(
        "source '{}'; let line = $env.LINE; \
         let word = ($line | split row ' ' | last); \
         let words = (nu-complete rusty-mux-ctl $line); \
         if $words != null {{ $words | where {{|w| $w | str starts-with $word }} | str join (char nl) }} else {{ '' }}",
        script.display()
    );
    check("nu", &["--no-config-file", "-c", &program], &work);
}

#[test]
#[cfg(windows)]
fn powershell_completes() {
    let program = ["pwsh", "powershell"]
        .into_iter()
        .find(|p| installed(p))
        .expect("PowerShell on Windows");
    let (script, work) = setup("powershell");
    let bin = Path::new(ctl()).parent().expect("bin dir");
    let command = format!(
        "$env:PATH = '{};' + $env:PATH; . '{}'; \
         (TabExpansion2 $env:LINE $env:LINE.Length).CompletionMatches | \
         ForEach-Object {{ $_.CompletionText }}",
        bin.display(),
        script.display()
    );
    check(
        program,
        &["-NoProfile", "-NonInteractive", "-Command", &command],
        &work,
    );
}

#[test]
fn a_wrong_shell_is_a_usage_error() {
    for args in [
        &["completion"][..],
        &["completion", "csh"],
        &["completion", "bash", "zsh"],
    ] {
        let output = Command::new(ctl()).args(args).output().expect("run");
        assert_eq!(output.status.code(), Some(2), "{args:?}");
    }
}
