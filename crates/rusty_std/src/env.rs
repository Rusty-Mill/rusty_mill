//! Sovereign Environment variables and arguments for rusty_std.

use crate::path::PathBuf;
use alloc::string::String;
use alloc::vec::Vec;

/// Returns the command line arguments passed to the binary.
pub fn args() -> Vec<String> {
    Vec::new()
}

/// Returns the temporary directory path for the current system.
pub fn temp_dir() -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from("C:\\Temp")
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/tmp")
    }
}
