//! A zero-dependency, `#![no_std]` INI and Key-Value configuration file parser.
//!
//! Provides lightweight, safe parsing of `.ini` and `.conf` files into sections and key-value maps
//! without pulling in heavy serde dependencies.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(not(feature = "std"))]
use alloc::{
    collections::BTreeMap,
    string::String,
    string::ToString,
    vec::Vec,
};

#[cfg(feature = "std")]
use std::collections::BTreeMap;

use core::fmt;

/// An INI configuration document containing section blocks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Config {
    /// Global key-value pairs (outside any `[section]`).
    pub global: BTreeMap<String, String>,
    /// Named section blocks `[section_name] -> (key -> value)`.
    pub sections: BTreeMap<String, BTreeMap<String, String>>,
}

impl Config {
    /// Parse an INI format string.
    pub fn parse(input: &str) -> Result<Self, ParseError> {
        let mut config = Config::default();
        let mut current_section: Option<String> = None;

        for (line_num, raw_line) in input.lines().enumerate() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }

            if line.starts_with('[') && line.ends_with(']') {
                let section_name = line[1..line.len() - 1].trim();
                if section_name.is_empty() {
                    return Err(ParseError {
                        line: line_num + 1,
                        message: "empty section header",
                    });
                }
                current_section = Some(section_name.to_string());
                config
                    .sections
                    .entry(section_name.to_string())
                    .or_default();
                continue;
            }

            if let Some((key, val)) = line.split_once('=') {
                let key = key.trim().to_string();
                let val = val.trim();
                // Strip optional surrounding quotes
                let val = if (val.starts_with('"') && val.ends_with('"'))
                    || (val.starts_with('\'') && val.ends_with('\''))
                {
                    &val[1..val.len() - 1]
                } else {
                    val
                }
                .to_string();

                if let Some(ref sec) = current_section {
                    config
                        .sections
                        .get_mut(sec)
                        .unwrap()
                        .insert(key, val);
                } else {
                    config.global.insert(key, val);
                }
            } else {
                return Err(ParseError {
                    line: line_num + 1,
                    message: "expected key=value or [section]",
                });
            }
        }

        Ok(config)
    }

    /// Get a string value from a named section.
    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.sections.get(section)?.get(key).map(|s| s.as_str())
    }

    /// Get a global string value.
    pub fn get_global(&self, key: &str) -> Option<&str> {
        self.global.get(key).map(|s| s.as_str())
    }

    /// Get a boolean value (`true`/`false`, `1`/`0`, `yes`/`no`).
    pub fn get_bool(&self, section: &str, key: &str) -> Option<bool> {
        let v = self.get(section, key)?;
        match v.to_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Some(true),
            "false" | "0" | "no" | "off" => Some(false),
            _ => None,
        }
    }
}

/// INI Parse Error detailing line number and message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// 1-based line number where parse failed.
    pub line: usize,
    /// Error message.
    pub message: &'static str,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "config parse error on line {}: {}", self.line, self.message)
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sections_and_values() {
        let sample = r#"
# Server settings
port = 8080

[database]
host = "localhost"
port = 5432
ssl = true
"#;
        let cfg = Config::parse(sample).unwrap();
        assert_eq!(cfg.get_global("port"), Some("8080"));
        assert_eq!(cfg.get("database", "host"), Some("localhost"));
        assert_eq!(cfg.get("database", "port"), Some("5432"));
        assert_eq!(cfg.get_bool("database", "ssl"), Some(true));
    }
}
