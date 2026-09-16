//! Windows-only: does rustils' `OsStr`-only path boundary (D-11) actually
//! round-trip a lone (unpaired) UTF-16 surrogate, the specific hard case
//! `RT-001` (Rusty-Mill `TRIAL-0002`) found no executable evidence for
//! in this repo's own suite — see `docs/02-capabilities/filesystem/
//! rustils-comparison.md` in the Rusty-Mill AKB. NTFS stores filenames
//! as unvalidated UTF-16LE code-unit sequences (no requirement that
//! surrogate pairs be well-formed); Rust's `OsString` on Windows uses
//! WTF-8 internally specifically so a lone surrogate survives a
//! round-trip losslessly. `sys::nt::to_wide_nt_component` passes
//! `encode_wide()`'s output straight through with no validation, so
//! the claim should hold — this test is the executable evidence for
//! that, not an assumption.
//!
//! `#![cfg(windows)]`: integration test files don't inherit the
//! library crate's own platform gate — see `parity.rs`'s identical
//! header note; without it, a non-Windows `--workspace` build tries to
//! compile this file against the host's plain (non-Windows) `OsString`,
//! which has neither `OsStrExt`/`OsStringExt` nor `platform_windows`'s
//! `#[cfg(windows)]`-gated `WindowsDir` re-export.
#![cfg(windows)]

use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};

use platform::fs::{Dir, FileType, OpenOptions};

/// `0xD800` is the first UTF-16 high surrogate; alone (no following low
/// surrogate in `0xDC00..=0xDFFF`), it is not valid UTF-16 and cannot be
/// represented by a plain Rust `String` at all — exactly the case
/// `OsString`'s WTF-8 encoding exists to still carry.
fn lone_surrogate_name(suffix: &str) -> OsString {
    let mut units: Vec<u16> = "file-".encode_utf16().collect();
    units.push(0xD800);
    units.extend(format!("-{suffix}").encode_utf16());
    OsString::from_wide(&units)
}

#[test]
fn windows_open_round_trips_a_lone_surrogate_filename() {
    let tmp = std::env::temp_dir().join(format!("rustils-rt001-open-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("mk tempdir");
    let root = platform_windows::WindowsDir::open_ambient(&tmp).expect("open ambient");

    let name = lone_surrogate_name("name");
    let create_new = OpenOptions {
        write: true,
        create: true,
        create_new: true,
        ..Default::default()
    };
    root.open(&name, &create_new)
        .expect("create a file whose name contains a lone UTF-16 surrogate");

    // Round trip 1: open it back by the same lone-surrogate name.
    root.open(&name, &OpenOptions::read())
        .expect("re-open the same lone-surrogate name");

    // Round trip 2: read_dir returns the exact code units back, not a
    // lossy substitution (e.g. U+FFFD) and not a truncation at the
    // unpaired surrogate.
    let entries = root.read_dir().expect("read_dir");
    let found = entries
        .iter()
        .find(|e| e.file_type == FileType::File)
        .expect("the created file must appear in read_dir");
    let expected: Vec<u16> = name.encode_wide().collect();
    let actual: Vec<u16> = found.name.encode_wide().collect();
    assert_eq!(
        actual, expected,
        "read_dir must return the exact UTF-16 code units create_new was given, including the lone surrogate"
    );

    std::fs::remove_dir_all(&tmp).ok();
}

#[test]
fn windows_create_dir_and_rename_round_trip_a_lone_surrogate_name() {
    let tmp = std::env::temp_dir().join(format!("rustils-rt001-dir-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("mk tempdir");
    let root = platform_windows::WindowsDir::open_ambient(&tmp).expect("open ambient");

    let name = lone_surrogate_name("dir");
    root.create_dir(&name)
        .expect("create a directory whose name contains a lone UTF-16 surrogate");
    root.open_dir(&name)
        .expect("open_dir the same lone-surrogate name back");

    let renamed = lone_surrogate_name("dir-renamed");
    root.rename(&name, &renamed)
        .expect("rename to a second lone-surrogate name");

    let entries = root.read_dir().expect("read_dir");
    let found = entries
        .iter()
        .find(|e| e.file_type == FileType::Dir)
        .expect("the renamed directory must appear in read_dir");
    let expected: Vec<u16> = renamed.encode_wide().collect();
    let actual: Vec<u16> = found.name.encode_wide().collect();
    assert_eq!(
        actual, expected,
        "the renamed lone-surrogate name must round-trip exactly through read_dir"
    );

    std::fs::remove_dir_all(&tmp).ok();
}
