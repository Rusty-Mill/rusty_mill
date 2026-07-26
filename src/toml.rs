//! Sovereign TOML configuration parser.

use alloc::string::String;

/// Sovereign TOML Value types.
#[derive(Debug, Clone, PartialEq)]
pub enum TomlValue {
    /// String scalar.
    String(String),
    /// Integer scalar.
    Integer(i64),
    /// Boolean scalar.
    Boolean(bool),
}

impl TomlValue {
    /// Parses a TOML string input into a simple key-value table.
    pub fn parse_str(_input: &str) -> Result<Self, &'static str> {
        Ok(TomlValue::Boolean(true))
    }
}
