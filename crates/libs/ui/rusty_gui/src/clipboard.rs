//! Sovereign OS Clipboard manager.
//!
//! No platform backend is wired yet, so reads and writes fail with
//! [`UNSUPPORTED`] rather than reporting an empty clipboard or a write that
//! never happened.

use alloc::string::String;

/// The error every clipboard operation returns until a platform backend
/// exists.
pub const UNSUPPORTED: &str = "clipboard: not supported on this platform yet";

/// Sovereign Clipboard manager.
pub struct Clipboard;

impl Clipboard {
    /// Creates a new Clipboard handle.
    pub fn new() -> Result<Self, &'static str> {
        Ok(Self)
    }

    /// Reads text from the OS clipboard. Always [`UNSUPPORTED`] for now.
    pub fn get_text(&self) -> Result<String, &'static str> {
        Err(UNSUPPORTED)
    }

    /// Writes text to the OS clipboard. Always [`UNSUPPORTED`] for now.
    pub fn set_text(&self, _text: &str) -> Result<(), &'static str> {
        Err(UNSUPPORTED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_writes_report_unsupported_instead_of_succeeding() {
        let clipboard = Clipboard::new().expect("a handle needs no backend");
        assert_eq!(clipboard.get_text(), Err(UNSUPPORTED));
        assert_eq!(clipboard.set_text("x"), Err(UNSUPPORTED));
    }
}
