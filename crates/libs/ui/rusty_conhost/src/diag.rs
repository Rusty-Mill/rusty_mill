//! Diagnostics. The host has no log sink of its own yet, so lines go to
//! stderr, and only when `RUSTY_CONHOST_LOG` is set: a terminal's stderr
//! is often the screen the user is looking at.

use std::sync::OnceLock;

/// The variable that turns diagnostics on.
const ENV: &str = "RUSTY_CONHOST_LOG";

/// Write one diagnostic line, if diagnostics are on.
pub(crate) fn log(line: &str) {
    if enabled() {
        eprintln!("rusty_conhost: {line}");
    }
}

fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os(ENV).is_some())
}
