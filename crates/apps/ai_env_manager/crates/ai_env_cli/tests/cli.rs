//! End to end on real temp folders: a fake home, a skills folder, the real
//! built-in catalog (claude-code and codex).

use ai_env_cli::{args, run, Error, Outcome};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

struct Tmp(PathBuf);

impl Tmp {
    fn new() -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "aienv-cli-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        Tmp(dir)
    }

    fn home(&self) -> PathBuf {
        self.0.join("home")
    }

    /// A skills folder holding `code-review` and `notes-helper`.
    fn skills(&self) -> PathBuf {
        let dir = self.0.join("skills");
        for name in ["code-review", "notes-helper"] {
            fs::create_dir_all(dir.join(name)).unwrap();
            fs::write(dir.join(name).join("SKILL.md"), name).unwrap();
        }
        dir
    }

    fn target(&self, tool_dir: &str, entry: &str) -> PathBuf {
        self.home().join(tool_dir).join("skills").join(entry)
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Result, stdout and stderr of one command line.
fn run_full(line: &str, home: &Path) -> (Result<Outcome, Error>, String, String) {
    let argv: Vec<String> = line.split_whitespace().map(str::to_owned).collect();
    let cmd = args::parse(&argv).unwrap();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let result = run(&cmd, home, &mut out, &mut err);
    (
        result,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
    )
}

fn run_line(line: &str, home: &Path) -> (Result<Outcome, Error>, String) {
    let (r, out, _) = run_full(line, home);
    (r, out)
}

fn line(verb: &str, skills: &Path, extra: &str) -> String {
    format!("{verb} --skills {} {extra}", skills.display())
}

#[test]
fn list_shows_every_skill_and_tool() {
    let t = Tmp::new();
    let (r, out) = run_line(&line("list", &t.skills(), ""), &t.home());
    assert!(matches!(r, Ok(Outcome::Done)));
    assert_eq!(
        out,
        "code-review  claude-code=-  codex=-\nnotes-helper  claude-code=-  codex=-\n"
    );
}

#[test]
fn enable_without_yes_is_a_dry_run() {
    let t = Tmp::new();
    let (r, out) = run_line(&line("enable code-review", &t.skills(), ""), &t.home());
    assert!(matches!(r, Ok(Outcome::Done)));
    assert!(out.contains("link") && out.contains("dry run"), "{out}");
    assert!(fs::symlink_metadata(t.target(".claude", "code-review")).is_err());
    assert!(
        !t.home().exists(),
        "a dry run must not even create the home folder"
    );
}

#[test]
fn enable_then_disable_round_trip_with_yes() {
    let t = Tmp::new();
    let skills = t.skills();
    let (r, _) = run_line(
        &line("enable code-review --yes --tool claude-code", &skills, ""),
        &t.home(),
    );
    assert!(matches!(r, Ok(Outcome::Done)));
    assert!(t
        .target(".claude", "code-review")
        .join("SKILL.md")
        .is_file());
    assert!(
        !t.target(".agents", "code-review").exists(),
        "only the chosen tool"
    );

    let (_, listed) = run_line(&line("list", &skills, ""), &t.home());
    assert!(
        listed.contains("code-review  claude-code=enabled  codex=-"),
        "{listed}"
    );

    let (_, again) = run_line(
        &line("enable code-review --yes --tool claude-code", &skills, ""),
        &t.home(),
    );
    assert!(again.contains("nothing to do"), "{again}");

    let (r, _) = run_line(&line("disable code-review --yes", &skills, ""), &t.home());
    assert!(matches!(r, Ok(Outcome::Done)));
    assert!(fs::symlink_metadata(t.target(".claude", "code-review")).is_err());
    assert!(
        skills.join("code-review").join("SKILL.md").is_file(),
        "source survives"
    );
}

#[test]
fn a_foreign_folder_is_refused_and_kept_even_with_yes() {
    let t = Tmp::new();
    let foreign = t.target(".claude", "code-review");
    fs::create_dir_all(&foreign).unwrap();
    fs::write(foreign.join("mine.md"), "keep").unwrap();
    let skills = t.skills();
    let enable = line("enable code-review --yes", &skills, "");
    let (r, out) = run_line(&enable, &t.home());
    assert!(matches!(r, Ok(Outcome::Refused)), "{out}");
    assert!(out.contains("refuse"), "{out}");
    // Each tool is planned on its own: the other one is still applied.
    assert!(fs::symlink_metadata(t.target(".agents", "code-review")).is_ok());

    let (r, _) = run_line(&line("disable code-review --yes", &skills, ""), &t.home());
    assert!(matches!(r, Ok(Outcome::Refused)));
    assert_eq!(fs::read_to_string(foreign.join("mine.md")).unwrap(), "keep");
    assert!(fs::symlink_metadata(t.target(".agents", "code-review")).is_err());
}

#[test]
fn bad_names_are_reported() {
    let t = Tmp::new();
    let skills = t.skills();
    assert!(matches!(
        run_line(&line("enable nope --yes", &skills, ""), &t.home()).0,
        Err(Error::UnknownEntry(_))
    ));
    assert!(matches!(
        run_line(&line("list", &skills, "--tool nope"), &t.home()).0,
        Err(Error::UnknownTool(_))
    ));
    let (r, _) = run_line(
        &format!("list --skills {0} --skills {0}", skills.display()),
        &t.home(),
    );
    assert!(matches!(r, Err(Error::DuplicateEntry(_))));
    let (r, _) = run_line("list --skills /no/such/folder", &t.home());
    let Err(Error::Folder(path, _)) = r else {
        panic!("expected a folder error")
    };
    assert_eq!(path, Path::new("/no/such/folder"));
    // A file where a folder is expected also names the path in the message.
    let file = t.0.join("a-file");
    fs::write(&file, "x").unwrap();
    let (r, _) = run_line(&format!("list --skills {}", file.display()), &t.home());
    assert!(r.unwrap_err().to_string().contains("a-file"));
}

#[test]
fn the_binary_sets_exit_codes() {
    let t = Tmp::new();
    let skills = t.skills();
    let aienv = |args: &[&str]| {
        let mut c = Command::new(env!("CARGO_BIN_EXE_aienv"));
        c.args(args)
            .env("HOME", t.home())
            .env("USERPROFILE", t.home());
        c.output().unwrap()
    };
    let s = skills.to_str().unwrap();
    assert_eq!(aienv(&["list", "--skills", s]).status.code(), Some(0));
    assert_eq!(
        aienv(&[
            "enable",
            "code-review",
            "--skills",
            s,
            "--yes",
            "--tool",
            "codex"
        ])
        .status
        .code(),
        Some(0)
    );
    assert_eq!(aienv(&["frob"]).status.code(), Some(2));
    assert_eq!(
        aienv(&["enable", "missing", "--skills", s]).status.code(),
        Some(3)
    );
    assert_eq!(
        aienv(&["list", "--skills", "/no/such/folder"])
            .status
            .code(),
        Some(3)
    );
    assert_eq!(
        aienv(&["list", "--skills", s, "--tool", "x", "--tool", "x"])
            .status
            .code(),
        Some(2)
    );
    let foreign = t.target(".claude", "notes-helper");
    fs::create_dir_all(&foreign).unwrap();
    assert_eq!(
        aienv(&["enable", "notes-helper", "--skills", s, "--yes"])
            .status
            .code(),
        Some(1)
    );
}

#[test]
fn skipped_folders_go_to_stderr_not_stdout() {
    let t = Tmp::new();
    let skills = t.skills();
    fs::create_dir_all(skills.join("Bad_Name")).unwrap();
    fs::write(skills.join("Bad_Name").join("SKILL.md"), "x").unwrap();
    let (r, out, err) = run_full(&line("list", &skills, ""), &t.home());
    assert!(matches!(r, Ok(Outcome::Done)));
    assert!(err.contains("skipped") && err.contains("Bad_Name"), "{err}");
    assert!(!out.contains("skipped"), "{out}");
}

#[test]
fn a_failure_partway_says_which_tools_were_already_applied() {
    let t = Tmp::new();
    let skills = t.skills();
    // codex links into home/.agents/skills; a file named .agents makes that fail
    // after claude-code, which comes first in the catalog, has been applied.
    fs::create_dir_all(t.home()).unwrap();
    fs::write(t.home().join(".agents"), "in the way").unwrap();
    let (r, out) = run_line(&line("enable code-review --yes", &skills, ""), &t.home());
    assert!(r.is_err(), "{out}");
    assert!(
        out.contains("claude-code:\n") && out.contains("applied"),
        "{out}"
    );
    assert!(fs::symlink_metadata(t.target(".claude", "code-review")).is_ok());
}

#[test]
fn help_runs_without_a_home_folder() {
    // The binary must print help even when HOME and USERPROFILE are unset.
    let out = Command::new(env!("CARGO_BIN_EXE_aienv"))
        .arg("--help")
        .env_remove("HOME")
        .env_remove("USERPROFILE")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("usage: aienv"));
}

#[cfg(unix)]
#[test]
fn a_non_utf8_argument_exits_2_instead_of_panicking() {
    use std::os::unix::ffi::OsStrExt;
    let out = Command::new(env!("CARGO_BIN_EXE_aienv"))
        .arg("list")
        .arg("--skills")
        .arg(std::ffi::OsStr::from_bytes(&[b'a', 0xff]))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("not valid UTF-8"));
}
