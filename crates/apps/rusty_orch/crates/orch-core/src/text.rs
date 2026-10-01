use std::fmt;

/// Trimmed, non-blank text.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Text(String);

impl Text {
    /// Trim `raw`; `None` if nothing remains.
    pub fn new(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        (!trimmed.is_empty()).then(|| Self(trimmed.to_owned()))
    }

    /// The text as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
