//! Acceptance criteria 1-2: `cargo +stable build --features
//! thread-per-core,io-uring-fs` (this file only builds under those
//! features -- see `Cargo.toml`'s `required-features`), and N =
//! `available_parallelism()` pinned threads each doing independent
//! positional file I/O. See `examples/thread_per_core_uring_smoke.rs`
//! for the `strace`-driven version of the same shape.

use rusty_tokio::io::UringFile;
use rusty_tokio::Builder;
use std::path::PathBuf;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rusty_tokio_tpc_uring_test_{tag}_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn each_core_runs_its_own_independent_positional_file_io() {
    let n = std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(2)
        .clamp(2, 8);
    let rt = Builder::new_thread_per_core()
        .worker_threads(n)
        .build()
        .unwrap();
    assert_eq!(rt.num_cores(), n);

    let dir = scratch_dir("basic");

    let handles: Vec<_> = (0..n)
        .map(|core| {
            let path = dir.join(format!("core-{core}.dat"));
            rt.spawn(async move {
                let payload = format!("payload-from-core-{core}").into_bytes();
                let file = UringFile::create(&path).await.unwrap();

                let result = file.write_at(payload.clone(), 0).await;
                assert_eq!(result.0.unwrap(), payload.len());

                file.fsync().await.unwrap();

                let buf = vec![0u8; payload.len()];
                let result = file.read_at(buf, 0).await;
                assert_eq!(result.0.unwrap(), payload.len());
                assert_eq!(result.1, payload);
            })
        })
        .collect();

    rt.block_on(async {
        for h in handles {
            h.await.unwrap();
        }
    });

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn core_handle_reaches_a_specific_core_directly() {
    let rt = Builder::new_thread_per_core()
        .worker_threads(2)
        .build()
        .unwrap();
    let handle0 = rt.core_handle(0);
    let handle1 = rt.core_handle(1);

    // Spawning explicitly onto each core (bypassing `Runtime::spawn`'s
    // round-robin) still runs -- and each core's `Handle::spawn` reaches
    // its own reactor/timer, not a shared one.
    rt.block_on(async move {
        let h0 = handle0.spawn(async { 1 + 1 });
        let h1 = handle1.spawn(async { 2 + 2 });
        assert_eq!(h0.await.unwrap(), 2);
        assert_eq!(h1.await.unwrap(), 4);
    });
}

#[rusty_tokio::test(flavor = "thread_per_core")]
async fn attribute_macro_accepts_the_thread_per_core_flavor() {
    let value = rusty_tokio::spawn(async { 41 + 1 }).await.unwrap();
    assert_eq!(value, 42);
}

#[rusty_tokio::test(flavor = "thread_per_core", worker_threads = 2)]
async fn attribute_macro_combines_flavor_and_worker_threads() {
    assert_eq!(
        rusty_tokio::Handle::current().runtime_flavor(),
        rusty_tokio::RuntimeFlavor::ThreadPerCore
    );
}
