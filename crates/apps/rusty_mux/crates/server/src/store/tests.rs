use std::collections::BTreeMap;

use rusty_mux_core::{Mux, Size};
use rusty_term_core::{AnsiParser, Grid};

use super::*;

/// A fresh store under the temp directory, removed when dropped.
struct Scratch(Store);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("rusty-mux-store-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        Self(Store::open(dir).unwrap())
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0.dir);
    }
}

fn one_block() -> (Manifest, BlockId) {
    let mut mux = Mux::new();
    let s = mux.create_session("work").unwrap();
    let w = mux.create_window(s, "main").unwrap();
    let block = mux.create_block(w, Size::new(20, 4).unwrap()).unwrap();
    let started = Started {
        shell: true,
        read_only: false,
        program: "/bin/sh".into(),
        args: Vec::new(),
        cwd: "/".into(),
    };
    let manifest = Manifest {
        saved: mux.save(),
        started: BTreeMap::from([(block, started)]),
    };
    (manifest, block)
}

fn screen(text: &[u8]) -> Vec<u8> {
    let (mut grid, mut parser) = (Grid::new(20, 4), AnsiParser::new());
    parser.advance(&mut grid, text);
    rusty_term_core::checkpoint::encode(&grid, &parser).unwrap()
}

#[test]
fn an_empty_store_holds_nothing() {
    let store = Scratch::new("empty");
    assert!(store.0.load().unwrap().is_none());
}

#[test]
fn saves_and_loads_the_manifest_and_screens() {
    let store = Scratch::new("round");
    let (manifest, block) = one_block();
    assert!(store.0.load().unwrap().is_none(), "a new store");
    store
        .0
        .save(1, &manifest, &[(block, screen(b"hello"))])
        .unwrap();
    let loaded = store.0.load().unwrap().unwrap();
    assert_eq!(loaded.manifest, manifest);
    let [(id, grid, _)] = loaded.screens.as_slice() else {
        panic!("one screen");
    };
    assert_eq!(*id, block);
    assert!(grid.line_text(0).unwrap().starts_with("hello"));
}

#[test]
fn a_bad_screen_restarts_blank_and_a_closed_block_loses_its_screen() {
    let store = Scratch::new("stale");
    let (manifest, block) = one_block();
    fs::write(store.0.screen(block), b"not RTCP").unwrap();
    assert!(store.0.load().unwrap().is_none());
    store.0.save(1, &manifest, &[]).unwrap();
    assert!(store.0.load().unwrap().unwrap().screens.is_empty());
    store.0.save(2, &Manifest::default(), &[]).unwrap();
    assert!(
        !store.0.screen(block).exists(),
        "closed block's screen removed"
    );
}

#[test]
fn a_spool_outlives_a_restart_until_its_block_is_gone() {
    let store = Scratch::new("spool");
    let (manifest, block) = one_block();
    store.0.load().unwrap();
    store.0.save(1, &manifest, &[]).unwrap();
    store
        .0
        .spool(block)
        .unwrap()
        .append(vec![b"line".to_vec()])
        .unwrap();
    let orphan = BlockId::from_uuid(rusty_uuid::Uuid::new_v4());
    store.0.spool(orphan).unwrap();
    store.0.load().unwrap();
    assert_eq!(store.0.spool(block).unwrap().lines(), 1, "kept");
    assert!(
        !store.0.dir.join(format!("{orphan}.spool")).exists(),
        "orphan removed"
    );
    store.0.save(2, &Manifest::default(), &[]).unwrap();
    store.0.load().unwrap();
    assert!(!store.0.dir.join(format!("{block}.spool")).exists());
}

#[test]
fn an_older_save_never_overwrites_a_newer_one() {
    let store = Scratch::new("order");
    let (manifest, _) = one_block();
    store.0.load().unwrap();
    store.0.save(2, &manifest, &[]).unwrap();
    store.0.save(1, &Manifest::default(), &[]).unwrap();
    assert_eq!(store.0.load().unwrap().unwrap().manifest, manifest);
}

