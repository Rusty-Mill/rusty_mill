//! `deploy/backup.sh` and `deploy/restore-drill.sh` against a real data directory and the
//! real binary: a live store is refused, archives rotate and ship, the drill restores the
//! tasks, and a damaged archive fails the drill. Linux only (the scripts use flock).
#![cfg(target_os = "linux")]

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output};
use std::thread::sleep;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_rusty_tick");
const INBOX: &str = "00000000-0000-7000-8000-000000000001";

fn script(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("deploy")
        .join(name)
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .expect("free port")
        .port()
}

fn tick(data: &Path, args: &[&str]) -> String {
    let out = Command::new(BIN)
        .args(args)
        .arg("--data-dir")
        .arg(data)
        .output()
        .expect("rusty_tick");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .expect("utf-8")
        .trim()
        .to_owned()
}

/// A server on `data` until dropped.
struct Server(Child);

impl Server {
    fn start(data: &Path, port: u16) -> Self {
        let server = Self(
            Command::new(BIN)
                .arg("--data-dir")
                .arg(data)
                .args(["--addr", &format!("127.0.0.1:{port}")])
                .env_remove("RUSTY_TICK_TOKEN")
                .spawn()
                .expect("serve"),
        );
        for _ in 0..100 {
            let up = Command::new("curl")
                .args(["-fsS", &format!("http://127.0.0.1:{port}/health")])
                .output()
                .is_ok_and(|o| o.status.success());
            if up {
                return server;
            }
            sleep(Duration::from_millis(50));
        }
        panic!("rusty_tick did not start"); // `server` drops: killed and reaped
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn add_task(port: u16, token: &str, title: &str) {
    let out = Command::new("curl")
        .args(["-fsS", "-H", &format!("Authorization: Bearer {token}")])
        .args(["-H", "content-type: application/json", "-d"])
        .arg(format!(r#"{{"title":"{title}","listId":"{INBOX}"}}"#))
        .arg(format!("http://127.0.0.1:{port}/api/v1/tasks"))
        .output()
        .expect("curl");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn run(script_name: &str, env: &[(&str, &Path)], args: &[&str]) -> Output {
    let mut command = Command::new(script(script_name));
    command
        .args(args)
        .env_remove("RUSTY_TICK_TOKEN")
        .env("RUSTY_TICK_BIN", BIN)
        .env("RUSTY_TICK_DRILL_PORT", free_port().to_string())
        .env("RUSTY_TICK_BACKUP_KEEP", "2");
    for (key, value) in env {
        command.env(key, value);
    }
    command.output().expect("script")
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn archives(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn a_backup_ships_rotates_and_restores_and_a_damaged_one_fails_the_drill() {
    let root = tempfile::tempdir().expect("tempdir");
    let data = root.path().join("data");
    let local = root.path().join("backups");
    let off = root.path().join("offhost");
    let token = tick(&data, &["user", "add", "nano"]);
    tick(&data, &["user", "add", "gone"]);
    tick(&data, &["user", "disable", "gone"]);
    let port = free_port();
    let server = Server::start(&data, port);
    for title in ["one", "two", "three"] {
        add_task(port, &token, title);
    }
    let env = [
        ("RUSTY_TICK_DATA_DIR", data.as_path()),
        ("RUSTY_TICK_BACKUP_DIR", local.as_path()),
        ("RUSTY_TICK_BACKUP_DEST", off.as_path()),
    ];

    let live = run("backup.sh", &env, &["snapshot"]);
    assert!(!live.status.success());
    assert!(text(&live).contains("is held"), "{}", text(&live));
    drop(server);

    for _ in 0..3 {
        let out = run("backup.sh", &env, &[]);
        assert!(out.status.success(), "{}", text(&out));
        sleep(Duration::from_millis(1100)); // archive names are per second
    }
    let kept = archives(&local);
    assert_eq!(kept.len(), 4, "two archives and their sums: {kept:?}");
    assert_eq!(
        archives(&off),
        kept,
        "the off-host copy mirrors the local one"
    );

    let drill = run(
        "restore-drill.sh",
        &[("RUSTY_TICK_BACKUP_DIR", off.as_path())],
        &[],
    );
    assert!(drill.status.success(), "{}", text(&drill));
    assert!(
        text(&drill).contains("nano has 3 tasks"),
        "{}",
        text(&drill)
    );
    assert!(!text(&drill).contains(&token), "the drill prints no token");
    assert_eq!(
        tick(&data, &["user", "list"]).matches("drill").count(),
        0,
        "drill tokens never reach the live registry"
    );

    let newest = off.join(&kept[2]);
    let mut bytes = std::fs::read(&newest).expect("read");
    bytes.push(b'x');
    std::fs::write(&newest, bytes).expect("write");
    let damaged = run("restore-drill.sh", &[], &[newest.to_str().expect("utf-8")]);
    assert!(!damaged.status.success());
    assert!(
        text(&damaged).contains("does not match its sha256"),
        "{}",
        text(&damaged)
    );
}

#[test]
fn shipping_needs_a_destination_and_keep_must_be_a_count() {
    let root = tempfile::tempdir().expect("tempdir");
    let data = root.path().join("data");
    std::fs::create_dir_all(&data).expect("mkdir");
    let local = root.path().join("backups");
    let env = [
        ("RUSTY_TICK_DATA_DIR", data.as_path()),
        ("RUSTY_TICK_BACKUP_DIR", local.as_path()),
    ];
    let out = run("backup.sh", &env, &[]);
    assert!(!out.status.success());
    assert!(
        text(&out).contains("RUSTY_TICK_BACKUP_DEST"),
        "{}",
        text(&out)
    );

    let mut command = Command::new(script("backup.sh"));
    let out = command
        .env("RUSTY_TICK_DATA_DIR", &data)
        .env("RUSTY_TICK_BACKUP_DIR", &local)
        .env("RUSTY_TICK_BACKUP_KEEP", "0")
        .output()
        .expect("script");
    assert!(!out.status.success());
    assert!(
        text(&out).contains("positive whole number"),
        "{}",
        text(&out)
    );
}
