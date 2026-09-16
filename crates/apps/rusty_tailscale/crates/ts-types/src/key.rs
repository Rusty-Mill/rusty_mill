//! Typed public-key newtypes with Go's prefixed-hex JSON encoding.
//!
//! Go encodes public keys as `"<prefix><64 lowercase hex>"`, e.g.
//! `"nodekey:43b662bf…"`. Each key kind gets a distinct Rust type so a disco
//! key can never be passed where a node key is expected.

use std::fmt;
use std::str::FromStr;

use crate::hex;

/// Error parsing a prefixed public key string.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyParseError {
    #[error("key does not start with expected prefix {expected:?}")]
    WrongPrefix { expected: &'static str },
    #[error("key hex part is not exactly 64 hex characters")]
    BadHex,
}

macro_rules! public_key {
    ($(#[$doc:meta])* $name:ident, $prefix:literal) => {
        $(#[$doc])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub [u8; 32]);

        impl $name {
            /// The Go wire-encoding prefix, including the colon.
            pub const PREFIX: &'static str = $prefix;

            pub fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }
        }

        impl FromStr for $name {
            type Err = KeyParseError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let hex_part = s
                    .strip_prefix($prefix)
                    .ok_or(KeyParseError::WrongPrefix { expected: $prefix })?;
                let bytes = hex::decode32(hex_part).ok_or(KeyParseError::BadHex)?;
                Ok(Self(bytes))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}{}", $prefix, hex::encode(&self.0))
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(self, f)
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.collect_str(self)
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                struct V;
                impl serde::de::Visitor<'_> for V {
                    type Value = $name;

                    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                        write!(f, "a string like {:?}", concat!($prefix, "<64 hex>"))
                    }

                    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<$name, E> {
                        v.parse().map_err(E::custom)
                    }
                }
                d.deserialize_str(V)
            }
        }
    };
}

public_key! {
    /// A node's public WireGuard key (`nodekey:…`).
    NodePublic, "nodekey:"
}

public_key! {
    /// A node's machine identity key (`mkey:…`), used with control.
    MachinePublic, "mkey:"
}

public_key! {
    /// A node's NAT-traversal (disco) key (`discokey:…`).
    DiscoPublic, "discokey:"
}

#[cfg(test)]
mod tests {
    use super::*;

    const NK: &str = "nodekey:43b662bffd68e54a8f31f88d2ed52f445df297567de6ae08a4692f53cee68c13";

    #[test]
    fn parse_display_round_trip() {
        let k: NodePublic = NK.parse().unwrap();
        assert_eq!(k.to_string(), NK);
        assert_eq!(k.as_bytes()[0], 0x43);
    }

    #[test]
    fn kind_confusion_rejected() {
        assert_eq!(
            NK.parse::<DiscoPublic>(),
            Err(KeyParseError::WrongPrefix {
                expected: "discokey:"
            })
        );
    }

    #[test]
    fn bad_hex_rejected() {
        assert_eq!(
            "nodekey:zz".parse::<NodePublic>(),
            Err(KeyParseError::BadHex)
        );
        assert_eq!("nodekey:".parse::<NodePublic>(), Err(KeyParseError::BadHex));
    }
}
