//! Acceptance criterion 3: drop an in-flight `read_at`/`write_at`
//! before completion, and confirm a completion that arrives *after* the
//! drop doesn't panic, double-free, or touch freed memory. See
//! `src/io/uring_fs.rs`'s top-level docs for the invariant these tests
//! hold to the bar: a submitted buffer is only ever freed *after* a real
//! completion queue entry names its operation done, regardless of
//! whether the `Future` that submitted it is still alive at that point.
//!
//! Run under Miri where feasible and under ASAN otherwise -- Miri can't
//! emulate the real `io_uring_enter`/`io_uring_setup` syscalls this
//! module's driver thread makes, so the genuinely UB-relevant claim
//! ("a submitted buffer is never touched before the kernel says it's
//! done, never touched twice") is exercised here against the real
//! kernel under ASAN (`RUSTFLAGS="-Zsanitizer=address" cargo +nightly
//! test --features io-uring-fs -Zbuild-std --target
//! x86_64-unknown-linux-gnu --test uring_fs_cancellation`), which *can*
//! observe a real use-after-free/double-free in the driver's own
//! bookkeeping even though it can't instrument the kernel side of the
//! syscall itself.

use rusty_tokio::io::UringFile;
use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;

fn scratch_file(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rusty_tokio_uring_cancel_test_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(format!("{tag}.dat"))
}

/// Drops a `read_at` future before its completion ever has a chance to
/// arrive, then keeps the runtime alive long enough that -- if the
/// completion queue entry were ever mishandled (the buffer freed too
/// early, or the completion handler touching a dangling `Future`) --
/// something would already have gone wrong by the time this returns.
#[test]
fn dropping_an_in_flight_read_at_before_completion_is_sound() {
    let rt = rusty_tokio::Runtime::new().unwrap();
    let path = scratch_file("drop_read");

    rt.block_on(async {
        // A real file with real content, so a real `IORING_OP_READ`
        // genuinely gets submitted and would genuinely write into the
        // buffer if it ran to completion.
        let file = UringFile::create(&path).await.unwrap();
        let payload = vec![0xABu8; 64 * 1024];
        let result = file.write_at(payload, 0).await;
        result.0.unwrap();
        file.fsync().await.unwrap();

        for _ in 0..200 {
            let buf = vec![0u8; 64 * 1024];
            // Poll the future exactly once (starts the submission, the
            // driver thread wakes and may or may not have completed it
            // yet) then drop it immediately -- the race this test wants
            // is "completion lands after the drop", not "before".
            let fut = file.read_at(buf, 0);
            let mut fut = Box::pin(fut);
            let waker = futures_waker();
            let mut cx = std::task::Context::from_waker(&waker);
            let _ = fut.as_mut().poll(&mut cx);
            drop(fut);
        }

        // Give the driver thread plenty of time to actually process
        // every one of those now-orphaned completions -- under Miri/
        // ASAN this is where a use-after-free or double-free in the
        // driver's own completion handling would actually trip.
        rusty_tokio::time::sleep(Duration::from_millis(200)).await;

        // The runtime and driver are still alive and functional --
        // confirms the driver thread didn't panic or corrupt its own
        // state while processing those orphaned completions.
        let buf = vec![0u8; 64 * 1024];
        let result = file.read_at(buf, 0).await;
        let n = result.0.unwrap();
        assert_eq!(n, 64 * 1024);
        assert!(result.1.iter().all(|&b| b == 0xAB));
    });

    std::fs::remove_file(&path).ok();
}

/// Same shape, for `write_at` -- the kernel reads from (rather than
/// writes into) the buffer, but the ownership hazard is identical: the
/// pointer has to stay valid for the whole in-flight duration regardless
/// of which direction the data flows.
#[test]
fn dropping_an_in_flight_write_at_before_completion_is_sound() {
    let rt = rusty_tokio::Runtime::new().unwrap();
    let path = scratch_file("drop_write");

    rt.block_on(async {
        let file = UringFile::create(&path).await.unwrap();

        for i in 0..200u8 {
            let buf = vec![i; 64 * 1024];
            let fut = file.write_at(buf, 0);
            let mut fut = Box::pin(fut);
            let waker = futures_waker();
            let mut cx = std::task::Context::from_waker(&waker);
            let _ = fut.as_mut().poll(&mut cx);
            drop(fut);
        }

        rusty_tokio::time::sleep(Duration::from_millis(200)).await;

        // Still alive, still correct -- one final write/read pair
        // completes normally after 200 cancelled-mid-flight ops ahead
        // of it.
        let buf = vec![0x42u8; 4096];
        let result = file.write_at(buf, 0).await;
        assert_eq!(result.0.unwrap(), 4096);
        file.fsync().await.unwrap();

        let read_buf = vec![0u8; 4096];
        let result = file.read_at(read_buf, 0).await;
        assert_eq!(result.0.unwrap(), 4096);
        assert!(result.1.iter().all(|&b| b == 0x42));
    });

    std::fs::remove_file(&path).ok();
}

/// A no-op `Waker` -- these tests deliberately poll a future exactly
/// once and then drop it, so nothing here ever needs a real wake to
/// fire.
fn futures_waker() -> std::task::Waker {
    fn no_op(_: *const ()) {}
    fn clone(_: *const ()) -> std::task::RawWaker {
        raw_waker()
    }
    fn raw_waker() -> std::task::RawWaker {
        static VTABLE: std::task::RawWakerVTable =
            std::task::RawWakerVTable::new(clone, no_op, no_op, no_op);
        std::task::RawWaker::new(std::ptr::null(), &VTABLE)
    }
    // SAFETY: every vtable function is a genuine no-op over a dangling-
    // but-never-dereferenced null data pointer -- sound for any `Waker`
    // that's only ever cloned/woken/dropped, never inspected.
    unsafe { std::task::Waker::from_raw(raw_waker()) }
}
