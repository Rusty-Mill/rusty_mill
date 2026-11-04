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
    store.0.save(1, &manifest, &[]).unwrap();
    assert!(store.0.load().unwrap().unwrap().screens.is_empty());
    store.0.save(2, &Manifest::default(), &[]).unwrap();
    assert!(
        !store.0.screen(block).exists(),
        "closed block's screen removed"
    );
}

#[test]
fn an_older_save_never_overwrites_a_newer_one() {
    let store = Scratch::new("order");
    let (manifest, _) = one_block();
    store.0.save(2, &manifest, &[]).unwrap();
    store.0.save(1, &Manifest::default(), &[]).unwrap();
    assert_eq!(store.0.load().unwrap().unwrap().manifest, manifest);
}

#[test]
fn a_bad_manifest_is_an_error_and_can_be_set_aside() {
    let store = Scratch::new("bad");
    fs::write(store.0.dir.join(MANIFEST), "rusty_mux state 9\n").unwrap();
    assert!(store.0.load().is_err());
    let aside = store.0.set_aside().unwrap();
    assert!(aside.join(MANIFEST).exists());
    assert!(store.0.load().unwrap().is_none());
    fs::remove_dir_all(aside).unwrap();
}

#[cfg(unix)]
#[test]
fn the_directory_is_the_owners_alone() {
    use std::os::unix::fs::PermissionsExt;
    let store = Scratch::new("mode");
    let mode = fs::metadata(&store.0.dir).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o700);
}
