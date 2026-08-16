//! Shared black-box test support: drives the real compiled `sessionmgr`
//! binary as a subprocess against an isolated state root, the way a real
//! user or the Phase 4 TUI would.
//!
//! Deliberately black-box. This package has a `[lib]` target, so these
//! tests *could* call internals directly -- but the thing under test is
//! the daemon/worker/detach architecture itself: real OS processes, real
//! detachment, real sockets, real files. A lib-level call would bypass
//! exactly the machinery these tests exist to be evidence of.
//!
//! Ported from `rusty_prime_agent`'s `tests/common/mod.rs`, which
//! established this pattern for the same reasons.
//!
//! # Note on test placement
//!
//! PLAN.md's tree puts these at the workspace root (`sessionmgr/tests/`).
//! They live in `crates/sessionmgr-daemon/tests/` instead, for a purely
//! mechanical reason: Cargo only sets `CARGO_BIN_EXE_sessionmgr` for
//! tests belonging to the package that defines that binary, and a virtual
//! workspace root has no package to hang integration tests off at all.
//! Same tests, same harness, the only placement Cargo actually supports.

// Each integration-test file includes this module separately and no one
// of them uses every helper, so Clippy flags the unused ones per binary.
// This file is a shared toolbox; not every caller needs every tool.
#![allow(dead_code)]

use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// A short-named temporary state root, removed on drop -- along with any
/// processes the test left running.
///
/// **The name is deliberately tiny, and that is load-bearing.** `AF_UNIX`
/// allows 107 bytes for an entire socket path, and this project's own
/// worker sockets add `/s/<12-char-id>.sock` on top of whatever this
/// returns. A descriptive `sessionmgr-test-<name>-<pid>-<nanos>` directory
/// blows that budget on any real machine -- which is not hypothetical:
/// it is exactly what a manual smoke test of this code hit before the
/// name was shortened. A hash of the same uniqueness inputs keeps the
/// path tiny while staying collision-free.
pub struct TempRoot(PathBuf);

