//! A validated single path component, as raw bytes (WTF-8 on Windows).

use std::fmt;

use crate::Errno;

/// The longest name, in bytes, the port accepts (Linux `NAME_MAX`).
pub const NAME_MAX: usize = 255;

/// One path component: non-empty, at most [`NAME_MAX`] bytes, no `/` or NUL,
/// and never `.` or `..`.
///
/// Backends may reject further names their storage cannot hold (for example
/// `\` or `:` on Windows).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Name<'a>(&'a [u8]);

impl<'a> Name<'a> {
    /// Validates `bytes` as a name.
    ///
    /// # Errors
    /// `Inval` for an empty name, `.`, `..`, or one holding `/` or NUL;
    /// `NameTooLong` past [`NAME_MAX`].
    pub fn new(bytes: &'a [u8]) -> Result<Self, Errno> {
        if bytes.len() > NAME_MAX {
            return Err(Errno::NameTooLong);
        }
        let reserved = bytes.is_empty() || bytes == b"." || bytes == b"..";
        if reserved || bytes.iter().any(|&b| b == b'/' || b == 0) {
            return Err(Errno::Inval);
        }
        Ok(Self(bytes))
    }

    /// The name's bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &'a [u8] {
        self.0
    }
}

impl fmt::Debug for Name<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Name({:?})", String::from_utf8_lossy(self.0))
    }
}

impl<'a> TryFrom<&'a str> for Name<'a> {
    type Error = Errno;

    fn try_from(value: &'a str) -> Result<Self, Self::Error> {
        Self::new(value.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_ordinary_names() {
        assert_eq!(
            Name::new(b"hello.txt").map(|n| n.as_bytes()),
            Ok(&b"hello.txt"[..])
        );
        assert!(Name::new(b"...").is_ok());
        assert!(
            Name::new(&[0xff, 0xfe]).is_ok(),
            "non-UTF-8 bytes are allowed"
        );
    }

    #[test]
    fn rejects_reserved_and_separators() {
        for bad in [&b""[..], b".", b"..", b"a/b", b"/", b"a\0b"] {
            assert_eq!(Name::new(bad), Err(Errno::Inval), "{bad:?}");
        }
    }

    #[test]
    fn enforces_length_boundary() {
        let max = vec![b'a'; NAME_MAX];
        assert!(Name::new(&max).is_ok());
        let over = vec![b'a'; NAME_MAX + 1];
        assert_eq!(Name::new(&over), Err(Errno::NameTooLong));
    }
}
