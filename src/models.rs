//! Port of `src/models` — protocol-wide constants.
//!
//! The Go package also resolves the default relay hostnames at init time
//! (optionally through a hardcoded list of public DNS servers); that logic
//! lives with the caller here so library users aren't forced into DNS lookups.

/// Maximum packet size used when piping raw data through the relay.
pub const TCP_BUFFER_SIZE: usize = 1024 * 64;

pub const DEFAULT_RELAY: &str = "croc.schollz.com";
pub const DEFAULT_RELAY6: &str = "croc6.schollz.com";
pub const DEFAULT_PORT: &str = "9009";
pub const DEFAULT_PASSPHRASE: &str = "pass123";
