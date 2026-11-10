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

/// Run a deploy script with `env` on top of the test defaults (KEEP=2, a free drill port).
fn run(script_name: &str, env: &[(&str, &str)], args: &[&str]) -> Output {
    let mut command = Command::new(script(script_name));
    command
        .args(args)
        .env_remove("RUSTY_TICK_TOKEN")
        .env_remove("RUSTY_TICK_ADDR")
        .env("RUSTY_TICK_BIN", BIN)
        .env("RUSTY_TICK_DRILL_PORT", free_port().to_string())
        .env("RUSTY_TICK_BACKUP_KEEP", "2");
    for (key, value) in env {
        command.env(key, value);
    }
    command.output().expect("script")
}

fn path(p: &Path) -> &str {
    p.to_str().expect("utf-8 path")
}

/// Where `ship` puts this host's archives under a destination.
fn host_dir(dest: &Path) -> PathBuf {
    let out = Command::new("hostname")
        .arg("-s")
        .output()
        .expect("hostname");
    dest.join(String::from_utf8(out.stdout).expect("utf-8").trim())
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
    std::fs::create_dir(&off).expect("destination");
    let token = tick(&data, &["user", "add", "nano"]);
    tick(&data, &["user", "add", "gone"]);
    tick(&data, &["user", "disable", "gone"]);
    let port = free_port();
    let server = Server::start(&data, port);
    for title in ["one", "two", "three"] {
        add_task(port, &token, title);
    }
    let env = [
        ("RUSTY_TICK_DATA_DIR", path(&data)),
        ("RUSTY_TICK_BACKUP_DIR", path(&local)),
        ("RUSTY_TICK_BACKUP_DEST", path(&off)),
    ];

    let live = run("backup.sh", &env, &["snapshot"]);
    assert!(!live.status.success());
    assert!(text(&live).contains("is held"), "{}", text(&live));
    drop(server);

    // A multi-user server that has opened no user yet holds no store lock: its address gives it
    // away.
    let idle = Server::start(&data, port);
    let addr = format!("127.0.0.1:{port}");
    let mut with_addr = env.to_vec();
    with_addr.push(("RUSTY_TICK_ADDR", &addr));
    let live = run("backup.sh", &with_addr, &["snapshot"]);
    assert!(!live.status.success());
    assert!(text(&live).contains("answers on"), "{}", text(&live));
    drop(idle);

    for _ in 0..3 {
        let out = run("backup.sh", &env, &[]);
        assert!(out.status.success(), "{}", text(&out));
        sleep(Duration::from_millis(1100)); // archive names are per second
    }
    let shipped = host_dir(&off);
    let kept = archives(&local);
    assert_eq!(kept.len(), 4, "two archives and their sums: {kept:?}");
    assert_eq!(
        archives(&shipped),
        kept,
        "the off-host copy mirrors the local one"
    );

    // A wiped local directory must not prune the destination down to one archive.
    std::fs::remove_dir_all(&local).expect("wipe");
    let out = run("backup.sh", &env, &[]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("not pruning"), "{}", text(&out));
    assert_eq!(archives(&shipped).len(), 6, "{:?}", archives(&shipped));

    let drill = run(
        "restore-drill.sh",
        &[("RUSTY_TICK_BACKUP_DIR", path(&shipped))],
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

    // A busy drill port is named as such, not read as an empty backup.
    let busy = Server::start(&data, port);
    let newest = shipped.join(&kept[2]);
    let blocked = run(
        "restore-drill.sh",
        &[("RUSTY_TICK_DRILL_PORT", &port.to_string())],
        &[path(&newest)],
    );
    assert!(!blocked.status.success());
    assert!(
        text(&blocked).contains("already in use"),
        "{}",
        text(&blocked)
    );
    drop(busy);

    let mut bytes = std::fs::read(&newest).expect("read");
    bytes.push(b'x');
    std::fs::write(&newest, bytes).expect("write");
    let damaged = run("restore-drill.sh", &[], &[path(&newest)]);
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
        ("RUSTY_TICK_DATA_DIR", path(&data)),
        ("RUSTY_TICK_BACKUP_DIR", path(&local)),
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

#[test]
fn the_drill_fails_when_one_user_cannot_be_read_even_if_another_can() {
    let root = tempfile::tempdir().expect("tempdir");
    let data = root.path().join("data");
    let local = root.path().join("backups");
    let token = tick(&data, &["user", "add", "nano"]);
    let other = tick(&data, &["user", "add", "bob"]);
    let port = free_port();
    let server = Server::start(&data, port);
    add_task(port, &token, "kept");
    add_task(port, &other, "lost");
    drop(server);
    std::fs::write(data.join("users/bob/tasks.mmap"), b"not a store").expect("corrupt");

    let env = [
        ("RUSTY_TICK_DATA_DIR", path(&data)),
        ("RUSTY_TICK_BACKUP_DIR", path(&local)),
    ];
    let out = run("backup.sh", &env, &["snapshot"]);
    assert!(out.status.success(), "{}", text(&out));
    let drill = run("restore-drill.sh", &env, &[]);
    assert!(!drill.status.success(), "{}", text(&drill));
    assert!(
        text(&drill).contains("could not read bob's tasks"),
        "{}",
        text(&drill)
    );
}

#[test]
fn a_destination_that_cannot_be_listed_is_never_pruned() {
    let root = tempfile::tempdir().expect("tempdir");
    let data = root.path().join("data");
    let local = root.path().join("backups");
    let off = root.path().join("offhost");
    std::fs::create_dir(&off).expect("destination");
    tick(&data, &["user", "add", "nano"]);
    let env = [
        ("RUSTY_TICK_DATA_DIR", path(&data)),
        ("RUSTY_TICK_BACKUP_DIR", path(&local)),
        ("RUSTY_TICK_BACKUP_DEST", path(&off)),
    ];
    for _ in 0..2 {
        let out = run("backup.sh", &env, &[]);
        assert!(out.status.success(), "{}", text(&out));
        sleep(Duration::from_millis(1100)); // archive names are per second
    }

    // An rsync whose listing fails (ssh down, timeout) but whose copy works.
    let shim = root.path().join("bin");
    std::fs::create_dir(&shim).expect("bin");
    let real = Command::new("sh")
        .args(["-c", "command -v rsync"])
        .output()
        .expect("which rsync");
    let real = String::from_utf8(real.stdout).expect("utf-8");
    let fake = shim.join("rsync");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\nfor a in \"$@\"; do [ \"$a\" = --list-only ] && exit 255; done\nexec {} \"$@\"\n",
            real.trim()
        ),
    )
    .expect("shim");
    std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755))
        .expect("chmod");
    let search = format!("{}:{}", path(&shim), std::env::var("PATH").expect("PATH"));

    std::fs::remove_dir_all(&local).expect("wipe");
    let mut with_shim = env.to_vec();
    with_shim.push(("PATH", &search));
    let out = run("backup.sh", &with_shim, &[]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("could not list"), "{}", text(&out));
    assert_eq!(
        archives(&host_dir(&off)).len(),
        6,
        "{:?}",
        archives(&host_dir(&off))
    );
}
