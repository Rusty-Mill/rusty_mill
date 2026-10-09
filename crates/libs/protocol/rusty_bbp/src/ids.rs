//! Identifiers and scalar newtypes. All ids are task-scoped integers except
//! `TaskId`, `PrincipalId` and `OpId`, which the host supplies.

use rusty_serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! str_id {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        pub struct $name(pub String);
        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_owned())
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

macro_rules! int_id {
    ($name:ident, $prefix:literal) => {
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        pub struct $name(pub u64);
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}{}", $prefix, self.0)
            }
        }
    };
}

str_id!(TaskId);
str_id!(PrincipalId);
str_id!(OpId);

int_id!(MsgId, "msg:");
int_id!(ArtId, "art:");
int_id!(RunId, "run:");
int_id!(TurnId, "turn:");

/// Card revision, incremented on every event.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
pub struct Rev(pub u64);

/// Milliseconds since an epoch the driver chooses. The core never reads a clock.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
pub struct Time(pub u64);

impl Time {
    pub fn plus(self, ms: u64) -> Time {
        Time(self.0.saturating_add(ms))
    }
}

/// SHA-256 digest. Serialized as 64 hex characters.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Sha256(pub [u8; 32]);

impl Sha256 {
    pub fn of(bytes: &[u8]) -> Sha256 {
        Sha256(rusty_rsa::sha256(bytes))
    }
    pub fn from_hex(s: &str) -> Option<Sha256> {
        if s.len() != 64 {
            return None;
        }
        let mut out = [0u8; 32];
        for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
            let hi = (chunk[0] as char).to_digit(16)?;
            let lo = (chunk[1] as char).to_digit(16)?;
            out[i] = (hi * 16 + lo) as u8;
        }
        Some(Sha256(out))
    }
    pub fn hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
}

impl Serialize for Sha256 {
    fn serialize<S: rusty_serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.hex())
    }
}

impl<'de> Deserialize<'de> for Sha256 {
    fn deserialize<D: rusty_serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Sha256::from_hex(&s).ok_or_else(|| {
            <D::Error as rusty_serde::error::Error>::custom("expected 64 hex characters")
        })
    }
}

impl fmt::Debug for Sha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sha256:{}", &self.hex()[..12])
    }
}

/// Execution token. Derived deterministically from (task, principal, turn) so the
/// pure core can issue and check it. Opaque to agents; the adapter injects it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Token(pub Sha256);

/// Per-run runner secret, same derivation scheme.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RunSecret(pub Sha256);

/// A blob reference as the store reports it: digest and length. The core never sees bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BlobRef {
    pub sha: Sha256,
    pub len: u64,
}
