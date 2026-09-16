#![cfg(target_os = "linux")]
// io-uring-fs is a Cargo feature, not a target predicate -- required-features
// can't express "only on Linux", so a plain `--features io-uring-fs` build on
// another OS still tries to compile this file against items `src/io/mod.rs`
// only re-exports under `cfg(target_os = "linux")`. This file-level cfg is what
// actually keeps it Linux-only.
//! #256: `global_driver` is the only way to obtain a real, production
//! `Arc<dyn OpDriver>` from outside this crate -- `IoUringDriver` has no
//! public constructor of its own, by design (see its docs). This
//! exercises the acceptance criteria from that issue directly: the
//! function is callable from an external crate (this is an integration
//! test, compiled separately), and the singleton invariant holds across
//! repeated calls.

use rusty_tokio::io::{uring_global_driver, OpDriver};
use std::sync::Arc;

#[test]
fn global_driver_returns_the_same_singleton_instance_every_call() {
    let a = uring_global_driver().unwrap();
    let b = uring_global_driver().unwrap();
    assert!(
        Arc::ptr_eq(&a, &b),
        "global_driver must hand back the same process-wide instance, not a fresh one"
    );
}

/// The returned `Arc<dyn OpDriver>` is usable directly by
/// `OpDriver`-generic code, without going through [`UringFile`] at all --
/// the exact shape `rusty_stream`'s `Segment::create_on`/`open_on` need.
#[test]
fn global_driver_is_usable_directly_as_an_op_driver() {
    let rt = rusty_tokio::Runtime::new().unwrap();
    rt.block_on(async {
        let driver: Arc<dyn OpDriver> = uring_global_driver().unwrap();

        let dir = std::env::temp_dir().join(format!(
            "rusty_tokio_uring_global_driver_test_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("direct.dat");

        rusty_tokio::io::UringOpenOptions::new()
            .create(true)
            .write(true)
            .open_on(driver, &path)
            .await
            .unwrap();
        assert!(path.exists());
    });
}
