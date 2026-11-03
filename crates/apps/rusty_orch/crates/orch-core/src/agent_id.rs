//! Provider identity: a validated token naming one configured model backend.
//!
//! Providers are configuration, not code: adding one is a config entry, so
//! the domain carries a token instead of a closed enum. The token is stored
//! inline, which keeps the type `Copy` like the ids it sits beside.

use std::fmt;

/// Longest accepted id, in bytes.
pub const MAX_LEN: usize = 32;

/// A provider id: 1 to 32 bytes of `[a-z0-9_-]`, e.g. `claude` or `local-high`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AgentId {
    len: u8,
    bytes: [u8; MAX_LEN],
}

/// Why a string is not an [`AgentId`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentIdError {
    Empty,
    TooLong(usize),
    /// The first byte outside `[a-z0-9_-]`.
    BadByte(u8),
}

impl fmt::Display for AgentIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("agent id is empty"),
            Self::TooLong(n) => write!(f, "agent id is {n} bytes; the limit is {MAX_LEN}"),
            Self::BadByte(b) => write!(
                f,
                "agent id byte {:?} is outside [a-z0-9_-]",
                char::from(*b)
            ),
        }
    }
}

impl std::error::Error for AgentIdError {}

impl AgentId {
    /// Validate `raw` as an id.
    pub const fn new(raw: &str) -> Result<Self, AgentIdError> {
        let src = raw.as_bytes();
        if src.is_empty() {
            return Err(AgentIdError::Empty);
        }
        if src.len() > MAX_LEN {
            return Err(AgentIdError::TooLong(src.len()));
        }
        let mut bytes = [0u8; MAX_LEN];
        let mut i = 0;
        while i < src.len() {
            let b = src[i];
            if !matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-') {
                return Err(AgentIdError::BadByte(b));
            }
            bytes[i] = b;
            i += 1;
        }
        // `src.len() <= MAX_LEN = 32`, so the cast is lossless.
        Ok(Self {
            len: src.len() as u8,
            bytes,
        })
    }

    /// An id from a literal, for `const` items: an invalid literal fails the
    /// build, because the check runs at compile time there.
    pub const fn from_static(raw: &'static str) -> Self {
        match Self::new(raw) {
            Ok(id) => id,
            Err(_) => panic!("invalid AgentId literal"),
        }
    }

    /// The id as a string slice.
    pub fn as_str(&self) -> &str {
        let used = &self.bytes[..usize::from(self.len)];
        // Every stored byte is ASCII, checked in `new`, so this cannot fail;
        // the fallback keeps library code free of panics.
        std::str::from_utf8(used).unwrap_or_default()
    }
}

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Debug for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AgentId({:?})", self.as_str())
    }
}

impl std::str::FromStr for AgentId {
    type Err = AgentIdError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_the_token_alphabet_and_round_trips() {
        for raw in ["claude", "local-high", "a", "x_9", &"z".repeat(MAX_LEN)] {
            let id = AgentId::new(raw).expect(raw);
            assert_eq!(id.as_str(), raw);
            assert_eq!(id.to_string(), raw);
        }
    }

    #[test]
    fn refuses_empty_long_and_foreign_bytes() {
        assert_eq!(AgentId::new(""), Err(AgentIdError::Empty));
        assert_eq!(
            AgentId::new(&"a".repeat(MAX_LEN + 1)),
            Err(AgentIdError::TooLong(MAX_LEN + 1))
        );
        assert_eq!(AgentId::new("Claude"), Err(AgentIdError::BadByte(b'C')));
        assert_eq!(AgentId::new("a b"), Err(AgentIdError::BadByte(b' ')));
        assert_eq!(AgentId::new("é"), Err(AgentIdError::BadByte(0xc3)));
    }

    #[test]
    fn equality_ignores_nothing_but_unused_capacity() {
        const A: AgentId = AgentId::from_static("codex");
        assert_eq!(A, AgentId::new("codex").expect("valid"));
        assert_ne!(A, AgentId::new("codex2").expect("valid"));
        assert_eq!(format!("{A:?}"), r#"AgentId("codex")"#);
    }
}
