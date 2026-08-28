//! Logging setup.
//!
//! Everything goes to **stderr**, never stdout. On the stdio transport stdout
//! carries framed JSON-RPC, so a stray `println!` corrupts the stream — this is
//! also the migration the 2026-07-28 spec recommends now that the `logging`
//! feature is deprecated.

use tracing_subscriber::{EnvFilter, fmt, prelude::*};

/// Install the global subscriber, writing to stderr.
///
/// `filter` is the fallback directive; `RUST_LOG` wins when set. Calling this
/// more than once is a no-op after the first success, so a binary that also
/// sets up its own subscriber will not panic.
pub fn init(filter: &str) {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(filter));

    let _ = tracing_subscriber::registry()
        .with(fmt::layer().with_writer(std::io::stderr).with_ansi(false))
        .with(env_filter)
        .try_init();
}
