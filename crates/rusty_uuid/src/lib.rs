//! Minimal, dependency-free UUID v4 (random) generation per RFC 4122.
//!
//! ```
//! let id = rusty_uuid::Uuid::new_v4();
//! let id_string: String = id.to_string();
//! assert_eq!(id_string.len(), 36);
//! ```

mod rand;

use std::fmt;
use std::str::FromStr;

/// A 128-bit universally unique identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Uuid([u8; 16]);

impl Uuid {
    /// Generates a random (version 4, variant 1) UUID.
    pub fn new_v4() -> Self {
        let mut bytes = [0u8; 16];
        rand::fill(&mut bytes);
        bytes[6] = (bytes[6] & 0x0f) | 0x40; // version 4
        bytes[8] = (bytes[8] & 0x3f) | 0x80; // variant 10xx (RFC 4122)
        Uuid(bytes)
    }

    /// The nil UUID, `00000000-0000-0000-0000-000000000000`.
    pub const fn nil() -> Self {
        Uuid([0; 16])
    }

    /// Builds a `Uuid` from its raw 16 bytes, unmodified.
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Uuid(bytes)
    }

    /// Returns the raw 16 bytes of the UUID.
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    /// The UUID as 32 lowercase hex digits with no hyphens, e.g.
    /// `"67e5504410b1426f9247bb680e5fe0c8"` — matches the external `uuid`
    /// crate's `Uuid::simple()` formatting, for callers that need a
    /// hyphen-free identifier (e.g. as a SQL table-name suffix).
    pub fn simple(&self) -> String {
        let mut out = String::with_capacity(32);
        for byte in self.0 {
            out.push_str(&format!("{byte:02x}"));
        }
        out
    }

    /// True if every byte is zero.
    pub fn is_nil(&self) -> bool {
        self.0 == [0; 16]
    }
}

impl Default for Uuid {
    /// Returns the nil UUID, matching the convention used by other UUID crates.
    fn default() -> Self {
        Uuid::nil()
    }
}

impl fmt::Display for Uuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let b = &self.0;
        write!(
            f,
            "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
        )
    }
}

/// Error returned when parsing a string into a [`Uuid`] fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid UUID: {}", self.0)
    }
}

impl std::error::Error for ParseError {}

impl FromStr for Uuid {
    type Err = ParseError;

    /// Parses the standard hyphenated `8-4-4-4-12` hex representation.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes_s = s.as_bytes();
        let is_well_formed = bytes_s.len() == 36
            && bytes_s[8] == b'-'
            && bytes_s[13] == b'-'
            && bytes_s[18] == b'-'
            && bytes_s[23] == b'-';
        if !is_well_formed {
            return Err(ParseError(s.to_string()));
        }

        let hex: String = s.chars().filter(|&c| c != '-').collect();
        if hex.len() != 32 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ParseError(s.to_string()));
        }

        let mut bytes = [0u8; 16];
        for (i, byte) in bytes.iter_mut().enumerate() {
            let pair = &hex[i * 2..i * 2 + 2];
            *byte = u8::from_str_radix(pair, 16).map_err(|_| ParseError(s.to_string()))?;
        }
        Ok(Uuid(bytes))
    }
}

#[cfg(feature = "rusty_serde")]
impl rusty_serde::Serialize for Uuid {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: rusty_serde::ser::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

#[cfg(feature = "rusty_serde")]
impl<'de> rusty_serde::Deserialize<'de> for Uuid {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: rusty_serde::de::Deserializer<'de>,
    {
        struct UuidVisitor;
        impl<'de> rusty_serde::de::Visitor<'de> for UuidVisitor {
            type Value = Uuid;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "a hyphenated UUID string")
            }
            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: rusty_serde::error::Error,
            {
                v.parse::<Uuid>().map_err(E::custom)
            }
            fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
            where
                E: rusty_serde::error::Error,
            {
                v.parse::<Uuid>().map_err(E::custom)
            }
        }
        deserializer.deserialize_str(UuidVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_string_has_standard_hyphenated_form() {
        let id = Uuid::new_v4().to_string();
        assert_eq!(id.len(), 36);
        assert_eq!(id.as_bytes()[8], b'-');
        assert_eq!(id.as_bytes()[13], b'-');
        assert_eq!(id.as_bytes()[18], b'-');
        assert_eq!(id.as_bytes()[23], b'-');
        assert!(id.chars().all(|c| c.is_ascii_hexdigit() || c == '-'));
    }

    #[test]
    fn new_v4_sets_version_and_variant_bits() {
        let id = Uuid::new_v4();
        let b = id.as_bytes();
        assert_eq!(b[6] & 0xf0, 0x40, "version nibble must be 4");
        assert_eq!(b[8] & 0xc0, 0x80, "variant bits must be 10xx");
    }

    #[test]
    fn new_v4_generates_distinct_ids() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        assert_ne!(a, b);
    }

    #[test]
    fn nil_is_all_zero_and_recognized() {
        let nil = Uuid::nil();
        assert!(nil.is_nil());
        assert_eq!(nil.to_string(), "00000000-0000-0000-0000-000000000000");
        assert_eq!(Uuid::default(), nil);
    }

    #[test]
    fn round_trips_through_display_and_from_str() {
        let id = Uuid::new_v4();
        let parsed: Uuid = id.to_string().parse().unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn from_str_rejects_malformed_input() {
        assert!("not-a-uuid".parse::<Uuid>().is_err());
        assert!("00000000-0000-0000-0000-00000000000"
            .parse::<Uuid>()
            .is_err()); // too short
    }

    #[test]
    fn simple_is_32_lowercase_hex_digits_no_hyphens() {
        let id = Uuid::new_v4();
        let simple = id.simple();
        assert_eq!(simple.len(), 32);
        assert!(simple.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_eq!(simple, id.to_string().chars().filter(|&c| c != '-').collect::<String>());
    }

    #[test]
    fn nil_simple_is_32_zeros() {
        assert_eq!(Uuid::nil().simple(), "0".repeat(32));
    }

    #[cfg(feature = "rusty_serde")]
    #[test]
    fn rusty_serde_json_round_trips_as_hyphenated_string() {
        let id = Uuid::new_v4();
        let json = rusty_serde::json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{id}\""));
        let parsed: Uuid = rusty_serde::json::from_str(&json).unwrap();
        assert_eq!(parsed, id);
    }

    #[cfg(feature = "rusty_serde")]
    #[test]
    fn rusty_serde_json_rejects_malformed_string() {
        let json = "\"not-a-uuid\"";
        assert!(rusty_serde::json::from_str::<Uuid>(json).is_err());
    }
}