impl TempRoot {
    pub fn new(label: &str) -> Self {
        use std::hash::{Hash, Hasher};
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (label, std::process::id(), nanos).hash(&mut hasher);
        let dir = std::env::temp_dir().join(format!("sm{:x}", hasher.finish() & 0xffff_ffff));
        std::fs::create_dir_all(&dir).expect("create the temp state root");
        TempRoot(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    /// Kills anything the test left running before removing the
    /// directory.
    ///
    /// Necessary rather than tidy: every worker this project spawns is
    /// **deliberately detached**, so it survives the test binary exiting.
    /// Without this, a failing test leaks a background process that
    /// outlives the whole `cargo test` run.
    fn drop(&mut self) {
        for pid in all_recorded_pids(&self.0) {
            force_kill(pid);
        }
        if let Some(state) = daemon_pid(&self.0) {
            force_kill(state);
        }
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_sessionmgr"))
}

/// A throwaway git repository with one commit, removed on drop.
///
/// **Tests must never run against the repository they live in.** An
/// earlier version of the worktree test did exactly that, and left two
/// real worktrees and two real branches behind in this project's own
/// checkout. Worktree operations mutate a repository's administrative
/// state, so a test that points at the working repo is modifying the
/// thing it is being run from.
pub struct TempRepo {
    dir: PathBuf,
}

impl TempRepo {
    pub fn new(label: &str) -> Self {
        use std::hash::{Hash, Hasher};
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (label, "repo", std::process::id(), nanos).hash(&mut hasher);
        let dir = std::env::temp_dir().join(format!("smr{:x}", hasher.finish() & 0xffff_ffff));
        std::fs::create_dir_all(&dir).expect("create the temp repo");

        let git = |args: &[&str]| {
            let out = Command::new("git")
                .args(args)
                .current_dir(&dir)
                // A deterministic identity and no signing: the machine
                // running the tests may have neither configured, or may
                // have commit signing on, and neither should be able to
                // make this fail.
                .env("GIT_AUTHOR_NAME", "sessionmgr tests")
                .env("GIT_AUTHOR_EMAIL", "tests@example.invalid")
                .env("GIT_COMMITTER_NAME", "sessionmgr tests")
                .env("GIT_COMMITTER_EMAIL", "tests@example.invalid")
                .output()
                .expect("run git");
            assert!(
                out.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        // An explicit initial branch name, rather than whatever this
        // machine's `init.defaultBranch` happens to be.
        git(&["init", "--initial-branch=main"]);
        // Persisted into the repository's config, not just passed as env
        // vars to the calls above. Sessions commit from inside a worktree
        // via a shell command of their own, which inherits none of this
        // process's environment -- and linked worktrees share the main
        // repository's config, so writing it once here covers all of
        // them. Without it, every session's `git commit` fails for want
        // of an identity on a machine that has none configured globally.
        //
        // No spaces in the name, deliberately: the value has to survive a
        // trip through a shell command line on both `cmd.exe` and `sh`,
        // and `cmd.exe` does not understand the single quotes that would
        // otherwise be needed to protect it.
        git(&["config", "user.email", "tests@example.invalid"]);
        git(&["config", "user.name", "sessionmgr-tests"]);
        git(&["commit", "--allow-empty", "-m", "initial", "--no-gpg-sign"]);
        TempRepo { dir }
    }

    pub fn path(&self) -> &Path {
        &self.dir
    }

    pub fn path_str(&self) -> String {
        self.dir.to_string_lossy().into_owned()
    }

    /// Branches in this repository, one per line.
    pub fn branches(&self) -> String {
        let out = Command::new("git")
            .args(["branch", "--list"])
            .current_dir(&self.dir)
            .output()
            .expect("run git branch");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// Worktrees registered with this repository.
    pub fn worktrees(&self) -> String {
        let out = Command::new("git")
            .args(["worktree", "list"])
            .current_dir(&self.dir)
            .output()
            .expect("run git worktree list");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// Does a commit whose message is `message` exist on the current
    /// branch?
    pub fn log_contains(&self, message: &str) -> bool {
        let out = Command::new("git")
            .args(["log", "--oneline"])
            .current_dir(&self.dir)
            .output()
            .expect("run git log");
        String::from_utf8_lossy(&out.stdout).contains(message)
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A command that commits a file inside whatever directory it runs in,
/// so a worktree session produces something a merge can actually move.
///
/// **Not a single quote anywhere, and that is the whole trick.** This
/// string is handed to `cmd /C` on Windows and `sh -c` elsewhere, and
/// `cmd.exe` has no concept of single quotes -- it passes them through
/// literally. An earlier version used `-m 'add {name}'`, which `cmd`
/// split into `-m`, `'add`, and `{name}'`, so git took `'add` as the
/// whole message and `{name}'` as a pathspec:
///
/// ```text
/// error: pathspec 'from-the-session.txt'' did not match any file(s)
/// ```
///
/// Six of the ten worktree tests failed that way on Windows and passed on
/// Linux. The identity now lives in the repository's config (see
/// [`TempRepo::new`]) and the commit message contains no spaces, so
/// nothing here needs quoting on either shell.
pub fn commit_a_file(name: &str) -> Vec<String> {
    let script =
        format!("echo hello > {name} && git add {name} && git commit --no-gpg-sign -m add-{name}");
    if cfg!(windows) {
        vec!["cmd".into(), "/C".into(), script]
    } else {
        vec!["sh".into(), "-c".into(), script]
    }
}

/// Runs one CLI invocation to completion.
///
/// Stdin is explicitly nulled rather than inherited: `attach` reads
/// stdin, and a test runner's stdin is not necessarily anything that will
/// ever produce an EOF.
pub fn run(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(bin())
        .arg("--state-root")
        .arg(root)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("failed to run the sessionmgr binary")
}

pub fn stdout_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

pub fn assert_success(label: &str, output: &std::process::Output) {
    assert!(
        output.status.success(),
        "{label} failed (status {:?})\nstdout: {}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// Creates a session running `command`, returning its id.
pub fn session_new(root: &Path, command: &[&str]) -> String {
    session_new_in(root, &[], command)
}

/// As [`session_new`], but with extra flags (`--kind`, `--repo`) before
/// the `--` separator.
pub fn session_new_in(root: &Path, flags: &[&str], command: &[&str]) -> String {
    let mut args = vec!["new"];
    args.extend_from_slice(flags);
    args.push("--");
    args.extend_from_slice(command);
    let output = run(root, &args);
    if !output.status.success() {
        // The daemon's own stderr is the only place a startup failure is
        // recorded, so surface it rather than reporting a bare timeout.
        let log = std::fs::read_to_string(root.join("daemon.log"))
            .unwrap_or_else(|e| format!("<could not read daemon.log: {e}>"));
        panic!(
            "session new failed (status {:?})\nstdout: {}\nstderr: {}\ndaemon.log:\n{log}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
    stdout_of(&output)
}

/// A command that runs until something stops it, for testing survival.
pub fn long_running() -> Vec<&'static str> {
    if cfg!(windows) {
        // `ping` as a sleep is the classic portable-on-Windows trick;
        // `timeout` requires a real console and fails when redirected,
        // which is exactly how a worker runs its child.
        vec!["cmd", "/C", "ping -n 600 127.0.0.1 > NUL"]
    } else {
        vec!["sh", "-c", "sleep 600"]
    }
}

/// A command that prints `text` and exits successfully.
pub fn echo(text: &str) -> Vec<String> {
    if cfg!(windows) {
        vec!["cmd".into(), "/C".into(), format!("echo {text}")]
    } else {
        vec!["sh".into(), "-c".into(), format!("echo {text}")]
    }
}

pub fn session_list(root: &Path) -> String {
    let output = run(root, &["list"]);
    assert_success("list", &output);
    stdout_of(&output)
}

/// Reads a field out of a session's `state.json` directly.
///
/// Straight from the file rather than through the CLI: a test that needs
/// a worker's pid in order to kill it must not depend on the tool
/// choosing to expose pids, which it deliberately does not (pids have no
/// business on a list a UI renders).
fn state_json(root: &Path, id: &str) -> String {
    let path = root.join("sessions").join(id).join("state.json");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Extracts `"<key>": <number>` from a nested JSON object.
///
/// A deliberately small hand-rolled scan rather than a JSON dependency
/// for the test harness: these files are written by this same project and
/// the tests only ever need two integers out of them.
fn json_number(text: &str, key: &str) -> Option<u32> {
    let needle = format!("\"{key}\":");
    let start = text.find(&needle)? + needle.len();
    let rest = text[start..].trim_start();
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn json_string(text: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\":");
    let start = text.find(&needle)? + needle.len();
    let rest = text[start..].trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_owned())
}

/// The recorded worker pid for a session.
pub fn worker_pid(root: &Path, id: &str) -> u32 {
    let text = state_json(root, id);
    // `worker` precedes `child` in the record, so the first `pid` after
    // the `"worker"` key is the worker's.
    let from = text.find("\"worker\"").expect("state.json has a worker");
    json_number(&text[from..], "pid").expect("state.json has a worker pid")
}

/// The recorded child pid for a session.
pub fn child_pid(root: &Path, id: &str) -> u32 {
    let text = state_json(root, id);
    let from = text.find("\"child\"").expect("state.json has a child");
    json_number(&text[from..], "pid").expect("state.json has a child pid")
}

/// The session's status as recorded on disk.
pub fn session_status(root: &Path, id: &str) -> String {
    json_string(&state_json(root, id), "status").expect("state.json has a status")
}

/// The running daemon's pid, if one is recorded.
pub fn daemon_pid(root: &Path) -> Option<u32> {
    let text = std::fs::read_to_string(root.join("daemon.json")).ok()?;
    json_number(&text, "pid")
}

/// Every pid this state root currently records, for cleanup.
fn all_recorded_pids(root: &Path) -> Vec<u32> {
    let mut pids = Vec::new();
    let Ok(entries) = std::fs::read_dir(root.join("sessions")) else {
        return pids;
    };
    for entry in entries.flatten() {
        let path = entry.path().join("state.json");
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for key in ["\"worker\"", "\"child\""] {
            if let Some(from) = text.find(key) {
                if let Some(pid) = json_number(&text[from..], "pid") {
                    pids.push(pid);
                }
            }
        }
    }
    pids
}

/// Simulates an external crash: an unclean, uncatchable kill from
/// outside, the way an OOM killer or Task Manager would do it.
///
/// Deliberately not this project's own graceful shutdown path -- the
/// whole point is to act like an outside force. On Windows,
/// `TerminateProcess` (which is what `sessionmgr_proc::terminate` does
/// there, since Windows has no cooperative signal equivalent) is already
/// exactly that; on Unix it takes `SIGKILL` rather than the `SIGTERM`
/// that `terminate` sends, since a catchable signal is not a crash.
pub fn force_kill(pid: u32) {
    #[cfg(unix)]
    {
        let _ = Command::new("kill")
            .arg("-9")
            .arg(pid.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(windows)]
    {
        let _ = sessionmgr_proc::terminate(pid);
    }
}

/// Does a session's transcript contain `needle` anywhere in its output?
///
/// Decodes rather than grepping the file. Output events carry base64
/// payloads (session output is a terminal byte stream, not text), so a
/// raw substring search over `transcript.jsonl` silently never matches --
/// which reads as "the output never arrived" rather than "the test is
/// looking at it wrong".
pub fn transcript_contains(root: &Path, id: &str, needle: &str) -> bool {
    !transcript_output(root, id).is_empty()
        && String::from_utf8_lossy(&transcript_output(root, id)).contains(needle)
}

/// Every output byte a session has produced so far, decoded and
/// concatenated.
pub fn transcript_output(root: &Path, id: &str) -> Vec<u8> {
    let path = root.join("sessions").join(id).join("transcript.jsonl");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in text.lines() {
        // Only output events carry a payload; status and exit events have
        // no `data` field at all.
        if !line.contains("\"type\":\"output\"") {
            continue;
        }
        let needle = "\"data\":\"";
        let Some(start) = line.find(needle) else { continue };
        let rest = &line[start + needle.len()..];
        let Some(end) = rest.find('"') else { continue };
        if let Ok(decoded) = sessionmgr_protocol::base64::decode(&rest[..end]) {
            out.extend_from_slice(&decoded);
        }
    }
    out
}

/// Is `pid` a live process (a zombie does not count)?
pub fn is_alive(pid: u32) -> bool {
    sessionmgr_proc::is_alive(pid).unwrap_or(false)
}

/// Spawns `attach` and collects up to `max_lines` lines of its output,
/// then kills it. Attach is a long-lived stream, not a one-shot command,
/// so it cannot go through [`run`].
///
/// Reads stderr as well as stdout: status and recovery notices go to
/// stderr so they never contaminate a session's actual output, and
/// several tests are about exactly those notices.
pub fn attach_lines(root: &Path, id: &str, max_lines: usize, timeout: Duration) -> Vec<String> {
    let mut child = Command::new(bin())
        .arg("--state-root")
        .arg(root)
        .args(["attach", id])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn attach");

    let (tx, rx) = std::sync::mpsc::channel();
    for stream in [
        child.stdout.take().map(Streams::Out),
        child.stderr.take().map(Streams::Err),
    ]
    .into_iter()
    .flatten()
    {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let reader: Box<dyn BufRead> = match stream {
                Streams::Out(s) => Box::new(std::io::BufReader::new(s)),
                Streams::Err(s) => Box::new(std::io::BufReader::new(s)),
            };
            for line in reader.lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    return;
                }
            }
        });
    }
    drop(tx);

    let mut lines = Vec::new();
    let deadline = Instant::now() + timeout;
    while lines.len() < max_lines {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match rx.recv_timeout(remaining) {
            Ok(line) => lines.push(line),
            Err(_) => break,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    lines
}

enum Streams {
    Out(std::process::ChildStdout),
    Err(std::process::ChildStderr),
}

/// Polls `condition` until it holds or `timeout` elapses.
///
/// Polling rather than a fixed sleep: these tests coordinate real
/// processes across real sockets, and a sleep long enough to be reliable
/// on a loaded CI machine is far longer than the wait usually needs.
pub fn wait_until(mut condition: impl FnMut() -> bool, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if condition() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}
