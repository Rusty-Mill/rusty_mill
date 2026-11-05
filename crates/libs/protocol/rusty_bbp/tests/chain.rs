#![allow(clippy::expect_used, clippy::unwrap_used)]
//! #722: the hash-chained log. Every load checks the chain, so an edited,
//! removed, reordered or inserted line is refused; `verify_log` reports the
//! head without touching the file.
mod common;
use common::*;
use rusty_bbp::fs_store::{check, verify_log, GENESIS};
use rusty_bbp::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static N: AtomicU64 = AtomicU64::new(0);

fn tempdir() -> PathBuf {
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rusty_bbp_chain_{}_{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// A task in the build stage, written to disk; returns its log path.
fn built(dir: &Path) -> PathBuf {
    let mut fx = Fx::with_store(FsStore::open(dir).expect("open"));
    fx.reach_build();
    fs_store::log_path_for(dir, &fx.st().task.clone())
}

fn lines(path: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(path).expect("read log");
    text.lines().map(str::to_owned).collect()
}

fn write_lines(path: &Path, lines: &[String]) {
    let mut text = lines.join("\n");
    text.push('\n');
    std::fs::write(path, text).expect("write log");
}

/// A named edit to a log's lines.
type Tamper = (&'static str, fn(&mut Vec<String>));

/// The open error's text, or a panic if the store opened.
fn refused(dir: &Path) -> String {
    match FsStore::open(dir) {
        Ok(_) => panic!("a tampered log must not open"),
        Err(e) => e.to_string(),
    }
}

/// A legacy line: the bare events array of a chained one.
fn unchain(line: &str) -> String {
    let start = line.find("\"events\":").expect("chained") + "\"events\":".len();
    line[start..line.len() - 1].to_owned()
}

#[test]
fn a_new_log_is_chained_from_genesis() {
    let dir = tempdir();
    let log = built(&dir);
    let ls = lines(&log);
    assert!(ls.len() > 2, "several batches");
    assert!(ls[0].starts_with(&format!("{{\"prev\":\"{}\"", GENESIS.hex())));
    for pair in ls.windows(2) {
        let prev = Sha256::of(pair[0].as_bytes()).hex();
        assert!(pair[1].starts_with(&format!("{{\"prev\":\"{prev}\"")));
    }
    let chain = verify_log(&dir, &TaskId("T1".into())).expect("verify");
    assert_eq!(chain.hashes.len(), ls.len());
    assert_eq!(chain.unchained, 0);
    assert_eq!(chain.head(), Sha256::of(ls[ls.len() - 1].as_bytes()));
    assert!(!chain.partial);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_empty_log_checks_with_a_genesis_head() {
    let chain = check(b"").expect("empty");
    assert_eq!(
        (chain.head(), chain.next, chain.partial),
        (GENESIS, GENESIS, false)
    );
}

#[test]
fn a_legacy_log_loads_and_its_first_chained_line_covers_the_prefix() {
    let dir = tempdir();
    let log = built(&dir);
    let legacy: Vec<String> = lines(&log).iter().map(|l| unchain(l)).collect();
    write_lines(&log, &legacy);
    let prefix = std::fs::read(&log).expect("read");
    let mut fx = Fx::with_store(FsStore::open(&dir).expect("legacy log opens"));
    fx.d.reload().expect("reload");
    posted(fx.post(Who::Coder, ask(Who::Tester, "Which 429 cases matter?")));
    let after = lines(&log);
    let first = &after[legacy.len()];
    assert!(first.starts_with(&format!("{{\"prev\":\"{}\"", Sha256::of(&prefix).hex())));
    let chain = verify_log(&dir, &TaskId("T1".into())).expect("verify");
    assert_eq!(chain.unchained, legacy.len());
    drop(fx);
    // An edit inside the legacy prefix now breaks the chain too.
    let mut edited = after.clone();
    edited[0] = edited[0].replacen("T1", "T2", 1);
    write_lines(&log, &edited);
    assert!(refused(&dir).contains("chain broken"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn edited_removed_reordered_or_inserted_lines_are_refused() {
    let tampers: [Tamper; 4] = [
        ("edit", |ls| ls[0] = ls[0].replacen("T1", "T2", 1)),
        ("remove", |ls| {
            ls.remove(1);
        }),
        ("reorder", |ls| ls.swap(1, 2)),
        ("insert", |ls| {
            let copy = ls[1].clone();
            ls.insert(2, copy);
        }),
    ];
    for (name, tamper) in tampers {
        let dir = tempdir();
        let log = built(&dir);
        let mut ls = lines(&log);
        tamper(&mut ls);
        write_lines(&log, &ls);
        let err = refused(&dir);
        assert!(err.contains("chain broken"), "{name}: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn blank_lines_carriage_returns_and_legacy_lines_inside_the_chain_are_refused() {
    let tampers: [Tamper; 3] = [
        ("blank", |ls| ls.insert(1, String::new())),
        ("cr", |ls| ls[1].push('\r')),
        ("legacy", |ls| {
            let tail = unchain(&ls[ls.len() - 1]);
            ls.push(tail);
        }),
    ];
    for (name, tamper) in tampers {
        let dir = tempdir();
        let log = built(&dir);
        let mut ls = lines(&log);
        tamper(&mut ls);
        write_lines(&log, &ls);
        let err = refused(&dir);
        assert!(err.contains("line"), "{name}: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn verify_reports_a_partial_line_and_leaves_it_in_place() {
    let dir = tempdir();
    let log = built(&dir);
    let mut bytes = std::fs::read(&log).expect("read");
    bytes.extend_from_slice(b"{\"prev\":");
    std::fs::write(&log, &bytes).expect("write");
    let chain = verify_log(&dir, &TaskId("T1".into())).expect("verify");
    assert!(chain.partial);
    assert_eq!(
        std::fs::read(&log).expect("read"),
        bytes,
        "verify never repairs"
    );
    // Load still repairs it, as a crash mid-write.
    FsStore::open(&dir).expect("open repairs");
    assert!(
        !verify_log(&dir, &TaskId("T1".into()))
            .expect("verify")
            .partial
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The final line has no successor whose `prev` would catch a bad envelope,
/// so the envelope itself is checked. Load and verify refuse the same lines.
#[test]
fn a_malformed_final_envelope_is_refused_by_load_and_verify() {
    let tampers: [Tamper; 6] = [
        ("brace replaced", |ls| {
            let last = ls.last_mut().unwrap();
            last.pop();
            last.push('x');
        }),
        ("brace omitted", |ls| {
            ls.last_mut().unwrap().pop();
        }),
        ("trailing junk", |ls| {
            ls.last_mut().unwrap().push_str("junk")
        }),
        ("extra brace", |ls| ls.last_mut().unwrap().push('}')),
        ("junk inside", |ls| {
            let last = ls.last_mut().unwrap();
            last.insert(last.len() - 1, 'x');
        }),
        ("empty events", |ls| {
            let last = ls.last_mut().unwrap();
            let at = last.find("\"events\":").unwrap() + "\"events\":".len();
            last.replace_range(at..last.len() - 1, "");
        }),
    ];
    for one_line in [false, true] {
        for (name, tamper) in tampers {
            let dir = tempdir();
            let log = built(&dir);
            let mut ls = lines(&log);
            if one_line {
                ls.truncate(1);
            }
            tamper(&mut ls);
            write_lines(&log, &ls);
            let err = refused(&dir);
            assert!(err.contains("line"), "{name} (one line: {one_line}): {err}");
            assert!(
                verify_log(&dir, &TaskId("T1".into())).is_err(),
                "{name} (one line: {one_line}): verify must refuse what load refuses"
            );
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

#[test]
fn a_one_line_chained_log_loads_and_verifies() {
    let dir = tempdir();
    let log = built(&dir);
    let mut ls = lines(&log);
    ls.truncate(1);
    write_lines(&log, &ls);
    FsStore::open(&dir).expect("a one-line chained log opens");
    let chain = verify_log(&dir, &TaskId("T1".into())).expect("verify");
    assert_eq!((chain.hashes.len(), chain.unchained), (1, 0));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Legacy lines were read with `str::lines`, which drops a trailing `\r`,
/// and blank lines were skipped. A legacy prefix written with CRLF endings
/// and a blank CRLF line still loads, and chaining starts after it.
#[test]
fn a_legacy_crlf_prefix_with_a_blank_line_still_loads() {
    let dir = tempdir();
    let log = built(&dir);
    let legacy: Vec<String> = lines(&log).iter().map(|l| unchain(l)).collect();
    let mut text = String::new();
    for (i, l) in legacy.iter().enumerate() {
        text.push_str(l);
        text.push_str("\r\n");
        if i == 0 {
            text.push_str("\r\n");
        }
    }
    std::fs::write(&log, &text).expect("write");
    let mut fx = Fx::with_store(FsStore::open(&dir).expect("a CRLF legacy log opens"));
    fx.d.reload().expect("reload");
    posted(fx.post(Who::Coder, ask(Who::Tester, "Which 429 cases matter?")));
    let chain = verify_log(&dir, &TaskId("T1".into())).expect("verify");
    assert_eq!(
        chain.unchained,
        legacy.len() + 1,
        "the blank line counts as legacy"
    );
    drop(fx);
    FsStore::open(&dir).expect("reopens after chaining");
    let _ = std::fs::remove_dir_all(&dir);
}
