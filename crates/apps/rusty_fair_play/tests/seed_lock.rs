//! `rusty_fair_play seed` is a second process writing the same stores a
//! running service holds in memory, so it must be refused while the
//! directory is held, and must leave every file as it found it.

use rusty_fair_play::service::Service;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_rusty_fair_play");

fn seed(dir: &Path) -> Output {
    Command::new(BIN)
        .args(["seed", "--data-dir"])
        .arg(dir)
        .output()
        .expect("the binary runs")
}

/// Every file under `dir` with its length and bytes' checksum.
fn contents(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap())
        .filter(|e| e.file_name() != "store.lock")
        .map(|e| {
            (
                e.file_name().to_string_lossy().into_owned(),
                std::fs::read(e.path()).unwrap(),
            )
        })
        .collect()
}

#[test]
fn seed_is_refused_while_a_service_holds_the_directory() {
    let dir = tempfile::tempdir().unwrap();
    let mut service = Service::open(dir.path()).unwrap();
    service.create_person("Ada").unwrap();
    let before = contents(dir.path());

    let out = seed(dir.path());
    assert!(
        !out.status.success(),
        "seed must fail while the service is open"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("in use by another process"), "{stderr}");
    assert_eq!(contents(dir.path()), before, "no file changed");

    // The service's view is intact and still accepts writes.
    assert_eq!(service.snapshot().unwrap().people.len(), 1);
    drop(service);

    // Once the directory is free the same command succeeds.
    let out = seed(dir.path());
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let reopened = Service::open(dir.path()).unwrap();
    assert_eq!(reopened.snapshot().unwrap().cards.len(), 100);
}
