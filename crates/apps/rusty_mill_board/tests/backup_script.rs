//! `deploy/backup.sh` against synthetic databases and a disposable local destination: copies
//! are verified pairs, retention counts only pairs, a failed checksum or shipment never costs
//! the last good copies, a wiped local directory never prunes the destination, and an absent
//! mount is refused. Linux only; needs bash, python3 and rsync (a missing tool fails the test).
#![cfg(target_os = "linux")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use uuid::Uuid;

struct Rig {
    root: PathBuf,
}

impl Rig {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("mill_backup_{}", Uuid::new_v4()));
        fs::create_dir_all(root.join("dest")).expect("rig dir"); // the target exists, as a mount does
        let db = root.join("board.db");
        let made = Command::new("python3")
            .args(["-I", "-c", "import sqlite3,sys;c=sqlite3.connect(sys.argv[1]);c.execute('pragma journal_mode=wal');c.execute('create table t(x)');c.execute('insert into t values(42)');c.commit()"])
            .arg(&db)
            .status()
            .expect("python3");
        assert!(made.success());
        Self { root }
    }

    fn local(&self) -> PathBuf {
        self.root.join("local")
    }

    fn dest(&self) -> PathBuf {
        self.root.join("dest")
    }

    /// `backup.sh <mode>` with `keep` copies, and `extra` env pairs; `bin` is put first on PATH.
    fn run(&self, mode: &str, keep: u32, bin: Option<&Path>, extra: &[(&str, &str)]) -> Output {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("deploy/backup.sh");
        let path = match bin {
            Some(b) => format!("{}:/usr/bin:/bin", b.display()),
            None => "/usr/bin:/bin".into(),
        };
        Command::new("bash")
            .arg(script)
            .arg(mode)
            .env_clear()
            .env("PATH", path)
            .env("MILL_BOARD_DB", self.root.join("board.db"))
            .env("MILL_BOARD_BACKUP_DIR", self.local())
            .env("MILL_BOARD_BACKUP_KEEP", keep.to_string())
            .env("MILL_BOARD_BACKUP_DEST", self.dest())
            .envs(extra.iter().copied())
            .output()
            .expect("bash")
    }

    /// A directory with `name` as an executable shell script.
    fn fake(&self, name: &str, body: &str) -> PathBuf {
        let bin = self.root.join(format!("bin_{name}"));
        fs::create_dir_all(&bin).expect("bin");
        let file = bin.join(name);
        fs::write(&file, format!("#!/bin/sh\n{body}\n")).expect("fake");
        Command::new("chmod")
            .arg("755")
            .arg(&file)
            .status()
            .expect("chmod");
        bin
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).ok();
    }
}

fn ok(out: &Output) {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

fn pairs(dir: &Path) -> Vec<String> {
    names(dir)
        .into_iter()
        .filter(|n| n.ends_with(".sqlite3.gz"))
        .collect()
}

fn dest_host(rig: &Rig) -> PathBuf {
    let host = Command::new("hostname")
        .arg("-s")
        .output()
        .expect("hostname");
    rig.dest()
        .join(String::from_utf8_lossy(&host.stdout).trim())
}

fn snap(rig: &Rig, keep: u32) {
    ok(&rig.run("snapshot", keep, None, &[]));
    std::thread::sleep(std::time::Duration::from_millis(1100)); // names are per second
}

#[test]
fn snapshot_is_a_verified_pair_and_retention_keeps_the_newest() {
    let rig = Rig::new();
    for _ in 0..3 {
        snap(&rig, 2);
    }
    let kept = pairs(&rig.local());
    assert_eq!(kept.len(), 2, "{:?}", names(&rig.local()));
    assert_eq!(
        names(&rig.local()).len(),
        4,
        "gzip + checksum each, nothing else"
    );
    let check = Command::new("sha256sum")
        .arg("-c")
        .arg(format!("{}.sha256", kept[1]))
        .current_dir(rig.local())
        .output()
        .expect("sha256sum");
    assert!(check.status.success());
    let restored = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "gunzip -c {} > r.db && python3 -I -c \"import sqlite3;print(sqlite3.connect('r.db').execute('select x from t').fetchone()[0])\"",
            kept[1]
        ))
        .current_dir(rig.local())
        .output()
        .expect("restore");
    assert_eq!(String::from_utf8_lossy(&restored.stdout).trim(), "42");
}

