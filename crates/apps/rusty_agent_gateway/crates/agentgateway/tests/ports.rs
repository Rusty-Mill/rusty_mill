//! The test suite's own port allocator.
//!
//! Worth testing because when it is wrong it does not fail here: it fails
//! somewhere else, rarely, as `Address already in use` in whichever test drew
//! the port second.

mod common;

use common::{claim_band, free_port};

#[test]
fn two_claims_cannot_hold_the_same_band() {
    // The cross-process case is the one that matters, and this is a faithful
    // stand-in for it: `flock` conflicts between separate opens of a file even
    // within one process, so a second claim here fails exactly as another test
    // binary's would.
    let directory = tempfile::tempdir().expect("should make a directory");
    let (_first_lock, first) = claim_band(directory.path()).expect("should claim a band");
    let (_second_lock, second) = claim_band(directory.path()).expect("should claim another");
    assert_ne!(first, second);
}

#[test]
fn bands_do_not_overlap() {
    let directory = tempfile::tempdir().expect("should make a directory");
    let (_first_lock, first) = claim_band(directory.path()).expect("should claim a band");
    let (_second_lock, second) = claim_band(directory.path()).expect("should claim another");
    // Wide enough for the largest test binary here, which asks for a few
    // hundred ports.
    assert!(second.abs_diff(first) >= 1024, "{first} and {second}");
}

#[test]
fn a_claim_ends_with_the_process_that_made_it() {
    // Dropping the file is what a process exiting does to its lock, which is
    // why this needs no cleanup of its own: a killed run leaves no band that
    // later runs believe is taken.
    let directory = tempfile::tempdir().expect("should make a directory");
    let (lock, first) = claim_band(directory.path()).expect("should claim a band");
    drop(lock);

    let (_lock, again) = claim_band(directory.path()).expect("should reclaim it");
    assert_eq!(first, again);
}

#[test]
fn a_machine_with_no_band_left_says_so_rather_than_waiting() {
    // The caller falls back to an OS-assigned port on `None`. Blocking until a
    // band frees would hang the suite instead.
    let directory = tempfile::tempdir().expect("should make a directory");
    let mut held = Vec::new();
    while let Some(claim) = claim_band(directory.path()) {
        held.push(claim);
        assert!(held.len() <= 64, "the bands should be finite");
    }
    assert!(!held.is_empty());
}

#[tokio::test]
async fn a_port_is_handed_out_once_and_is_free_when_it_is() {
    let first = free_port().await;
    let second = free_port().await;
    assert_ne!(first, second);

    // Both are still bindable: the allocator does not hand back something it
    // is holding open itself.
    for port in [first, second] {
        tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .unwrap_or_else(|err| panic!("{port} should be free: {err}"));
    }
}
