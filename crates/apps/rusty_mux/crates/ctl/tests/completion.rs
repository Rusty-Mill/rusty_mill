//! `completion SHELL` scripts run in the real shells: each script is asked
//! to complete the same command lines. A shell that is not installed is
//! skipped with a note. PR CI has bash on Linux; PowerShell runs where
//! Windows tests run.

use std::fs;
use std::io;
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
    // A quoted value with a space is one word, not two.
    (
        "rusty-mux-ctl --socket \"some path\" terminal.fo",
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

/// A private directory this test created itself, removed when dropped.
/// An existing path is never reused or cleared, however predictable its
/// name.
struct Scratch(PathBuf);

impl Scratch {
    /// Creates `path` exclusively (mode 0700 on Unix); fails if anything
    /// is there.
    fn acquire(path: PathBuf) -> io::Result<Self> {
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(&path)?;
        Ok(Self(path))
    }

    /// A fresh directory under `base`, trying new names past any collision.
    fn fresh_in(base: &Path, tag: &str) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        for _ in 0..64 {
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let name = format!(
                "rusty-mux-completion-{}-{nanos}-{n}-{tag}",
                std::process::id()
            );
            match Self::acquire(base.join(name)) {
                Ok(dir) => return dir,
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => panic!("scratch dir under {}: {e}", base.display()),
            }
        }
        panic!("no free scratch name under {}", base.display());
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The script for `shell` and an empty working directory, both inside one
/// scratch directory.
struct Setup {
    /// Holds the directory until the test ends.
    _scratch: Scratch,
    script: PathBuf,
    work: PathBuf,
}

fn setup(shell: &str) -> Setup {
    let scratch = Scratch::fresh_in(&std::env::temp_dir(), shell);
    let output = Command::new(ctl())
        .args(["completion", shell])
        .output()
        .expect("run rusty-mux-ctl");
    assert!(output.status.success(), "completion {shell}");
    let extension = if shell == "powershell" { "ps1" } else { shell };
    let script = scratch.path().join(format!("script.{extension}"));
    fs::write(&script, &output.stdout).expect("write script");
    let work = scratch.path().join("work");
    fs::create_dir(&work).expect("work dir");
    Setup {
        _scratch: scratch,
        script,
        work,
    }
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
/// returns the sorted lines it printed, one completion each.
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
    let mut words: Vec<String> = stdout
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect();
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

/// Bash, with `COMP_WORDS` split as readline splits the line: quotes kept
/// together, and an empty word after a trailing space.
fn bash_program(script: &Path) -> String {
    format!(
        "source '{}'; eval \"COMP_WORDS=($LINE)\"; \
         [[ $LINE == *' ' ]] && COMP_WORDS+=(''); \
         COMP_CWORD=$((${{#COMP_WORDS[@]}} - 1)); _rusty_mux_ctl; \
         printf '%s\\n' \"${{COMPREPLY[@]}}\"",
        script.display()
    )
}

#[test]
fn scratch_never_takes_or_clears_an_existing_path() {
    let base = Scratch::fresh_in(&std::env::temp_dir(), "base");
    let taken = base.path().join("taken");
    fs::create_dir(&taken).expect("taken");
    fs::write(taken.join("sentinel"), "keep").expect("sentinel");
    let err = Scratch::acquire(taken.clone())
        .err()
        .expect("an existing path is refused");
    assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(
        fs::read_to_string(taken.join("sentinel")).ok().as_deref(),
        Some("keep")
    );

    let fresh = Scratch::fresh_in(base.path(), "taken");
    assert_ne!(fresh.path(), taken);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(fresh.path())
            .expect("meta")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);
    }
    let gone = fresh.path().to_path_buf();
    drop(fresh);
    assert!(!gone.exists(), "the acquired dir is removed");
    assert_eq!(
        fs::read_to_string(taken.join("sentinel")).ok().as_deref(),
        Some("keep")
    );
}

#[test]
#[cfg(unix)]
fn bash_completes() {
    if !installed("bash") {
        return;
    }
    let s = setup("bash");
    check("bash", &["-c", &bash_program(&s.script)], &s.work);
}

#[test]
#[cfg(unix)]
fn bash_completes_a_path_with_spaces_and_glob_characters_as_one_word() {
    if !installed("bash") {
        return;
    }
    let s = setup("bash");
    let program = bash_program(&s.script);
    let dir = s.work.with_file_name("paths");
    fs::create_dir(&dir).expect("paths");
    fs::write(dir.join("my file.txt"), "").expect("spaced");
    fs::write(dir.join("my*star"), "").expect("star");
    fs::write(dir.join("other"), "").expect("other");
    assert_eq!(
        complete(
            "bash",
            &["-c", &program],
            &dir,
            "rusty-mux-ctl run --cwd my"
        ),
        ["my file.txt", "my*star"],
    );
}

#[test]
#[cfg(unix)]
fn zsh_completes() {
    if !installed("zsh") {
        return;
    }
    let s = setup("zsh");
    // zsh's completion system needs a terminal; stand in for the two
    // builtins the script calls, filtering by prefix as compadd does.
    let program = format!(
        "compdef() {{ :; }}; \
         compadd() {{ shift; local w; for w; do [[ $w == \"$PREFIX\"* ]] && print -r -- $w; done; }}; \
         _files() {{ print FILES; }}; \
         source '{}'; words=(${{(z)LINE}}); [[ $LINE == *' ' ]] && words+=(''); \
         CURRENT=${{#words}}; PREFIX=$words[CURRENT]; _rusty_mux_ctl; true",
        s.script.display()
    );
    check("zsh", &["-fc", &program], &s.work);
}

#[test]
#[cfg(unix)]
fn fish_completes() {
    if !installed("fish") {
        return;
    }
    let s = setup("fish");
    let program = format!(
        "source '{}'; complete -C\"$LINE\" | string replace -r '\\t.*' ''; true",
        s.script.display()
    );
    check("fish", &["--no-config", "-c", &program], &s.work);
}

#[test]
fn nu_completes() {
    if !installed("nu") {
        return;
    }
    let s = setup("nu");
    // A completer's words are filtered by Nushell's prompt; filter them
    // here by the word under the cursor.
    let program = format!(
        "source '{}'; let line = $env.LINE; \
         let word = ($line | split row ' ' | last); \
         let words = (nu-complete rusty-mux-ctl $line); \
         if $words != null {{ $words | where {{|w| $w | str starts-with $word }} | str join (char nl) }} else {{ '' }}",
        s.script.display()
    );
    check("nu", &["--no-config-file", "-c", &program], &s.work);
}

#[test]
#[cfg(windows)]
fn powershell_completes() {
    let program = ["pwsh", "powershell"]
        .into_iter()
        .find(|p| installed(p))
        .expect("PowerShell on Windows");
    let s = setup("powershell");
    let bin = Path::new(ctl()).parent().expect("bin dir");
    let command = format!(
        "$env:PATH = '{};' + $env:PATH; . '{}'; \
         (TabExpansion2 $env:LINE $env:LINE.Length).CompletionMatches | \
         ForEach-Object {{ $_.CompletionText }}",
        bin.display(),
        s.script.display()
    );
    check(
        program,
        &["-NoProfile", "-NonInteractive", "-Command", &command],
        &s.work,
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
