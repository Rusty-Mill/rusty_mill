//! Private key management: machine, node, and disco identities.
//!
//! Mirrors Go `types/key`: every identity is an X25519 keypair whose private
//! half serializes as `privkey:<64 hex>` (state files only — private keys
//! never go on the wire) and whose public half uses the typed prefixes from
//! `ts_types` (`mkey:`, `nodekey:`, `discokey:`).

mod state;

use std::fmt;
use std::str::FromStr;

use rand_core::OsRng;
use x25519_dalek::{PublicKey, StaticSecret};

pub use state::{NodeState, StateError};

/// Error parsing a `privkey:<64 hex>` string.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PrivateKeyParseError {
    #[error("private key does not start with \"privkey:\"")]
    WrongPrefix,
    #[error("private key hex part is not exactly 64 hex characters")]
    BadHex,
}

const PRIVKEY_PREFIX: &str = "privkey:";

macro_rules! private_key {
    ($(#[$doc:meta])* $name:ident, $public:ident) => {
        $(#[$doc])*
        #[derive(Clone)]
        pub struct $name(StaticSecret);

        impl $name {
            /// Generates a fresh random key from the OS RNG.
            pub fn generate() -> Self {
                Self(StaticSecret::random_from_rng(OsRng))
            }

            pub fn from_bytes(bytes: [u8; 32]) -> Self {
                Self(StaticSecret::from(bytes))
            }

            pub fn to_bytes(&self) -> [u8; 32] {
                self.0.to_bytes()
            }

            /// The corresponding typed public key.
            pub fn public(&self) -> ts_types::$public {
                ts_types::$public(*PublicKey::from(&self.0).as_bytes())
            }

            /// X25519 shared secret with a raw peer public key.
            ///
            /// Takes raw bytes rather than a typed public key because
            /// handshakes mix identities (e.g. machine private × control's
            /// machine public, machine private × ephemeral).
            pub fn shared_secret(&self, peer_public: &[u8; 32]) -> [u8; 32] {
                self.0
                    .diffie_hellman(&PublicKey::from(*peer_public))
                    .to_bytes()
            }

            /// Serializes as `privkey:<64 hex>` for state files.
            pub fn to_state_string(&self) -> String {
                format!("{PRIVKEY_PREFIX}{}", hex_encode(&self.to_bytes()))
            }

            /// Wrap key bytes in a zeroize-on-drop SecretBytes container.
            pub fn to_secret_bytes(&self) -> rusty_crypto_key::SecretBytes {
                rusty_crypto_key::SecretBytes::new(self.to_bytes().to_vec())
            }
        }

        impl FromStr for $name {
            type Err = PrivateKeyParseError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let hex_part = s
                    .strip_prefix(PRIVKEY_PREFIX)
                    .ok_or(PrivateKeyParseError::WrongPrefix)?;
                let bytes = hex_decode32(hex_part).ok_or(PrivateKeyParseError::BadHex)?;
                Ok(Self::from_bytes(bytes))
            }
        }

        /// Debug never prints key material.
        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!(stringify!($name), "({})"), self.public())
            }
        }
    };
}

private_key! {
    /// The machine identity key, used for the Noise control channel.
    MachinePrivate, MachinePublic
}

private_key! {
    /// The node (WireGuard) key.
    NodePrivate, NodePublic
}

private_key! {
    /// The NAT-traversal (disco) key.
    DiscoPrivate, DiscoPublic
}

fn hex_encode(bytes: &[u8; 32]) -> String {
    const ALPHABET: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for &b in bytes {
        out.push(ALPHABET[usize::from(b >> 4)] as char);
        out.push(ALPHABET[usize::from(b & 0x0f)] as char);
    }
    out
}

fn hex_decode32(s: &str) -> Option<[u8; 32]> {
    let s = s.as_bytes();
    if s.len() != 64 {
        return None;
    }
    let nibble = |c: u8| -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    };
    let mut out = [0u8; 32];
    for (i, chunk) in s.as_chunks::<2>().0.iter().enumerate() {
        out[i] = (nibble(chunk[0])? << 4) | nibble(chunk[1])?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_string_round_trip() {
        let k = MachinePrivate::generate();
        let s = k.to_state_string();
        assert!(s.starts_with("privkey:"));
        let k2: MachinePrivate = s.parse().unwrap();
        assert_eq!(k.to_bytes(), k2.to_bytes());
        assert_eq!(k.public(), k2.public());
    }

    #[test]
    fn debug_hides_private_material() {
        let k = NodePrivate::generate();
        let dbg = format!("{k:?}");
        assert!(!dbg.contains(&hex_encode(&k.to_bytes())));
        assert!(dbg.contains("nodekey:"));
    }

    /// RFC 7748 §6.1 test vector: DH(alice_priv, bob_pub).
    #[test]
    fn x25519_rfc7748_vector() {
        let alice = MachinePrivate::from_bytes(
            hex_decode32("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a")
                .unwrap(),
        );
        let bob_pub =
            hex_decode32("de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f")
                .unwrap();
        let shared = alice.shared_secret(&bob_pub);
        assert_eq!(
            hex_encode(&shared),
            "4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742"
        );
    }

    #[test]
    fn shared_secret_agreement() {
        let a = MachinePrivate::generate();
        let b = MachinePrivate::generate();
        assert_eq!(
            a.shared_secret(&b.public().0),
            b.shared_secret(&a.public().0)
        );
    }

    #[test]
    fn parse_rejects_garbage() {
        assert_eq!(
            "nodekey:aa".parse::<NodePrivate>().err(),
            Some(PrivateKeyParseError::WrongPrefix)
        );
        assert_eq!(
            format!("privkey:{}", "g".repeat(64))
                .parse::<NodePrivate>()
                .err(),
            Some(PrivateKeyParseError::BadHex)
        );
    }
}
