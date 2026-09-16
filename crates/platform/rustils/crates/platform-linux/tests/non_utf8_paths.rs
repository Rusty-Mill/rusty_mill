//! Linux-only: does rustils' `OsStr`-only path boundary (D-11) actually
//! round-trip a non-UTF-8 byte sequence, the specific hard case
//! `RT-001` (Rusty-Mill `TRIAL-0002`) found no executable evidence for
//! in this repo's own suite — see `docs/02-capabilities/filesystem/
//! rustils-comparison.md` in the Rusty-Mill AKB. POSIX filenames are
//! arbitrary bytes excluding NUL and `/`; this asserts rustils actually
//! preserves that, not just that its trait signatures use `OsStr`.
//! `#![cfg(target_os = "linux")]`: integration test files don't inherit
//! the library crate's own platform gate — see `tun_parity.rs`'s
//! identical header note.
#![cfg(target_os = "linux")]

use std::os::unix::ffi::OsStrExt;
use std::{ffi::OsStr, ffi::OsString};

use platform::fs::{Dir, FileType, OpenOptions};

/// `0xFF` never appears in well-formed UTF-8 (max lead byte is `0xF4`);
/// mixed with ASCII so the entry is still recognizably a "file" if
/// something along the way *did* lossily substitute the invalid byte.
const NON_UTF8_NAME: &[u8] = b"file-\xFF-name";

#[test]
fn linux_open_round_trips_a_non_utf8_filename() {
    let tmp = std::env::temp_dir().join(format!("rustils-rt001-open-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("mk tempdir");
    let root = platform_linux::LinuxDir::open_ambient(&tmp).expect("open ambient");

    let name = OsStr::from_bytes(NON_UTF8_NAME);
    let create_new = OpenOptions {
        write: true,
        create: true,
        create_new: true,
        ..Default::default()
    };
    root.open(name, &create_new)
        .expect("create a file whose name is not valid UTF-8");

    // Round trip 1: open it back by the same non-UTF-8 name.
    root.open(name, &OpenOptions::read())
        .expect("re-open the same non-UTF-8 name");

    // Round trip 2: read_dir returns the exact bytes back, not a lossy
    // substitution (e.g. U+FFFD) and not a truncation at the invalid byte.
    let entries = root.read_dir().expect("read_dir");
    let found = entries
        .iter()
        .find(|e| e.file_type == FileType::File)
        .expect("the created file must appear in read_dir");
    assert_eq!(
        found.name.as_bytes(),
        NON_UTF8_NAME,
        "read_dir must return the exact bytes create_new was given, not a lossy or truncated substitute"
    );

    std::fs::remove_dir_all(&tmp).ok();
}

#[test]
fn linux_create_dir_and_rename_round_trip_a_non_utf8_name() {
    let tmp = std::env::temp_dir().join(format!("rustils-rt001-dir-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("mk tempdir");
    let root = platform_linux::LinuxDir::open_ambient(&tmp).expect("open ambient");

    let name = OsStr::from_bytes(NON_UTF8_NAME);
    root.create_dir(name)
        .expect("create a directory whose name is not valid UTF-8");
    root.open_dir(name)
        .expect("open_dir the same non-UTF-8 name back");

    // rename *to* a second non-UTF-8 name — exercises the boundary on
    // both the `from` and `to` sides, not just directory creation.
    let renamed: Vec<u8> = NON_UTF8_NAME.iter().chain(b"-2").copied().collect();
    let renamed_name = OsStr::from_bytes(&renamed);
    root.rename(name, renamed_name)
        .expect("rename to a second non-UTF-8 name");

    let entries = root.read_dir().expect("read_dir");
    let found = entries
        .iter()
        .find(|e| e.file_type == FileType::Dir)
        .expect("the renamed directory must appear in read_dir");
    assert_eq!(
        found.name.as_bytes(),
        renamed.as_slice(),
        "the renamed non-UTF-8 name must round-trip exactly through read_dir"
    );

    std::fs::remove_dir_all(&tmp).ok();
}

#[test]
fn linux_symlink_round_trips_a_non_utf8_target_and_link_name() {
    let tmp = std::env::temp_dir().join(format!("rustils-rt001-symlink-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("mk tempdir");
    let root = platform_linux::LinuxDir::open_ambient(&tmp).expect("open ambient");

    // Both the link's own name *and* the (unvalidated, opaque per
    // docs/behavior/fs.md) target text carry the non-UTF-8 byte.
    let link_name = OsStr::from_bytes(NON_UTF8_NAME);
    let target: Vec<u8> = b"target-".iter().chain(NON_UTF8_NAME).copied().collect();
    let target_os: OsString = OsStr::from_bytes(&target).to_os_string();

    root.symlink(&target_os, link_name)
        .expect("create a symlink with a non-UTF-8 name and target");

    let read_back = root.read_link(link_name).expect("read_link");
    assert_eq!(
        read_back.as_bytes(),
        target.as_slice(),
        "read_link must return the exact target bytes symlink was given"
    );

    std::fs::remove_dir_all(&tmp).ok();
}
