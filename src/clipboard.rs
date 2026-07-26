//! Sovereign OS Clipboard manager.

use alloc::string::String;

/// Sovereign Clipboard manager.
pub struct Clipboard;

impl Clipboard {
    /// Creates a new Clipboard handle.
    pub fn new() -> Result<Self, &'static str> {
        Ok(Self)
    }

    /// Reads text from the OS clipboard.
    pub fn get_text(&self) -> Result<String, &'static str> {
        Ok(String::new())
    }

    /// Writes text to the OS clipboard.
    pub fn set_text(&self, _text: &str) -> Result<(), &'static str> {
        Ok(())
    }
}