#[test]
fn a_bad_manifest_is_an_error_and_can_be_set_aside() {
    let store = Scratch::new("bad");
    fs::write(store.0.dir.join(SESSIONS), b"not a slot file").unwrap();
    assert!(store.0.load().is_err());
    let aside = store.0.set_aside().unwrap();
    assert!(aside.join(SESSIONS).exists());
    assert!(!aside.join(LOCK).exists(), "the lock stays with the store");
    assert!(
        Store::open(store.0.dir.clone()).is_err(),
        "still locked: no second server takes the new store"
    );
    assert!(
        Store::open(aside.clone()).is_ok(),
        "the set-aside files are unlocked"
    );
    assert!(store.0.load().unwrap().is_none());
    fs::remove_dir_all(aside).unwrap();
}

#[test]
fn a_store_has_one_server_at_a_time() {
    let store = Scratch::new("lock");
    assert!(Store::open(store.0.dir.clone()).is_err(), "locked");
}

#[test]
fn many_saves_compact_and_still_load() {
    let store = Scratch::new("compact");
    store.0.load().unwrap();
    let (mut manifest, _) = one_block();
    for n in 1..=u64::from(COMPACT_EVERY) + 1 {
        manifest.saved.sessions[0].name = format!("work {n}");
        store.0.save(n, &manifest, &[]).unwrap();
    }
    drop(store.0.inner().unwrap().sessions.take());
    let loaded = store.0.load().unwrap().unwrap();
    assert_eq!(loaded.manifest, manifest);
}

#[cfg(unix)]
#[test]
fn the_directory_is_the_owners_alone() {
    use std::os::unix::fs::PermissionsExt;
    let store = Scratch::new("mode");
    let mode = fs::metadata(&store.0.dir).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o700);
}

/// Windows checks no parent's DACL to open a file, so a store's files are
/// restricted one by one, and a junction is refused.
#[cfg(windows)]
#[test]
fn a_windows_store_restricts_its_files_and_refuses_a_junction() {
    let dir = std::env::temp_dir().join(format!("rusty-mux-store-acl-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let old = dir.join("old.rtcp");
    fs::write(&old, b"x").unwrap();
    let grant = std::process::Command::new("icacls")
        .arg(&old)
        .args(["/grant", "*S-1-1-0:R"])
        .output()
        .unwrap();
    assert!(grant.status.success(), "{grant:?}");
    assert!(acl(&old).contains("Everyone"), "set up: {}", acl(&old));
    let store = Store::open(dir.clone()).unwrap();
    assert!(
        !acl(&old).contains("Everyone"),
        "existing file: {}",
        acl(&old)
    );
    let new = store.dir.join("new.rtcp");
    fs::write(&new, b"x").unwrap();
    assert_eq!(entries(&acl(&new)), 1, "new file: {}", acl(&new));
    drop(store);

    let target =
        std::env::temp_dir().join(format!("rusty-mux-store-shared-{}", std::process::id()));
    let junction =
        std::env::temp_dir().join(format!("rusty-mux-store-junction-{}", std::process::id()));
    let _ = fs::remove_dir_all(&junction);
    fs::create_dir_all(&target).unwrap();
    let made = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&junction)
        .arg(&target)
        .output()
        .unwrap();
    assert!(made.status.success(), "{made:?}");
    let refused = Store::open(junction.clone())
        .err()
        .expect("a junction is refused");
    assert!(refused.to_string().contains("junction"), "{refused}");
    assert!(!target.join(LOCK).exists(), "nothing written through it");
    let _ = fs::remove_dir(&junction);
    let _ = fs::remove_dir_all(&target);
    let _ = fs::remove_dir_all(&dir);
}

/// What `icacls` reports for `path`.
#[cfg(windows)]
fn acl(path: &Path) -> String {
    let out = std::process::Command::new("icacls")
        .arg(path)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The access entries in `icacls` output: each names an account then a colon.
#[cfg(windows)]
fn entries(acl: &str) -> usize {
    acl.lines().filter(|l| l.contains(":(")).count()
}
