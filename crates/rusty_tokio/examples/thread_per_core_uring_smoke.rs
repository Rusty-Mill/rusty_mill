//! Acceptance-criteria smoke test for the thread-per-core + io_uring-fs
//! handoff: `available_parallelism()` pinned worker threads, each doing
//! independent positional file I/O (create, `write_at`, `fsync`,
//! `read_at`, verify) through real io_uring ops -- not `spawn_blocking`.
//!
//! Run under `strace` to confirm the I/O actually goes through
//! `io_uring_enter` rather than `pread64`/`pwrite64` on a blocking-pool
//! thread:
//!
//! ```sh
//! strace -f -e trace=io_uring_enter,pread64,pwrite64,openat \
//!     cargo run --quiet --features thread-per-core,io-uring-fs \
//!     --example thread_per_core_uring_smoke
//! ```

// `UringFile` is a Cargo-feature-gated re-export that `src/io/mod.rs` only
// makes available under `cfg(target_os = "linux")` -- required-features
// can't express "only on Linux", so a bare `--features io-uring-fs` build
// on another OS needs its own cfg here, or `cargo build --all-features`
// (rusty_mill's workspace CI) fails to resolve the import on Windows/macOS.
#[cfg(target_os = "linux")]
use rusty_tokio::io::UringFile;
#[cfg(target_os = "linux")]
use rusty_tokio::Builder;

#[cfg(not(target_os = "linux"))]
fn main() {}

#[cfg(target_os = "linux")]
fn main() {
    let n = std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(2)
        .clamp(2, 8);

    let rt = Builder::new_thread_per_core()
        .worker_threads(n)
        .build()
        .expect("failed to build a thread-per-core runtime");
    assert_eq!(rt.num_cores(), n);

    let dir = std::env::temp_dir().join(format!("rusty_tokio_tpc_smoke_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("failed to create scratch dir");

    let mut handles = Vec::with_capacity(n);
    for core in 0..n {
        let path = dir.join(format!("core-{core}.dat"));
        // `Runtime::spawn` round-robins across every core's own `Shared`
        // -- with exactly `n` tasks spawned onto an `n`-core runtime,
        // each core gets exactly one, so each task's `UringFile` I/O
        // genuinely originates from a different pinned OS thread.
        handles.push(rt.spawn(async move {
            let expected = format!("hello from core {core}\n").into_bytes();

            let file = UringFile::create(&path).await.expect("create failed");
            let (result, _buf) = write_at(&file, expected.clone(), 0).await;
            assert_eq!(result, expected.len());

            file.fsync().await.expect("fsync failed");

            let read_buf = vec![0u8; expected.len()];
            let (n_read, read_buf) = read_at(&file, read_buf, 0).await;
            assert_eq!(n_read, expected.len());
            assert_eq!(read_buf, expected);

            file.close().await.expect("close failed");
            core
        }));
    }

    rt.block_on(async move {
        for h in handles {
            let core = h.await.expect("spawned task panicked");
            eprintln!("core {core}: create/write_at/fsync/read_at/verify OK");
        }
    });

    std::fs::remove_dir_all(&dir).expect("failed to clean up scratch dir");
    println!("thread_per_core_uring_smoke: OK ({n} cores)");
}

#[cfg(target_os = "linux")]
async fn write_at(file: &UringFile, buf: Vec<u8>, pos: u64) -> (usize, Vec<u8>) {
    let result = file.write_at(buf, pos).await;
    (result.0.expect("write_at failed"), result.1)
}

#[cfg(target_os = "linux")]
async fn read_at(file: &UringFile, buf: Vec<u8>, pos: u64) -> (usize, Vec<u8>) {
    let result = file.read_at(buf, pos).await;
    (result.0.expect("read_at failed"), result.1)
}
