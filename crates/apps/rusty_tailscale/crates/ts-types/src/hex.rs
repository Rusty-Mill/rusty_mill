//! Minimal panic-free hex codec (kept in-tree to avoid a dependency; see
//! DESIGN.md dependency policy).

pub(crate) fn encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(ALPHABET[usize::from(b >> 4)] as char);
        out.push(ALPHABET[usize::from(b & 0x0f)] as char);
    }
    out
}

/// Decodes exactly 64 hex characters (either case) into 32 bytes.
/// Returns `None` on any other input.
pub(crate) fn decode32(s: &str) -> Option<[u8; 32]> {
    let s = s.as_bytes();
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, chunk) in s.as_chunks::<2>().0.iter().enumerate() {
        out[i] = (nibble(chunk[0])? << 4) | nibble(chunk[1])?;
    }
    Some(out)
}

fn nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let bytes: [u8; 32] = core::array::from_fn(|i| (i * 7 + 3) as u8);
        let s = encode(&bytes);
        assert_eq!(decode32(&s), Some(bytes));
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(decode32(""), None);
        assert_eq!(decode32(&"0".repeat(63)), None);
        assert_eq!(decode32(&"0".repeat(65)), None);
        assert_eq!(decode32(&"g".repeat(64)), None);
        // Multi-byte UTF-8 must not panic.
        assert_eq!(decode32(&"é".repeat(32)), None);
    }

    #[test]
    fn accepts_uppercase() {
        let s = "AB".repeat(32);
        assert_eq!(decode32(&s), Some([0xab; 32]));
    }
}
