//! Standalone `rusty_stream` server binary — wires a real, production
//! `Log`/`ConsumerOffsets` (backed by `rusty_tokio`'s real io_uring
//! driver via `uring_global_driver`, not `SimDriver`) to `server::serve`
//! over a real TCP listener, with Ctrl-C triggering graceful shutdown.
//! Previously blocked entirely: nothing outside `rusty_tokio` could
//! construct a real `Arc<dyn OpDriver>` until
//! <https://github.com/baileyrd/rusty_tokio/issues/256> landed.
//!
//! Configuration is via environment variables, kept deliberately minimal
//! for this first runnable pass:
//! - `RUSTY_STREAM_ADDR` — address to bind (default `127.0.0.1:7420`)
//! - `RUSTY_STREAM_DATA_DIR` — directory for the log and consumer-offset
//!   segments (default `./data`)
//!
//! On startup, an existing log/consumer-offsets under `RUSTY_STREAM_DATA_DIR`
//! is recovered (via `Log::open`/`ConsumerOffsets::open_on`, reading the
//! manifest `crate::manifest::Manifest` persists); a fresh one is created
//! only if none exists yet. Retention is a fixed 128 MiB segment size with
//! no size/age limits by default — a real deployment should pick limits
//! based on its own disk budget, this is a starting point, not a
//! recommendation.

use std::path::PathBuf;
use std::sync::Arc;

use rusty_tokio::io::{uring_global_driver, TcpListener};
use rusty_tokio::sync::{watch, Mutex};

use rusty_stream::clock::SystemClock;
use rusty_stream::consumer::ConsumerOffsets;
use rusty_stream::retention::{Log, RetentionPolicy};
use rusty_stream::server::{self, AppState};

const DEFAULT_ADDR: &str = "127.0.0.1:7420";
const DEFAULT_DATA_DIR: &str = "./data";

fn default_policy() -> RetentionPolicy {
    RetentionPolicy {
        max_segment_bytes: 128 * 1024 * 1024,
        max_total_bytes: None,
        max_segment_age_millis: None,
    }
}

#[rusty_tokio::main(flavor = "thread_per_core")]
async fn main() -> std::io::Result<()> {
    let addr = std::env::var("RUSTY_STREAM_ADDR").unwrap_or_else(|_| DEFAULT_ADDR.to_string());
    let data_dir =
        std::env::var("RUSTY_STREAM_DATA_DIR").unwrap_or_else(|_| DEFAULT_DATA_DIR.to_string());
    let log_dir = PathBuf::from(&data_dir).join("log");
    let offsets_path = PathBuf::from(&data_dir).join("offsets.log");
    std::fs::create_dir_all(&log_dir)?;

    let driver = uring_global_driver()?;
    let clock = Arc::new(SystemClock);

    let log = match Log::open(driver.clone(), clock.clone(), &log_dir, default_policy()).await {
        Ok(log) => {
            println!(
                "rusty_stream: recovered existing log at {}",
                log_dir.display()
            );
            log
        }
        Err(_) => {
            println!(
                "rusty_stream: starting a fresh log at {}",
                log_dir.display()
            );
            Log::create(driver.clone(), clock, &log_dir, default_policy()).await?
        }
    };

    let consumer_offsets = match ConsumerOffsets::open_on(driver.clone(), &offsets_path).await {
        Ok(offsets) => {
            println!(
                "rusty_stream: recovered existing consumer offsets at {}",
                offsets_path.display()
            );
            offsets
        }
        Err(_) => {
            println!(
                "rusty_stream: starting fresh consumer offsets at {}",
                offsets_path.display()
            );
            ConsumerOffsets::create_on(driver, &offsets_path).await?
        }
    };

    let state = AppState {
        log: Arc::new(Mutex::new(log)),
        consumer_offsets: Arc::new(Mutex::new(consumer_offsets)),
    };

    let listener = TcpListener::bind_addrs(addr.as_str()).await?;
    println!("rusty_stream: listening on {addr}");

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let serve_handle = rusty_tokio::spawn(server::serve(listener, state, shutdown_rx));

    rusty_tokio::signal::ctrl_c().await?;
    println!("rusty_stream: Ctrl-C received, shutting down gracefully");
    // No receivers left to observe this is only possible if `serve` has
    // already returned on its own (e.g. a listener error) -- nothing to
    // signal in that case, so a failed send here is fine to ignore.
    let _ = shutdown_tx.send(true);

    match serve_handle.await {
        Ok(result) => result,
        // `JoinError` isn't `Sync` (it carries a `Box<dyn Any + Send>`
        // panic payload), so it can't go through `io::Error::other`
        // directly -- stringify it first, which is.
        Err(e) => Err(std::io::Error::other(e.to_string())),
    }
}
