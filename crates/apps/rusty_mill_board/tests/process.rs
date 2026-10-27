//! The real binary as a child process: retired settings refused at start, health, the first
//! credential kept off stderr, and a clean stop on SIGTERM.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// The only parent variables a child board gets: what the OS needs to run it and open sockets.
/// Everything else (every `MILL_BOARD_*`, `GITHUB_TOKEN`, `RUSTY_TICK_TOKEN`, ...) stays out, so
/// a developer's or CI's environment cannot point the board at real settings, credentials or a
/// real GitHub sync. Names are matched without case, as Windows does.
const PASSED_ENV: [&str; 5] = ["PATH", "SYSTEMROOT", "TEMP", "TMP", "TMPDIR"];

/// The parent variables a child board keeps.
fn passed(parent: impl IntoIterator<Item = (OsString, OsString)>) -> Vec<(OsString, OsString)> {
    parent
        .into_iter()
        .filter(|(name, _)| {
            name.to_str().is_some_and(|name| {
                PASSED_ENV
                    .iter()
                    .any(|kept| kept.eq_ignore_ascii_case(name))
            })
        })
        .collect()
}

fn dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mill_process_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    dir
}

fn get(port: u16, path: &str) -> Option<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut out = String::new();
    stream.read_to_string(&mut out).ok()?;
    Some(out)
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port()
}

/// An HTTP board with its settings in `settings`, and none of the caller's board environment.
fn board(settings: &Path, port: u16) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rusty_mill_board"));
    command
        .env_clear()
        .envs(passed(std::env::vars_os()))
        .arg("--settings-dir")
        .arg(settings)
        .arg("--http")
        .arg(format!("127.0.0.1:{port}"))
        .stderr(Stdio::piped());
    command
}

/// A polluted parent environment reaches no child board: settings files, credentials, the
/// GitHub sync and the store are all dropped, and only what the OS needs is kept.
#[test]
fn a_child_board_gets_none_of_the_parents_board_settings_or_credentials() {
    let polluted = [
        ("MILL_BOARD_IDENTITIES", "/sentinel/identities.json"),
        ("MILL_BOARD_SETTINGS_DIR", "/sentinel/settings"),
        ("MILL_BOARD_GITHUB_CONFIG", "/sentinel/github.json"),
        ("MILL_BOARD_GITHUB_REPO", "sentinel/repo"),
        ("MILL_BOARD_GITHUB_API", "http://127.0.0.1:9"),
        ("GITHUB_TOKEN", "sentinel-token"),
        ("MILL_BOARD_TICK_URL", "http://127.0.0.1:9"),
        ("RUSTY_TICK_TOKEN", "sentinel-token"),
        ("MILL_BOARD_STORE", "/sentinel/board.mmap"),
        ("MILL_BOARD_TOKEN", "sentinel-token"),
        ("Path", "/usr/bin"),
        ("TMPDIR", "/tmp"),
    ];
    let kept = passed(polluted.map(|(k, v)| (OsString::from(k), OsString::from(v))));
    let names: Vec<_> = kept.iter().map(|(k, _)| k.to_string_lossy()).collect();
    assert_eq!(names, ["Path", "TMPDIR"]);
}

/// A service still configured for the removed board file must not start an empty in-memory board
/// in its place: it stops at once, says what to do, and has touched nothing.
#[test]
fn a_retired_store_or_backup_setting_stops_the_start_before_anything_opens() {
    for retired in [
        "MILL_BOARD_STORE",
        "MILL_BOARD_BACKUP_DIR",
        "MILL_BOARD_BACKUP_SECS",
        "MILL_BOARD_BACKUP_KEEP",
    ] {
        let dir = dir("retired");
        let (settings, port) = (dir.join("settings"), free_port());
        let out = board(&settings, port)
            .env(retired, dir.join("board.mmap"))
            .output()
            .expect("run");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{retired}: started anyway");
        assert!(
            stderr.contains(retired) && stderr.contains("MILL_BOARD_TICK_URL"),
            "{stderr}"
        );
        assert!(!settings.exists(), "{retired}: settings were created");
        assert!(!dir.join("board.mmap").exists());
        assert!(
            get(port, "/health").is_none(),
            "{retired}: something listens"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}

/// Signals and file modes are Unix.
#[cfg(unix)]
#[test]
fn the_server_reports_health_keeps_its_first_credential_off_stderr_and_stops_on_sigterm() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::Instant;

    let dir = dir("serve");
    let (settings, port) = (dir.join("settings"), free_port());
    let mut server = board(&settings, port).spawn().expect("spawn");

    let up = Instant::now() + Duration::from_secs(20);
    let health = loop {
        match get(port, "/health") {
            Some(r) if r.starts_with("HTTP/1.1 200") => break r,
            _ if Instant::now() > up => panic!("never became healthy"),
            _ => std::thread::sleep(Duration::from_millis(100)),
        }
    };
    assert!(health.contains(r#""ok":true"#), "{health}");

    // The first admin's credential is in an owner-only file, not in the log.
    let file = settings.join("first-admin.token");
    let mode = std::fs::metadata(&file)
        .expect("credential file")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);

    let killed = Command::new("kill")
        .args(["-TERM", &server.id().to_string()])
        .status()
        .expect("kill");
    assert!(killed.success());
    let end = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = server.try_wait().expect("wait") {
            break status;
        }
        assert!(Instant::now() < end, "did not exit on SIGTERM");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(status.success(), "{status:?}");

    let mut log = String::new();
    server
        .stderr
        .take()
        .expect("stderr")
        .read_to_string(&mut log)
        .expect("read");
    let secret = std::fs::read_to_string(&file).expect("read credential");
    assert!(
        !secret.trim().is_empty() && !log.contains(secret.trim()),
        "{log}"
    );
    std::fs::remove_dir_all(&dir).ok();
}