#[test]
fn a_missing_or_corrupt_database_leaves_the_good_copies() {
    let rig = Rig::new();
    snap(&rig, 3);
    let before = names(&rig.local());
    fs::write(
        rig.root.join("board.db"),
        b"not a database at all, just text",
    )
    .expect("corrupt");
    let out = rig.run("snapshot", 3, None, &[]);
    assert!(!out.status.success());
    assert_eq!(names(&rig.local()), before);
}

#[test]
fn a_failed_checksum_publishes_nothing_and_costs_no_copy() {
    let rig = Rig::new();
    snap(&rig, 1);
    let before = names(&rig.local());
    let bin = rig.fake("sha256sum", "exit 1");
    let out = rig.run("snapshot", 1, Some(&bin), &[]);
    assert!(!out.status.success());
    assert_eq!(
        names(&rig.local()),
        before,
        "the one good pair is still there, nothing else"
    );
}

#[test]
fn a_gzip_without_its_checksum_is_not_a_copy() {
    let rig = Rig::new();
    snap(&rig, 1);
    let stray = rig.local().join("mill-board-29990101T000000Z.sqlite3.gz");
    fs::write(&stray, b"half").expect("stray");
    // It sorts newest: retention must not count it, and the next snapshot removes it.
    ok(&rig.run("ship", 1, None, &[]));
    let sent = names(&dest_host(&rig));
    assert!(sent.iter().all(|n| !n.contains("29990101")), "{sent:?}");
    snap(&rig, 1);
    assert!(!stray.exists());
    assert_eq!(pairs(&rig.local()).len(), 1);
}

#[test]
fn ship_mirrors_the_newest_and_prunes_the_destination() {
    let rig = Rig::new();
    snap(&rig, 3);
    ok(&rig.run("ship", 3, None, &[]));
    assert_eq!(pairs(&dest_host(&rig)).len(), 1);
    snap(&rig, 3);
    snap(&rig, 3);
    ok(&rig.run("ship", 3, None, &[]));
    assert_eq!(pairs(&dest_host(&rig)).len(), 3);
    snap(&rig, 1);
    ok(&rig.run("ship", 1, None, &[]));
    assert_eq!(
        pairs(&dest_host(&rig)).len(),
        1,
        "keep=1 prunes the destination"
    );
}

#[test]
fn a_wiped_local_directory_never_prunes_the_destination() {
    let rig = Rig::new();
    for _ in 0..3 {
        snap(&rig, 3);
    }
    ok(&rig.run("ship", 3, None, &[]));
    fs::remove_dir_all(rig.local()).expect("wipe");
    snap(&rig, 3);
    let out = rig.run("ship", 3, None, &[]);
    ok(&out);
    assert_eq!(pairs(&dest_host(&rig)).len(), 4, "the old three stay");
    assert!(String::from_utf8_lossy(&out.stderr).contains("not pruning"));
}

#[test]
fn a_failed_shipment_prunes_nothing() {
    let rig = Rig::new();
    for _ in 0..2 {
        snap(&rig, 1);
        ok(&rig.run("ship", 1, None, &[]));
    }
    let sent = names(&dest_host(&rig));
    snap(&rig, 1);
    // Every rsync call fails: the first (adding) pass dies before any prune.
    let bin = rig.fake("rsync", "exit 23");
    let out = rig.run("ship", 1, Some(&bin), &[]);
    assert!(!out.status.success());
    assert_eq!(
        names(&dest_host(&rig)),
        sent,
        "last known-good copy stays, nothing else changes"
    );
    // Only the pruning pass (--delete) fails: the new pair has landed, nothing was deleted.
    let bin = rig.fake(
        "rsync",
        "for a in \"$@\"; do [ \"$a\" = --delete ] && exit 23; done; exec /usr/bin/rsync \"$@\"",
    );
    let _ = rig.run("ship", 1, Some(&bin), &[]);
    assert!(
        pairs(&dest_host(&rig)).len() >= 2,
        "nothing pruned before the new copy landed"
    );
}

#[test]
fn an_absent_mount_is_refused() {
    let rig = Rig::new();
    snap(&rig, 1);
    let missing = rig.root.join("not_mounted");
    let out = rig.run(
        "ship",
        1,
        None,
        &[(
            "MILL_BOARD_BACKUP_REQUIRE_MOUNT",
            missing.to_str().expect("utf-8"),
        )],
    );
    assert!(!out.status.success());
    assert!(!dest_host(&rig).exists(), "nothing shipped");
}
